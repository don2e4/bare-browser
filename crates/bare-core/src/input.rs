//! Turning whatever was typed into the URL bar into something to load.
//!
//! One input, no modes: a URL navigates, anything else searches. Prefix `?` to force a search
//! for something that looks like a URL (`?main.rs`).

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// Load this URL as-is.
    Url(String),
    /// Run a web search for this text.
    Search(String),
}

pub fn resolve(input: &str) -> Option<Target> {
    let s = input.trim();
    if s.is_empty() {
        return None;
    }
    if let Some(rest) = s.strip_prefix('?') {
        let q = rest.trim();
        return (!q.is_empty()).then(|| Target::Search(q.to_string()));
    }
    if has_scheme_slashes(s) || s.starts_with("about:") {
        return Some(Target::Url(s.to_string()));
    }
    if s.starts_with('/') {
        return Some(Target::Url(format!("file://{}", percent_encode(s, true))));
    }
    if s.contains(char::is_whitespace) {
        return Some(Target::Search(s.to_string()));
    }
    if let Some(url) = host_like(s) {
        return Some(Target::Url(url));
    }
    Some(Target::Search(s.to_string()))
}

/// What to open for the words on the command line (or a desktop launcher's `%U`).
///
/// - nothing: nothing (the new-tab page);
/// - one argument: an address, an existing file, or a search, as typed in the URL bar;
/// - several arguments that are all addresses or files: one tab each (`bare https://a.org https://b.org`);
/// - otherwise the words together are one search (`bare rust lifetimes`).
///
/// `existing_file` maps an argument to its absolute path if it names a file that exists.
pub fn targets_from_args(
    args: &[String],
    existing_file: impl Fn(&str) -> Option<String>,
) -> Vec<Target> {
    let classify = |arg: &str| match existing_file(arg) {
        Some(path) => resolve(&path),
        None => resolve(arg),
    };
    match args {
        [] => Vec::new(),
        [one] => classify(one).into_iter().collect(),
        many => {
            let each: Vec<_> = many.iter().map(|a| classify(a)).collect();
            if each.iter().all(|t| matches!(t, Some(Target::Url(_)))) {
                each.into_iter().flatten().collect()
            } else {
                resolve(&many.join(" ")).into_iter().collect()
            }
        }
    }
}

/// The internal URL for a search; the `bare://search` scheme handler runs Bare Search.
pub fn search_url(query: &str) -> String {
    format!("bare://search?q={}", percent_encode(query, false))
}

/// The decoded `key` parameter of a URL's query string: `query_param("bare://search?q=a%20b", "q")`.
pub fn query_param(url: &str, key: &str) -> Option<String> {
    let query = url.split_once('?')?.1.split('#').next()?;
    query.split('&').find_map(|pair| {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        (percent_decode(k) == key).then(|| percent_decode(v))
    })
}

/// Decode `%XX` escapes and `+` (as space). Invalid escapes are kept literally; invalid UTF-8
/// becomes U+FFFD.
pub fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' if bytes
                .get(i + 1)
                .zip(bytes.get(i + 2))
                .is_some_and(|(a, b)| a.is_ascii_hexdigit() && b.is_ascii_hexdigit()) =>
            {
                out.push(u8::from_str_radix(&s[i + 1..i + 3], 16).unwrap_or(b'%'));
                i += 2;
            }
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `scheme://something`, scheme per RFC 3986.
fn has_scheme_slashes(s: &str) -> bool {
    let Some(i) = s.find("://") else { return false };
    let scheme = &s[..i];
    let mut chars = scheme.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
}

