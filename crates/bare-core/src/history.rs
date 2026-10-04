//! Browsing history in SQLite (`~/.local/share/bare/history.sqlite`): one row per page, with a
//! visit count and last-visit time. Suggestions rank by "frecency": visits, discounted by age.
//! Case-insensitive matching is ASCII-only (SQLite's `lower`).
//!
//! Typing in the URL bar searches history on every keystroke, on the UI thread, so it must stay
//! fast with a big history (200,000 rows took 14-64 ms). Two things keep it cheap: each row stores
//! its lower-cased search text (`hay`) instead of building it per row per query, and the query takes
//! the most recent [`CANDIDATES`] matches (newest first, stopping early) and ranks only those.

use rusqlite::{Connection, params};
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub url: String,
    pub title: String,
    pub visits: u32,
    /// Seconds since the Unix epoch.
    pub last_visit: i64,
}

pub struct History {
    conn: Connection,
}

/// How many of the most recent matches are ranked by frecency. Enough that a page you visit
/// regularly is always among them once the query is more than a letter or two.
const CANDIDATES: i64 = 500;

/// Higher is better: visits, halved-ish for every week since the last one.
const FRECENCY: &str = "visits * 1.0 / (1.0 + MAX(0, ?1 - last_visit) / 604800.0)";

impl History {
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        Self::init(Connection::open(path)?)
    }

    pub fn open_in_memory() -> rusqlite::Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> rusqlite::Result<Self> {
        conn.execute_batch(
            "PRAGMA synchronous = NORMAL;
             CREATE TABLE IF NOT EXISTS history (
                 url        TEXT PRIMARY KEY,
                 title      TEXT NOT NULL DEFAULT '',
                 visits     INTEGER NOT NULL DEFAULT 1,
                 last_visit INTEGER NOT NULL
             );",
        )?;
        // Databases from before `hay` existed get it added and filled once.
        let has_hay: bool = conn.query_row(
            "SELECT COUNT(*) > 0 FROM pragma_table_info('history') WHERE name = 'hay'",
            [],
            |r| r.get(0),
        )?;
        if !has_hay {
            conn.execute_batch(
                "ALTER TABLE history ADD COLUMN hay TEXT NOT NULL DEFAULT '';
                 UPDATE history SET hay = lower(url || ' ' || title);",
            )?;
        }
        // Newest first, with the search text in the index itself: the substring test runs on the
        // index and only rows that match are fetched. (Without `hay` in it, a query with few matches
        // does a random table lookup per row, and is slower than a plain scan.)
        conn.execute_batch(
            "DROP INDEX IF EXISTS history_last_visit;
             CREATE INDEX IF NOT EXISTS history_recent ON history(last_visit, hay);",
        )?;
        Ok(Self { conn })
    }

    /// A page was visited. Keeps the old title if the new one is empty.
    pub fn record_visit(&self, url: &str, title: &str, now: i64) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT INTO history (url, title, visits, last_visit) VALUES (?1, ?2, 1, ?3)
             ON CONFLICT(url) DO UPDATE SET
                 visits = visits + 1,
                 last_visit = excluded.last_visit,
                 title = CASE WHEN excluded.title = '' THEN title ELSE excluded.title END",
            params![url, title, now],
        )?;
        self.refresh_hay(url)
    }

    fn refresh_hay(&self, url: &str) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE history SET hay = lower(url || ' ' || title) WHERE url = ?1",
            params![url],
        )?;
        Ok(())
    }

    /// The page's title arrived after the visit was recorded.
    pub fn set_title(&self, url: &str, title: &str) -> rusqlite::Result<()> {
        if !title.is_empty() {
            self.conn.execute(
                "UPDATE history SET title = ?2 WHERE url = ?1",
                params![url, title],
            )?;
            self.refresh_hay(url)?;
        }
        Ok(())
    }

    /// Pages whose URL or title contains every token (already lower-case), best first.
    pub fn search(
        &self,
        tokens: &[String],
        limit: usize,
        now: i64,
    ) -> rusqlite::Result<Vec<Entry>> {
        let mut inner = String::from("SELECT url, title, visits, last_visit FROM history");
        for i in 0..tokens.len() {
            inner.push_str(if i == 0 { " WHERE" } else { " AND" });
            inner.push_str(&format!(" instr(hay, ?{}) > 0", i + 4));
        }
        let sql = format!(
            "SELECT url, title, visits, last_visit FROM ({inner} ORDER BY last_visit DESC LIMIT ?3)
             ORDER BY {FRECENCY} DESC, last_visit DESC LIMIT ?2"
        );
        let mut stmt = self.conn.prepare_cached(&sql)?;
        let mut args: Vec<rusqlite::types::Value> =
            vec![now.into(), (limit as i64).into(), CANDIDATES.into()];
        args.extend(tokens.iter().map(|t| t.clone().into()));
        let rows = stmt.query_map(rusqlite::params_from_iter(args), row)?;
        rows.collect()
    }

    /// The most frecent pages overall.
    pub fn top(&self, limit: usize, now: i64) -> rusqlite::Result<Vec<Entry>> {
        self.search(&[], limit, now)
    }

    pub fn forget(&self, url: &str) -> rusqlite::Result<()> {
        self.conn
            .execute("DELETE FROM history WHERE url = ?1", params![url])?;
        Ok(())
    }

    pub fn clear(&self) -> rusqlite::Result<()> {
        self.conn.execute("DELETE FROM history", [])?;
        self.conn.execute_batch("VACUUM")
    }

    pub fn is_empty(&self) -> rusqlite::Result<bool> {
        self.len().map(|n| n == 0)
    }

    pub fn len(&self) -> rusqlite::Result<usize> {
        self.conn
            .query_row("SELECT COUNT(*) FROM history", [], |r| r.get::<_, i64>(0))
            .map(|n| n as usize)
    }
}

