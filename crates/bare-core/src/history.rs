//! Browsing history in SQLite (`~/.local/share/bare/history.sqlite`): one row per page, with a
//! visit count and last-visit time. Suggestions rank by "frecency": visits, discounted by age.
//! Case-insensitive matching is ASCII-only (SQLite's `lower`).
//!
//! Typing in the URL bar searches history on every keystroke, on the UI thread, so it must stay
//! fast with a big history (200,000 rows took 14-64 ms). Each row stores its lower-cased search text
//! (`hay`) instead of building it per row per query, and the query takes the most recent
//! [`CANDIDATES`] matches and ranks only those. Matches are found one of two ways:
//!
//! - A trigram index (SQLite's FTS5) finds the rows containing a word of 3+ letters directly, which
//!   makes a rare word cheap: one that matches nothing no longer reads every row.
//! - Scanning newest first and stopping at [`CANDIDATES`] matches is quicker when matches are
//!   everywhere (a word in most addresses), and is all there is for 1-2 letters, or with a SQLite
//!   built without FTS5.
//!
//! Visits are recorded on the UI thread too, as pages load. The write-ahead log makes that a write
//! to the end of one file, without waiting for the disk.

use rusqlite::{Connection, params};
use std::{cell::Cell, path::Path, time::Duration};

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
    /// Searches use the trigram index: it is there and complete (FTS5 with its trigram tokenizer,
    /// SQLite 3.34 or later).
    fts: Cell<bool>,
}

/// How many of the most recent matches are ranked by frecency. Enough that a page you visit
/// regularly is always among them once the query is more than a letter or two.
const CANDIDATES: i64 = 500;

/// With more rows than this containing the long words, matches are dense and scanning newest first
/// finds [`CANDIDATES`] of them sooner than the index does.
const INDEX_MAX: i64 = 5000;

/// A history from before the index existed is indexed as it is opened if it has at most this many
/// pages (tens of milliseconds); a bigger one takes seconds (7 s for 200,000 pages), so the browser
/// has [`History::build_index`] do it on another thread.
const INDEX_NOW_MAX: i64 = 1000;

/// Higher is better: visits, halved-ish for every week since the last one.
const FRECENCY: &str = "visits * 1.0 / (1.0 + MAX(0, ?1 - last_visit) / 604800.0)";