/// Recognise `host[:port][/path…]` and return it with a scheme. Local things (localhost, IPs,
/// `name:port`) get http because that is what dev servers speak; public names get https.
fn host_like(s: &str) -> Option<String> {
    let auth_end = s.find(['/', '?', '#']).unwrap_or(s.len());
    let authority = &s[..auth_end];
    if authority.is_empty() || authority.contains('@') {
        return None; // email-ish or userinfo: don't guess
    }

    let (host, port) = if let Some(inner) = authority.strip_prefix('[') {
        let close = inner.find(']')?;
        let tail = &inner[close + 1..];
        let port = match tail {
            "" => None,
            t => Some(t.strip_prefix(':')?),
        };
        (&inner[..close], port)
    } else {
        match authority.rsplit_once(':') {
            Some((h, p)) => (h, Some(p)),
            None => (authority, None),
        }
    };
    if port.is_some_and(|p| p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit())) {
        return None;
    }

    let scheme = if authority.starts_with('[') {
        host.contains(':').then_some("http")?
    } else if host.eq_ignore_ascii_case("localhost")
        || host.to_ascii_lowercase().ends_with(".localhost")
        || is_ipv4(host)
        || (!host.contains('.') && port.is_some() && is_label(host))
    {
        "http"
    } else if is_public_hostname(host) {
        "https"
    } else {
        return None;
    };
    Some(format!("{scheme}://{s}"))
}

fn is_ipv4(h: &str) -> bool {
    let parts: Vec<_> = h.split('.').collect();
    parts.len() == 4
        && parts.iter().all(|p| {
            !p.is_empty()
                && p.len() <= 3
                && p.bytes().all(|b| b.is_ascii_digit())
                && p.parse::<u16>().is_ok_and(|n| n <= 255)
        })
}

fn is_label(l: &str) -> bool {
    !l.is_empty()
        && l.len() <= 63
        && !l.starts_with('-')
        && !l.ends_with('-')
        && l.chars().all(|c| c.is_alphanumeric() || c == '-')
}

/// Has a dot, every label valid, and an alphabetic TLD (so `3.14` and `e.g.` stay searches).
fn is_public_hostname(h: &str) -> bool {
    let labels: Vec<_> = h.split('.').collect();
    if labels.len() < 2 || !labels.iter().all(|l| is_label(l)) {
        return false;
    }
    let tld = labels[labels.len() - 1];
    tld.starts_with("xn--") || (tld.chars().count() >= 2 && tld.chars().all(char::is_alphabetic))
}

