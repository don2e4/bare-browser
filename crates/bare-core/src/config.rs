//! `~/.config/bare/config.toml`. Every key is optional; a missing or broken file means defaults.

use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Chrome {
    /// A slim URL bar is always visible.
    #[default]
    Bar,
    /// The URL bar is hidden until Ctrl+L summons it.
    Hidden,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct Config {
    pub chrome: Chrome,
    /// A 1 px hairline around the window (there is no frame to show where it ends).
    pub border: bool,
    /// The one quiet line of key hints on the home page.
    pub home_hint: bool,
    /// Background tabs idle this long are unloaded to save memory (they reload when you return).
    /// 0 turns it off.
    pub discard_minutes: u32,
    /// Remember visited pages, for URL-bar suggestions.
    pub history: bool,
    /// Block ads and trackers (a built-in baseline until the full lists have been fetched).
    pub filters: bool,
    /// Fetch the full lists (EasyList, EasyPrivacy) in the background, and again when they are a week
    /// old. Off: only `bare --update-filters` does it.
    pub filter_updates: bool,
    pub search: SearchConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(default)]
pub struct SearchConfig {
    /// A SearXNG instance (`https://…`) to ask when the built-in engines come up short. It must have
    /// the JSON format enabled, which many public instances turn off.
    pub instance: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            chrome: Chrome::Bar,
            border: false,
            home_hint: true,
            discard_minutes: 10,
            history: true,
            filters: true,
            filter_updates: true,
            search: SearchConfig::default(),
        }
    }
}

impl Config {
    pub fn parse(text: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(text)
    }

    /// Missing file is normal; an unreadable or invalid one warns on stderr and falls back.
    pub fn load(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::parse(&text).unwrap_or_else(|e| {
                eprintln!("bare: ignoring {}: {e}", path.display());
                Self::default()
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(e) => {
                eprintln!("bare: cannot read {}: {e}", path.display());
                Self::default()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_is_default() {
        assert_eq!(Config::parse("").unwrap(), Config::default());
    }

    #[test]
    fn overrides() {
        let c = Config::parse(
            "chrome = \"hidden\"\nborder = true\nhome_hint = false\ndiscard_minutes = 3\nhistory = false\nfilters = false\nfilter_updates = false\n\n[search]\ninstance = \"https://searx.example\"\n",
        )
        .unwrap();
        assert_eq!(
            c,
            Config {
                chrome: Chrome::Hidden,
                border: true,
                home_hint: false,
                discard_minutes: 3,
                history: false,
                filters: false,
                filter_updates: false,
                search: SearchConfig {
                    instance: Some("https://searx.example".into())
                },
            }
        );
    }

    #[test]
    fn partial_keeps_other_defaults() {
        let c = Config::parse("border = true").unwrap();
        assert_eq!(c.chrome, Chrome::Bar);
        assert!(c.home_hint);
    }

    #[test]
    fn defaults_for_the_new_keys() {
        let c = Config::default();
        assert_eq!((c.discard_minutes, c.history, c.filters), (10, true, true));
        assert!(c.filter_updates);
        assert_eq!(c.search.instance, None);
    }

    #[test]
    fn bad_value_is_an_error() {
        assert!(Config::parse("chrome = \"title-bar\"").is_err());
    }

    #[test]
    fn missing_file_is_default() {
        assert_eq!(
            Config::load(Path::new("/nonexistent/bare/config.toml")),
            Config::default()
        );
    }
}
