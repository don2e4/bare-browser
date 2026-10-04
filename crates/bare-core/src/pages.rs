//! Internal pages (home, load errors): plain HTML plus one inline stylesheet, no scripts.

const CSS: &str = include_str!("../../../resources/page.css");
const HOME: &str = include_str!("../../../resources/home.html");
const ERROR: &str = include_str!("../../../resources/error.html");

const HINT: &str = "<p class=\"keys\"><kbd>Ctrl</kbd>+<kbd>L</kbd> search or address \u{b7} \
    <kbd>Ctrl</kbd>+<kbd>W</kbd> close \u{b7} <kbd>F11</kbd> fullscreen</p>";

pub fn home(show_hint: bool) -> String {
    fill(
        HOME,
        &[("css", CSS), ("hint", if show_hint { HINT } else { "" })],
    )
}

pub fn error(title: &str, detail: &str, uri: &str) -> String {
    fill(
        ERROR,
        &[
            ("css", CSS),
            ("title", &escape(title)),
            ("detail", &escape(detail)),
            ("uri", &escape(uri)),
        ],
    )
}

pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

/// Single-pass `{{name}}` substitution. Substituted text is never rescanned, so a value that
/// happens to contain `{{other}}` stays literal. Unknown names are left as written.
fn fill(template: &str, vars: &[(&str, &str)]) -> String {
    let mut out = String::with_capacity(template.len() + 512);
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        match after.find("}}") {
            Some(end) => {
                let name = &after[..end];
                match vars.iter().find(|(k, _)| *k == name) {
                    Some((_, v)) => out.push_str(v),
                    None => {
                        out.push_str("{{");
                        out.push_str(name);
                        out.push_str("}}");
                    }
                }
                rest = &after[end + 2..];
            }
            None => {
                out.push_str(&rest[start..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_html() {
        assert_eq!(
            escape(r#"<a href="x">&'"#),
            "&lt;a href=&quot;x&quot;&gt;&amp;&#39;"
        );
    }

    #[test]
    fn fill_is_single_pass() {
        assert_eq!(
            fill("{{a}}-{{b}}", &[("a", "{{b}}"), ("b", "B")]),
            "{{b}}-B"
        );
        assert_eq!(fill("{{nope}} {{a", &[("a", "A")]), "{{nope}} {{a");
    }

    #[test]
    fn home_has_no_leftover_placeholders_and_respects_hint() {
        let with = home(true);
        assert!(!with.contains("{{"));
        assert!(with.contains("<kbd>Ctrl</kbd>"));
        assert!(!home(false).contains("<kbd>"));
        assert!(!with.contains("<script"));
    }

    #[test]
    fn error_page_escapes_hostile_input() {
        let page = error(
            "Can't load",
            "<script>alert(1)</script>",
            "https://x/\"><img src=x onerror=1>",
        );
        assert!(!page.contains("<script>alert"));
        assert!(!page.contains("<img src=x"));
        assert!(page.contains("&lt;script&gt;"));
        assert!(!page.contains("{{"));
    }
}