fn row(r: &rusqlite::Row) -> rusqlite::Result<Entry> {
    Ok(Entry {
        url: r.get(0)?,
        title: r.get(1)?,
        visits: r.get::<_, i64>(2)? as u32,
        last_visit: r.get(3)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: i64 = 86_400;
    const NOW: i64 = 1_800_000_000;

    fn toks(s: &str) -> Vec<String> {
        s.split_whitespace().map(str::to_lowercase).collect()
    }

    #[test]
    fn visits_accumulate_and_titles_are_kept() {
        let h = History::open_in_memory().unwrap();
        h.record_visit("https://a.org/", "A", NOW).unwrap();
        h.record_visit("https://a.org/", "", NOW + 5).unwrap();
        let e = &h.top(10, NOW + 5).unwrap()[0];
        assert_eq!(
            (e.visits, e.title.as_str(), e.last_visit),
            (2, "A", NOW + 5)
        );
        h.set_title("https://a.org/", "A, renamed").unwrap();
        h.set_title("https://nowhere/", "ignored").unwrap();
        assert_eq!(h.top(10, NOW).unwrap()[0].title, "A, renamed");
        assert_eq!(h.len().unwrap(), 1);
    }

    #[test]
    fn search_needs_every_token_in_url_or_title() {
        let h = History::open_in_memory().unwrap();
        h.record_visit(
            "https://doc.rust-lang.org/book/",
            "The Rust Programming Language",
            NOW,
        )
        .unwrap();
        h.record_visit("https://doc.rust-lang.org/std/", "std - Rust", NOW)
            .unwrap();
        h.record_visit("https://example.org/", "Example", NOW)
            .unwrap();
        let urls = |q: &str| {
            h.search(&toks(q), 10, NOW)
                .unwrap()
                .into_iter()
                .map(|e| e.url)
                .collect::<Vec<_>>()
        };
        assert_eq!(urls("rust").len(), 2);
        assert_eq!(
            urls("RUST book"),
            ["https://doc.rust-lang.org/book/"],
            "tokens combine across url and title, case-insensitively"
        );
        assert_eq!(urls("rust example"), Vec::<String>::new());
        assert_eq!(
            urls("100%_"),
            Vec::<String>::new(),
            "LIKE wildcards are not special"
        );
    }

    #[test]
    fn frecency_prefers_recent_and_frequent() {
        let h = History::open_in_memory().unwrap();
        let visit = |url: &str, times: u32, ago: i64| {
            for _ in 0..times {
                h.record_visit(url, "", NOW - ago).unwrap();
            }
        };
        visit("https://yesterday-thrice.org/", 3, DAY); // 3 / 1.14  = 2.6
        visit("https://today-once.org/", 1, 0); //          1 / 1     = 1.0
        visit("https://old-favourite.org/", 10, 60 * DAY); // 10 / 9.6  = 1.04 (a long-time favourite)
        visit("https://old-once.org/", 1, 60 * DAY); //     1 / 9.6   = 0.10
        let order: Vec<_> = h.top(10, NOW).unwrap().into_iter().map(|e| e.url).collect();
        assert_eq!(
            order,
            [
                "https://yesterday-thrice.org/",
                "https://old-favourite.org/",
                "https://today-once.org/",
                "https://old-once.org/"
            ]
        );
    }

    #[test]
    fn only_the_most_recent_matches_are_ranked() {
        // The documented trade-off: for a query matching more than CANDIDATES pages, a long-unvisited
        // page falls outside the window however often it was visited; a more specific query finds it.
        let h = History::open_in_memory().unwrap();
        for _ in 0..50 {
            h.record_visit(
                "https://old-favourite.example/page",
                "Old favourite",
                NOW - 400 * DAY,
            )
            .unwrap();
        }
        for i in 0..(CANDIDATES + 10) {
            h.record_visit(&format!("https://page{i}.example/page"), "Page", NOW - i)
                .unwrap();
        }
        let urls = |q: &str| {
            h.search(&toks(q), 20, NOW)
                .unwrap()
                .into_iter()
                .map(|e| e.url)
                .collect::<Vec<_>>()
        };
        assert!(!urls("page").contains(&"https://old-favourite.example/page".to_string()));
        assert_eq!(urls("favourite"), ["https://old-favourite.example/page"]);
    }

    #[test]
    fn old_databases_are_migrated_and_stay_searchable() {
        let dir = std::env::temp_dir().join(format!("bare-history-migrate-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("old.sqlite");
        {
            // The schema as it was before `hay` existed.
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE history (url TEXT PRIMARY KEY, title TEXT NOT NULL DEFAULT '', visits INTEGER NOT NULL DEFAULT 1, last_visit INTEGER NOT NULL);
                 INSERT INTO history VALUES ('https://Example.org/Docs', 'Some Docs', 3, 1800000000);",
            )
            .unwrap();
        }
        let h = History::open(&path).unwrap();
        assert_eq!(
            h.search(&toks("docs some"), 5, NOW).unwrap().len(),
            1,
            "migrated rows are searchable, case-insensitively"
        );
        h.record_visit("https://new.example/", "New", NOW).unwrap();
        assert_eq!(h.len().unwrap(), 2);
        drop(h);
        assert_eq!(
            History::open(&path).unwrap().len().unwrap(),
            2,
            "opening twice is harmless"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_title_that_arrives_later_is_searchable() {
        let h = History::open_in_memory().unwrap();
        h.record_visit("https://x.example/", "", NOW).unwrap();
        assert!(h.search(&toks("late"), 5, NOW).unwrap().is_empty());
        h.set_title("https://x.example/", "Late Title").unwrap();
        assert_eq!(h.search(&toks("late title"), 5, NOW).unwrap().len(), 1);
    }

    #[test]
    fn limit_forget_and_clear() {
        let h = History::open_in_memory().unwrap();
        for i in 0..5 {
            h.record_visit(&format!("https://x.org/{i}"), "", NOW)
                .unwrap();
        }
        assert_eq!(h.top(3, NOW).unwrap().len(), 3);
        h.forget("https://x.org/0").unwrap();
        assert_eq!(h.len().unwrap(), 4);
        h.clear().unwrap();
        assert_eq!(h.len().unwrap(), 0);
    }

    #[test]
    fn persists_in_a_file() {
        let dir = std::env::temp_dir().join(format!("bare-history-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("history.sqlite");
        History::open(&path)
            .unwrap()
            .record_visit("https://a.org/", "A", NOW)
            .unwrap();
        assert_eq!(History::open(&path).unwrap().len().unwrap(), 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
