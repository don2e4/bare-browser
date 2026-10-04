//! Query syntax, after SearXNG: leading `!name` picks engines (by name, shortcut or category) and
//! `:xx` picks a language, e.g. `!w :de rust` searches German Wikipedia for "rust".

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Parsed {
    /// What to search for, with the prefixes removed.
    pub text: String,
    /// Lower-cased `!` selectors: engine names, shortcuts or categories. Empty = default engines.
    pub selectors: Vec<String>,
    pub lang: Option<String>,
}

pub fn parse(raw: &str) -> Parsed {
    let mut parsed = Parsed::default();
    let mut rest = raw.trim_start();
    loop {
        let (token, after) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
        if let Some(sel) = token.strip_prefix('!').filter(|s| is_word(s)) {
            parsed.selectors.push(sel.to_lowercase());
        } else if let Some(lang) = token.strip_prefix(':').filter(|s| is_lang(s)) {
            parsed.lang = Some(lang.to_lowercase());
        } else {
            break;
        }
        rest = after.trim_start();
    }
    parsed.text = rest.split_whitespace().collect::<Vec<_>>().join(" ");
    parsed
}

fn is_word(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// `en`, `de`, `pt-br`: two or three letters, optional region.
fn is_lang(s: &str) -> bool {
    let mut parts = s.split('-');
    let primary = parts.next().unwrap_or("");
    let ok_primary =
        (2..=3).contains(&primary.len()) && primary.chars().all(|c| c.is_ascii_alphabetic());
    ok_primary
        && parts.all(|p| (2..=4).contains(&p.len()) && p.chars().all(|c| c.is_ascii_alphanumeric()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_query() {
        let p = parse("  rust   lifetimes ");
        assert_eq!(p.text, "rust lifetimes");
        assert!(p.selectors.is_empty() && p.lang.is_none());
    }

    #[test]
    fn engine_and_language_prefixes() {
        let p = parse("!w :de rust sprache");
        assert_eq!(p.text, "rust sprache");
        assert_eq!(p.selectors, ["w"]);
        assert_eq!(p.lang.as_deref(), Some("de"));
    }

    #[test]
    fn several_selectors_and_case() {
        let p = parse("!GH !so tokio runtime");
        assert_eq!(p.selectors, ["gh", "so"]);
        assert_eq!(p.text, "tokio runtime");
    }

    #[test]
    fn prefixes_only_count_at_the_start() {
        let p = parse("what does !important mean :)");
        assert_eq!(p.text, "what does !important mean :)");
        assert!(p.selectors.is_empty() && p.lang.is_none());
    }

    #[test]
    fn prefix_only_leaves_empty_text() {
        let p = parse("!w");
        assert_eq!(p.text, "");
        assert_eq!(p.selectors, ["w"]);
    }

    #[test]
    fn regional_languages() {
        assert_eq!(parse(":pt-br olá").lang.as_deref(), Some("pt-br"));
        assert_eq!(parse(":x hello").lang, None);
        assert_eq!(parse(":x hello").text, ":x hello");
    }
}
