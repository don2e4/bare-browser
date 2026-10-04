//! What the URL bar's dropdown offers as you type. The caller fetches candidates (open tabs,
//! bookmarks, history hits); this decides order, shadowing and caps. Pure, so it is tested without a UI.
//!
//! With text typed, the first row is always "what Enter will do" (go to an address, or search), then
//! open tabs, bookmarks, history. With nothing typed it is a tab switcher: other tabs, bookmarks,
//! then the most frecent pages.

use crate::bookmarks::Bookmarks;
use crate::history::Entry;
use crate::input::{self, Target};

pub type TabId = u64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    /// Enter will open this address.
    Go,
    /// Enter will search for this.
    Search,
    Tab(TabId),
    Bookmark,
    History,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    pub kind: Kind,
    pub title: String,
    /// For `Go`/`Search`, what was typed; otherwise the page URL.
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TabInfo {
    pub id: TabId,
    pub title: String,
    pub url: String,
    pub current: bool,
}

const MAX_TABS: usize = 4;
const MAX_BOOKMARKS: usize = 3;

/// Lower-cased whitespace-separated words.
pub fn tokens(query: &str) -> Vec<String> {
    query.split_whitespace().map(str::to_lowercase).collect()
}

fn matches(tokens: &[String], title: &str, url: &str) -> bool {
    let hay = format!("{title} {url}").to_lowercase();
    tokens.iter().all(|t| hay.contains(t.as_str()))
}

