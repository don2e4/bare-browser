//! How the URL bar shows an address: the host stays bright, everything else is dimmed.

use crate::input::Target;
use std::ops::Range;

/// Byte range of the host in `scheme://[user@]host[:port]/…`. `None` when there is no host
/// to emphasise (`file:///…`, `about:blank`), in which case the whole URL is shown uniformly.
pub fn host_range(url: &str) -> Option<Range<usize>> {
    let after_scheme = url.find("://")? + 3;
    let rest = &url[after_scheme..];
    let auth = &rest[..rest.find(['/', '?', '#']).unwrap_or(rest.len())];
    let start = auth.rfind('@').map_or(0, |i| i + 1);
    let hostport = &auth[start..];
    let len = if hostport.starts_with('[') {
        hostport.find(']').map_or(hostport.len(), |i| i + 1)
    } else {
        match hostport.rfind(':') {
            Some(i) if hostport[i + 1..].bytes().all(|b| b.is_ascii_digit()) => i,
            _ => hostport.len(),
        }
    };
    (len > 0).then_some(after_scheme + start..after_scheme + start + len)
}

/// What the URL bar shows for a page, and which part of it to emphasise.
pub struct BarView {
    pub text: String,
    /// Byte range of the host in `text`, if it is an address with one.
    pub host: Option<Range<usize>>,
}

/// A page's URL as the bar displays it: the internal home page shows an empty bar (so the
/// placeholder invites typing); a Bare Search page shows the query itself, editable, with a `?`
/// when the query would otherwise read as an address; everything else shows the URL.
pub fn bar_view(url: &str) -> BarView {
    let plain = |text: &str| BarView {
        text: text.to_string(),
        host: None,
    };
    if url == "bare://home" || url == "about:blank" {
        return plain("");
    }
    if url == "bare://search" || url.starts_with("bare://search?") {
        let q = crate::input::query_param(url, "q").unwrap_or_default();
        return match crate::input::resolve(&q) {
            Some(Target::Url(_)) => plain(&format!("?{q}")),
            _ => plain(&q),
        };
    }
    BarView {
        text: url.to_string(),
        host: host_range(url),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host(u: &str) -> Option<&str> {
        host_range(u).map(|r| &u[r])
    }

    #[test]
    fn finds_host() {
        assert_eq!(host("https://example.com/a?b#c"), Some("example.com"));
        assert_eq!(host("https://example.com"), Some("example.com"));
        assert_eq!(host("http://localhost:8080/x"), Some("localhost"));
        assert_eq!(
            host("https://user:pw@example.com:8443/"),
            Some("example.com")
        );
        assert_eq!(host("http://[::1]:80/"), Some("[::1]"));
        assert_eq!(host("https://a.b?x=http://evil.com"), Some("a.b"));
    }

    #[test]
    fn no_host_no_emphasis() {
        assert_eq!(host("file:///tmp/a.html"), None);
        assert_eq!(host("about:blank"), None);
        assert_eq!(host("not a url"), None);
    }

    #[test]
    fn home_and_blank_show_an_empty_bar() {
        assert_eq!(bar_view("bare://home").text, "");
        assert_eq!(bar_view("about:blank").text, "");
    }

    #[test]
    fn ordinary_pages_show_the_url_with_the_host_marked() {
        let v = bar_view("https://x.org/a");
        assert_eq!(v.text, "https://x.org/a");
        assert_eq!(v.host, Some(8..13));
    }

    #[test]
    fn search_pages_show_the_query() {
        assert_eq!(
            bar_view("bare://search?q=rust%20lifetimes").text,
            "rust lifetimes"
        );
        assert!(bar_view("bare://search?q=rust%20lifetimes").host.is_none());
        assert_eq!(bar_view("bare://search").text, "");
    }

    #[test]
    fn a_query_that_looks_like_an_address_gets_a_question_mark() {
        // Without it, pressing Enter in the bar would navigate to the site instead of searching again.
        assert_eq!(bar_view("bare://search?q=main.rs").text, "?main.rs");
        assert_eq!(
            bar_view("bare://search?q=see%20http%3A%2F%2Fa.b").host,
            None
        );
    }
}