impl History {
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        Self::init(Connection::open(path)?, INDEX_NOW_MAX)
    }

    pub fn open_in_memory() -> rusqlite::Result<Self> {
        Self::init(Connection::open_in_memory()?, i64::MAX)
    }

    fn init(conn: Connection, index_now_max: i64) -> rusqlite::Result<Self> {
        // An in-memory database has no log ("memory" is the answer); that's fine.
        conn.query_row("PRAGMA journal_mode = WAL", [], |_| Ok(()))?;
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
        let fts = match index_state(&conn) {
            Index::Ready => true,
            Index::Missing => {
                let rows: i64 = conn.query_row("SELECT COUNT(*) FROM history", [], |r| r.get(0))?;
                rows <= index_now_max && build_index_on(&conn).is_ok()
            }
            Index::Unavailable => {
                // Triggers left by a SQLite that had FTS5 would make every insert fail here.
                conn.execute_batch(DROP_TRIGGERS)?;
                false
            }
        };
        Ok(Self {
            conn,
            fts: Cell::new(fts),
        })
    }

    /// The trigram index still has to be built: [`History::build_index`], then
    /// [`History::use_index`]. Until then searches scan, as without the index.
    pub fn needs_index(&self) -> bool {
        !self.fts.get() && index_state(&self.conn) == Index::Missing
    }

    /// Build the trigram index of the history in `path`, on a connection of its own, so that it can
    /// run on another thread while the browser is in use. Searching keeps working meanwhile (by
    /// scanning); recording a visit may fail as "database is locked" until it is done.
    pub fn build_index(path: &Path) -> rusqlite::Result<()> {
        let conn = Connection::open(path)?;
        conn.busy_timeout(Duration::from_secs(5))?;
        build_index_on(&conn)
    }

    /// Start using the index that [`History::build_index`] has built.
    pub fn use_index(&self) {
        self.fts.set(index_state(&self.conn) == Index::Ready);
    }

    /// A page was visited. Keeps the old title if the new one is empty.
    pub fn record_visit(&self, url: &str, title: &str, now: i64) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT INTO history (url, title, visits, last_visit, hay)
                 VALUES (?1, ?2, 1, ?3, lower(?1 || ' ' || ?2))
             ON CONFLICT(url) DO UPDATE SET
                 visits = visits + 1,
                 last_visit = excluded.last_visit,
                 title = CASE WHEN excluded.title = '' THEN title ELSE excluded.title END,
                 hay = CASE WHEN excluded.title = '' THEN hay ELSE excluded.hay END",
            params![url, title, now],
        )?;
        Ok(())
    }

    /// The page's title arrived after the visit was recorded.
    pub fn set_title(&self, url: &str, title: &str) -> rusqlite::Result<()> {
        if !title.is_empty() {
            self.conn.execute(
                "UPDATE history SET title = ?2, hay = lower(url || ' ' || ?2) WHERE url = ?1",
                params![url, title],
            )?;
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
        let query = trigrams(tokens);
        let indexed = self.fts.get() && !query.is_empty() && self.few_in_index(&query)?;

        let mut inner = String::from("SELECT url, title, visits, last_visit FROM history WHERE 1");
        if indexed {
            inner.push_str(&format!(
                " AND rowid IN (SELECT rowid FROM history_fts WHERE history_fts MATCH ?{})",
                tokens.len() + 4
            ));
        }
        for i in 0..tokens.len() {
            inner.push_str(&format!(" AND instr(hay, ?{}) > 0", i + 4));
        }
        let sql = format!(
            "SELECT url, title, visits, last_visit FROM ({inner} ORDER BY last_visit DESC LIMIT ?3)
             ORDER BY {FRECENCY} DESC, last_visit DESC LIMIT ?2"
        );
        let mut stmt = self.conn.prepare_cached(&sql)?;
        let mut args: Vec<rusqlite::types::Value> =
            vec![now.into(), (limit as i64).into(), CANDIDATES.into()];
        args.extend(tokens.iter().map(|t| t.clone().into()));
        if indexed {
            args.push(query.into());
        }
        let rows = stmt.query_map(rusqlite::params_from_iter(args), row)?;
        rows.collect()
    }

    /// Do at most [`INDEX_MAX`] rows match the index query? Then the index is the way in. Answered
    /// from the index alone, without reading any rows.
    fn few_in_index(&self, query: &str) -> rusqlite::Result<bool> {
        let n: i64 = self
            .conn
            .prepare_cached(
                "SELECT COUNT(*) FROM
                 (SELECT rowid FROM history_fts WHERE history_fts MATCH ?2 LIMIT ?1)",
            )?
            .query_row(params![INDEX_MAX + 1, query], |r| r.get(0))?;
        Ok(n <= INDEX_MAX)
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

/// The index query for the words of 3+ letters: every three-letter piece of each, all required
/// (`"rus" "ust"`). That finds rows holding the pieces anywhere, a few more than hold the words, so
/// `instr` decides; but it needs only the index, which records where no trigram is (`detail = none`).
/// Empty if there is no such word.
fn trigrams(tokens: &[String]) -> String {
    /// A long pasted address doesn't need every piece to narrow things down.
    const MAX: usize = 24;
    let mut seen: Vec<String> = Vec::new();
    for t in tokens {
        let chars: Vec<char> = t.chars().collect();
        for w in chars.windows(3) {
            let piece: String = w.iter().collect();
            if seen.len() < MAX && !seen.contains(&piece) {
                seen.push(piece);
            }
        }
    }
    seen.iter()
        .map(|p| format!("\"{}\"", p.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(" ")
}

#[derive(Debug, PartialEq, Eq)]
enum Index {
    /// The index and the triggers that keep it in step with `history` are there.
    Ready,
    /// This SQLite can have the index, but the database doesn't (yet).
    Missing,
    /// This SQLite has no FTS5, or no trigram tokenizer.
    Unavailable,
}

const DROP_TRIGGERS: &str = "DROP TRIGGER IF EXISTS history_fts_insert;
     DROP TRIGGER IF EXISTS history_fts_delete;
     DROP TRIGGER IF EXISTS history_fts_update;";

fn index_state(conn: &Connection) -> Index {
    let usable = conn
        .execute_batch(
            "CREATE VIRTUAL TABLE temp.fts_probe USING fts5(x, tokenize = 'trigram');
             DROP TABLE temp.fts_probe;",
        )
        .is_ok();
    if !usable {
        return Index::Unavailable;
    }
    // The table and its three triggers are only ever made together, with the table filled.
    let parts: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE name IN
             ('history_fts', 'history_fts_insert', 'history_fts_delete', 'history_fts_update')",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    if parts == 4 {
        Index::Ready
    } else {
        Index::Missing
    }
}

/// (Re)build the trigram index over `hay` and the triggers that keep it in step with `history`, in
/// one transaction: nothing sees a half-built index, and a crash leaves none at all.
fn build_index_on(conn: &Connection) -> rusqlite::Result<()> {
    // `detail = none`: the index records which rows hold a trigram, not where: all `trigrams` needs,
    // and a fraction of the size.
    conn.execute_batch(&format!(
        "BEGIN IMMEDIATE;
         {DROP_TRIGGERS}
         DROP TABLE IF EXISTS history_fts;
         CREATE VIRTUAL TABLE history_fts USING fts5(
             hay, content = 'history', tokenize = 'trigram', detail = none);
         CREATE TRIGGER history_fts_insert AFTER INSERT ON history BEGIN
             INSERT INTO history_fts(rowid, hay) VALUES (new.rowid, new.hay);
         END;
         CREATE TRIGGER history_fts_delete AFTER DELETE ON history BEGIN
             INSERT INTO history_fts(history_fts, rowid, hay) VALUES ('delete', old.rowid, old.hay);
         END;
         CREATE TRIGGER history_fts_update AFTER UPDATE OF hay ON history
         WHEN old.hay IS NOT new.hay BEGIN
             INSERT INTO history_fts(history_fts, rowid, hay) VALUES ('delete', old.rowid, old.hay);
             INSERT INTO history_fts(rowid, hay) VALUES (new.rowid, new.hay);
         END;
         INSERT INTO history_fts(history_fts) VALUES ('rebuild');
         COMMIT;"
    ))
    .inspect_err(|_| {
        let _ = conn.execute_batch("ROLLBACK");
    })
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
    fn the_index_and_the_scan_find_the_same_pages() {
        let h = History::open_in_memory().unwrap();
        assert!(
            h.fts.get(),
            "this SQLite has FTS5 and its trigram tokenizer"
        );
        // More pages than INDEX_MAX share "example", so that word takes the scan even with the index.
        for i in 0..(INDEX_MAX + 500) {
            h.record_visit(
                &format!("https://site{}.example/{}/page-{i}", i % 37, i % 7),
                &format!("Article {i} on topic {}", i % 11),
                NOW - i,
            )
            .unwrap();
        }
        for q in [
            "page-12",
            "topic 7",
            "site3 page",
            "article 299 ",
            "nothing-here",
            "example",
            "example page-1",
            "e%a",
            "6/page_",
            "é",
        ] {
            let indexed = h.search(&toks(q), 20, NOW).unwrap();
            h.fts.set(false);
            let scanned = h.search(&toks(q), 20, NOW).unwrap();
            h.fts.set(true);
            assert_eq!(indexed, scanned, "{q}");
        }
        assert_eq!(h.search(&toks("page-2499"), 20, NOW).unwrap().len(), 1);
    }

    #[test]
    fn the_index_stays_in_step_with_the_table() {
        let h = History::open_in_memory().unwrap();
        for i in 0..50 {
            h.record_visit(&format!("https://x.org/{i}"), "", NOW)
                .unwrap();
            h.set_title(&format!("https://x.org/{i}"), &format!("Title {i}"))
                .unwrap();
        }
        h.record_visit("https://x.org/7", "Seven again", NOW + 1)
            .unwrap();
        h.forget("https://x.org/3").unwrap();
        h.conn
            .execute_batch(
                "INSERT INTO history_fts(history_fts, rank) VALUES ('integrity-check', 1)",
            )
            .unwrap();
        assert!(
            h.search(&toks("title 3"), 50, NOW)
                .unwrap()
                .iter()
                .all(|e| e.url != "https://x.org/3")
        );
        assert_eq!(
            h.search(&toks("seven"), 5, NOW).unwrap()[0].url,
            "https://x.org/7"
        );
    }

    #[test]
    fn a_big_old_history_is_indexed_in_the_background() {
        let dir = std::env::temp_dir().join(format!("bare-history-index-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("old.sqlite");
        {
            // A 0.1.0 database: `hay`, but no index, and too many pages to index while opening.
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE history (url TEXT PRIMARY KEY, title TEXT NOT NULL DEFAULT '', visits INTEGER NOT NULL DEFAULT 1, last_visit INTEGER NOT NULL, hay TEXT NOT NULL DEFAULT '');",
            )
            .unwrap();
            for i in 0..(INDEX_NOW_MAX + 10) {
                conn.execute(
                    "INSERT INTO history (url, title, last_visit, hay) VALUES (?1, ?2, ?3, lower(?1 || ' ' || ?2))",
                    params![format!("https://x.org/page-{i}"), format!("Page {i}"), NOW - i],
                )
                .unwrap();
            }
        }
        let h = History::open(&path).unwrap();
        assert!(
            h.needs_index() && !h.fts.get(),
            "opening doesn't wait for the index"
        );
        let before = h.search(&toks("page-1000"), 5, NOW).unwrap();
        assert_eq!(before.len(), 1, "searching works meanwhile");
        History::build_index(&path).unwrap();
        h.use_index();
        assert!(h.fts.get() && !h.needs_index());
        assert_eq!(h.search(&toks("page-1000"), 5, NOW).unwrap(), before);
        h.record_visit("https://x.org/new", "New page", NOW)
            .unwrap();
        assert_eq!(h.search(&toks("new page"), 5, NOW).unwrap().len(), 1);
        h.conn
            .execute_batch(
                "INSERT INTO history_fts(history_fts, rank) VALUES ('integrity-check', 1)",
            )
            .unwrap();
        drop(h);
        assert!(
            History::open(&path).unwrap().fts.get(),
            "and it is there from then on"
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