/// Percent-encode everything except RFC 3986 unreserved characters (and `/` for paths).
pub fn percent_encode(s: &str, keep_slash: bool) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric()
            || matches!(b, b'-' | b'.' | b'_' | b'~')
            || (keep_slash && b == b'/')
        {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use Target::*;

    fn url(s: &str) -> Option<Target> {
        Some(Url(s.into()))
    }
    fn search(s: &str) -> Option<Target> {
        Some(Search(s.into()))
    }

    #[test]
    fn empty_is_nothing() {
        assert_eq!(resolve(""), None);
        assert_eq!(resolve("   "), None);
        assert_eq!(resolve("?"), None);
    }

    #[test]
    fn explicit_schemes_pass_through() {
        assert_eq!(
            resolve("https://example.com/a b"),
            url("https://example.com/a b")
        );
        assert_eq!(
            resolve("http://localhost:3000"),
            url("http://localhost:3000")
        );
        assert_eq!(resolve("file:///tmp/x.html"), url("file:///tmp/x.html"));
        assert_eq!(resolve("bare://home"), url("bare://home"));
        assert_eq!(resolve("about:blank"), url("about:blank"));
    }

    #[test]
    fn public_hosts_get_https() {
        assert_eq!(resolve("example.com"), url("https://example.com"));
        assert_eq!(
            resolve("  en.wikipedia.org/wiki/Rust  "),
            url("https://en.wikipedia.org/wiki/Rust")
        );
        assert_eq!(
            resolve("example.com:8443/x?y=1#z"),
            url("https://example.com:8443/x?y=1#z")
        );
        assert_eq!(resolve("bücher.de"), url("https://bücher.de"));
        assert_eq!(resolve("foo.xn--p1ai"), url("https://foo.xn--p1ai"));
    }

    #[test]
    fn local_hosts_get_http() {
        assert_eq!(resolve("localhost"), url("http://localhost"));
        assert_eq!(
            resolve("localhost:8080/app"),
            url("http://localhost:8080/app")
        );
        assert_eq!(resolve("192.168.1.1"), url("http://192.168.1.1"));
        assert_eq!(resolve("127.0.0.1:5000"), url("http://127.0.0.1:5000"));
        assert_eq!(resolve("[::1]:8080"), url("http://[::1]:8080"));
        assert_eq!(resolve("nas:9000"), url("http://nas:9000"));
    }

    #[test]
    fn absolute_paths_become_file_urls() {
        assert_eq!(
            resolve("/home/me/a b#c.html"),
            url("file:///home/me/a%20b%23c.html")
        );
    }

    #[test]
    fn everything_else_searches() {
        assert_eq!(resolve("rust"), search("rust"));
        assert_eq!(resolve("rust lifetimes"), search("rust lifetimes"));
        assert_eq!(resolve("what is rust?"), search("what is rust?"));
        assert_eq!(resolve("3.14"), search("3.14"));
        assert_eq!(resolve("e.g."), search("e.g."));
        assert_eq!(resolve("me@example.com"), search("me@example.com"));
        assert_eq!(resolve("999.1.1.1"), search("999.1.1.1"));
        assert_eq!(
            resolve("data:text/html,<h1>hi</h1>"),
            search("data:text/html,<h1>hi</h1>")
        );
        assert_eq!(
            resolve("javascript:alert(1)"),
            search("javascript:alert(1)")
        );
    }

    #[test]
    fn question_mark_forces_search() {
        assert_eq!(resolve("?main.rs"), search("main.rs"));
        assert_eq!(resolve("? example.com"), search("example.com"));
    }

    #[test]
    fn command_line_arguments() {
        let none = |_: &str| None;
        let args = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(targets_from_args(&[], none), vec![]);
        assert_eq!(
            targets_from_args(&args(&["example.com"]), none),
            vec![Url("https://example.com".into())]
        );
        assert_eq!(
            targets_from_args(&args(&["rust"]), none),
            vec![Search("rust".into())]
        );
        // Several addresses: one tab each, as a desktop launcher passing %U expects.
        assert_eq!(
            targets_from_args(&args(&["https://a.org/x", "b.org", "localhost:8080"]), none),
            vec![
                Url("https://a.org/x".into()),
                Url("https://b.org".into()),
                Url("http://localhost:8080".into())
            ]
        );
        // Words are one search, even if one of them looks like an address.
        assert_eq!(
            targets_from_args(&args(&["rust", "lifetimes"]), none),
            vec![Search("rust lifetimes".into())]
        );
        assert_eq!(
            targets_from_args(&args(&["rust", "example.com"]), none),
            vec![Search("rust example.com".into())]
        );
        // An existing file wins over how its name would read.
        let file = |a: &str| (a == "main.rs").then(|| "/home/u/main.rs".to_string());
        assert_eq!(
            targets_from_args(&args(&["main.rs"]), file),
            vec![Url("file:///home/u/main.rs".into())]
        );
        assert_eq!(
            targets_from_args(&args(&["main.rs", "https://a.org"]), file),
            vec![
                Url("file:///home/u/main.rs".into()),
                Url("https://a.org".into())
            ]
        );
    }

    #[test]
    fn search_url_encodes() {
        assert_eq!(
            search_url("rust & c++"),
            "bare://search?q=rust%20%26%20c%2B%2B"
        );
    }

    #[test]
    fn search_url_round_trips_through_query_param() {
        for q in [
            "rust lifetimes",
            "c++ & rust = ?",
            "日本語 ünï",
            "100% sure #1",
            "a+b",
            "",
        ] {
            assert_eq!(query_param(&search_url(q), "q").as_deref(), Some(q), "{q}");
        }
    }

    #[test]
    fn query_param_edge_cases() {
        assert_eq!(query_param("bare://search", "q"), None);
        assert_eq!(
            query_param("bare://search?x=1&q=a+b#frag", "q").as_deref(),
            Some("a b")
        );
        assert_eq!(query_param("bare://search?q", "q").as_deref(), Some(""));
        assert_eq!(
            query_param("bare://search?q=%zz%4", "q").as_deref(),
            Some("%zz%4"),
            "bad escapes stay literal"
        );
        assert_eq!(percent_decode("%E6%97%A5"), "日");
        assert_eq!(percent_decode("%FF"), "\u{FFFD}");
    }
}
