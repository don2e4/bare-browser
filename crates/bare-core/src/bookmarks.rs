//! Bookmarks live in `~/.config/bare/bookmarks.txt`: one `url title` per line, edit it by hand.
//! Blank lines and `# comments` are kept exactly as written when Bare adds or removes a bookmark.

use std::io;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bookmark {
    pub url: String,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Line {
    /// Blank lines and `#` comments, verbatim.
    Other(String),
    Entry(Bookmark),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Bookmarks {
    lines: Vec<Line>,
}

impl Bookmarks {
    pub fn parse(text: &str) -> Self {
        let lines = text
            .lines()
            .map(|raw| {
                let t = raw.trim();
                if t.is_empty() || t.starts_with('#') {
                    return Line::Other(raw.to_string());
                }
                let (url, title) = t.split_once(char::is_whitespace).unwrap_or((t, ""));
                Line::Entry(Bookmark {
                    url: url.to_string(),
                    title: title.trim().to_string(),
                })
            })
            .collect();
        Self { lines }
    }

    pub fn to_text(&self) -> String {
        let mut out = String::new();
        for line in &self.lines {
            match line {
                Line::Other(s) => out.push_str(s),
                Line::Entry(b) if b.title.is_empty() => out.push_str(&b.url),
                Line::Entry(b) => {
                    out.push_str(&b.url);
                    out.push(' ');
                    out.push_str(&b.title);
                }
            }
            out.push('\n');
        }
        out
    }

    pub fn items(&self) -> impl Iterator<Item = &Bookmark> {
        self.lines.iter().filter_map(|l| match l {
            Line::Entry(b) => Some(b),
            Line::Other(_) => None,
        })
    }

    pub fn len(&self) -> usize {
        self.items().count()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn contains(&self, url: &str) -> bool {
        self.items().any(|b| b.url == url)
    }

    /// Add the page if it isn't bookmarked, remove it if it is. Returns `true` when it was added.
    pub fn toggle(&mut self, url: &str, title: &str) -> bool {
        if self.contains(url) {
            self.lines
                .retain(|l| !matches!(l, Line::Entry(b) if b.url == url));
            return false;
        }
        let title = title.split_whitespace().collect::<Vec<_>>().join(" ");
        self.lines.push(Line::Entry(Bookmark {
            url: url.to_string(),
            title,
        }));
        true
    }

    /// A missing file is an empty list.
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .map(|t| Self::parse(&t))
            .unwrap_or_default()
    }

    /// Write via a temporary file and rename, so a crash can't leave half a bookmarks file.
    pub fn save(&self, path: &Path) -> io::Result<()> {
        let tmp = path.with_extension("txt.tmp");
        std::fs::write(&tmp, self.to_text())?;
        std::fs::rename(&tmp, path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "# my bookmarks\n\nhttps://doc.rust-lang.org/book/ The Rust Book\nhttps://lwn.net\n  https://example.org/a   Spaced   out \n";

    #[test]
    fn parses_urls_and_titles() {
        let b = Bookmarks::parse(SAMPLE);
        let items: Vec<_> = b.items().cloned().collect();
        assert_eq!(items.len(), 3);
        assert_eq!(
            items[0],
            Bookmark {
                url: "https://doc.rust-lang.org/book/".into(),
                title: "The Rust Book".into()
            }
        );
        assert_eq!(items[1].title, "");
        assert_eq!(
            items[2],
            Bookmark {
                url: "https://example.org/a".into(),
                title: "Spaced   out".into()
            }
        );
    }

    #[test]
    fn comments_and_blank_lines_survive_edits() {
        let mut b = Bookmarks::parse(SAMPLE);
        assert!(b.toggle("https://new.example/", "A\nnew\tpage"));
        assert!(!b.toggle("https://lwn.net", ""));
        let text = b.to_text();
        assert!(
            text.starts_with("# my bookmarks\n\nhttps://doc.rust-lang.org/book/ The Rust Book\n"),
            "{text}"
        );
        assert!(
            text.ends_with("https://new.example/ A new page\n"),
            "{text}"
        );
        assert!(!text.contains("lwn.net"));
    }

    #[test]
    fn toggle_twice_is_a_no_op() {
        let mut b = Bookmarks::default();
        assert!(b.toggle("https://a.org/", "A"));
        assert!(b.contains("https://a.org/"));
        assert!(!b.toggle("https://a.org/", "A"));
        assert!(b.is_empty());
    }

    #[test]
    fn round_trips_through_the_filesystem() {
        let dir = std::env::temp_dir().join(format!("bare-bookmarks-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("bookmarks.txt");
        assert!(Bookmarks::load(&path).is_empty(), "missing file = empty");
        let mut b = Bookmarks::parse(SAMPLE);
        b.toggle("https://x.org/", "X");
        b.save(&path).unwrap();
        assert_eq!(Bookmarks::load(&path), b);
        assert!(!path.with_extension("txt.tmp").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