/// `tabs` in most-recently-used order. `history` already filtered and ranked by the caller.
pub fn suggest(
    query: &str,
    tabs: &[TabInfo],
    bookmarks: &Bookmarks,
    history: &[Entry],
    limit: usize,
) -> Vec<Suggestion> {
    let toks = tokens(query);
    let mut out: Vec<Suggestion> = Vec::new();
    // URLs already offered: an open tab hides its own bookmark and history rows. The page you are on
    // is never offered back to you.
    let mut seen: Vec<String> = tabs
        .iter()
        .filter(|t| t.current)
        .map(|t| t.url.clone())
        .collect();

    match input::resolve(query) {
        Some(Target::Url(u)) => out.push(Suggestion {
            kind: Kind::Go,
            title: u,
            url: query.trim().to_string(),
        }),
        Some(Target::Search(q)) => out.push(Suggestion {
            kind: Kind::Search,
            title: q,
            url: query.trim().to_string(),
        }),
        None => {}
    }

    for t in tabs
        .iter()
        .filter(|t| !t.current && matches(&toks, &t.title, &t.url))
        .take(MAX_TABS)
    {
        seen.push(t.url.clone());
        out.push(Suggestion {
            kind: Kind::Tab(t.id),
            title: t.title.clone(),
            url: t.url.clone(),
        });
    }
    let fresh_bookmarks: Vec<_> = bookmarks
        .items()
        .filter(|b| !seen.contains(&b.url) && matches(&toks, &b.title, &b.url))
        .take(MAX_BOOKMARKS)
        .collect();
    for b in fresh_bookmarks {
        seen.push(b.url.clone());
        out.push(Suggestion {
            kind: Kind::Bookmark,
            title: b.title.clone(),
            url: b.url.clone(),
        });
    }
    for e in history {
        if out.len() >= limit {
            break;
        }
        if seen.contains(&e.url) {
            continue;
        }
        seen.push(e.url.clone());
        out.push(Suggestion {
            kind: Kind::History,
            title: e.title.clone(),
            url: e.url.clone(),
        });
    }
    out.truncate(limit);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tab(id: u64, title: &str, url: &str, current: bool) -> TabInfo {
        TabInfo {
            id,
            title: title.into(),
            url: url.into(),
            current,
        }
    }
    fn hist(url: &str, title: &str) -> Entry {
        Entry {
            url: url.into(),
            title: title.into(),
            visits: 1,
            last_visit: 0,
        }
    }
    fn kinds(s: &[Suggestion]) -> Vec<&Kind> {
        s.iter().map(|x| &x.kind).collect()
    }

    #[test]
    fn typed_row_says_what_enter_does() {
        let s = suggest("rust lifetimes", &[], &Bookmarks::default(), &[], 8);
        assert_eq!(
            s[0],
            Suggestion {
                kind: Kind::Search,
                title: "rust lifetimes".into(),
                url: "rust lifetimes".into()
            }
        );
        let s = suggest("example.com", &[], &Bookmarks::default(), &[], 8);
        assert_eq!(s[0].kind, Kind::Go);
        assert_eq!(s[0].title, "https://example.com");
        assert_eq!(
            suggest("?main.rs", &[], &Bookmarks::default(), &[], 8)[0].kind,
            Kind::Search
        );
    }

    #[test]
    fn empty_query_is_a_tab_switcher_without_the_current_tab() {
        let tabs = [
            tab(1, "Current", "https://cur.org/", true),
            tab(2, "Docs", "https://docs.org/", false),
            tab(3, "Mail", "https://mail.org/", false),
        ];
        let s = suggest(
            "",
            &tabs,
            &Bookmarks::default(),
            &[hist("https://top.org/", "Top")],
            8,
        );
        assert_eq!(kinds(&s), [&Kind::Tab(2), &Kind::Tab(3), &Kind::History]);
    }

    #[test]
    fn matching_requires_every_word_in_title_or_url() {
        let tabs = [
            tab(2, "Rust Book", "https://doc.rust-lang.org/book/", false),
            tab(
                3,
                "Rust Reference",
                "https://doc.rust-lang.org/reference/",
                false,
            ),
        ];
        let s = suggest("rust ref", &tabs, &Bookmarks::default(), &[], 8);
        assert_eq!(kinds(&s), [&Kind::Search, &Kind::Tab(3)]);
    }

    #[test]
    fn open_tab_shadows_bookmark_shadows_history() {
        let tabs = [tab(2, "Docs", "https://docs.org/", false)];
        let bm = Bookmarks::parse("https://docs.org/ Docs\nhttps://marked.org/ Marked\n");
        let history = [
            hist("https://docs.org/", "Docs"),
            hist("https://marked.org/", "Marked"),
            hist("https://only-history.org/", "Only"),
        ];
        let s = suggest("", &tabs, &bm, &history, 8);
        let urls: Vec<_> = s.iter().map(|x| x.url.as_str()).collect();
        assert_eq!(
            urls,
            [
                "https://docs.org/",
                "https://marked.org/",
                "https://only-history.org/"
            ]
        );
        assert_eq!(kinds(&s), [&Kind::Tab(2), &Kind::Bookmark, &Kind::History]);
    }

    #[test]
    fn the_current_page_is_never_suggested() {
        let tabs = [tab(1, "Here", "https://here.org/", true)];
        let bm = Bookmarks::parse("https://here.org/ Here\n");
        let history = [
            hist("https://here.org/", "Here"),
            hist("https://else.org/", "Else"),
        ];
        for query in ["", "here"] {
            let s = suggest(query, &tabs, &bm, &history, 8);
            assert!(
                s.iter()
                    .all(|x| x.url != "https://here.org/" || matches!(x.kind, Kind::Search)),
                "{query}: {s:?}"
            );
        }
        assert!(
            suggest("", &tabs, &bm, &history, 8)
                .iter()
                .any(|x| x.url == "https://else.org/")
        );
    }

    #[test]
    fn tabs_and_bookmarks_are_capped_and_the_total_is_limited() {
        let tabs: Vec<_> = (0..10)
            .map(|i| tab(i, "t", &format!("https://t.org/{i}"), false))
            .collect();
        let bm = Bookmarks::parse(
            &(0..10)
                .map(|i| format!("https://b.org/{i} b\n"))
                .collect::<String>(),
        );
        let history: Vec<_> = (0..10)
            .map(|i| hist(&format!("https://h.org/{i}"), "h"))
            .collect();
        let s = suggest("", &tabs, &bm, &history, 8);
        assert_eq!(s.len(), 8);
        assert_eq!(
            s.iter().filter(|x| matches!(x.kind, Kind::Tab(_))).count(),
            MAX_TABS
        );
        assert_eq!(
            s.iter().filter(|x| x.kind == Kind::Bookmark).count(),
            MAX_BOOKMARKS
        );
        assert_eq!(s.iter().filter(|x| x.kind == Kind::History).count(), 1);
    }

    #[test]
    fn nothing_typed_and_nothing_known_is_empty() {
        assert!(suggest("   ", &[], &Bookmarks::default(), &[], 8).is_empty());
    }
}
