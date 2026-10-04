//! The engines that ship with Bare: one TOML file each in `engines/`.

use crate::engine::{Engine, EngineDef, Fields, Kind, Ua};

const DEFINITIONS: &[(&str, &str)] = &[
    ("bing", include_str!("../engines/bing.toml")),
    ("duckduckgo", include_str!("../engines/duckduckgo.toml")),
    ("wikipedia", include_str!("../engines/wikipedia.toml")),
    ("marginalia", include_str!("../engines/marginalia.toml")),
    ("archwiki", include_str!("../engines/archwiki.toml")),
    (
        "stackexchange",
        include_str!("../engines/stackexchange.toml"),
    ),
    ("github", include_str!("../engines/github.toml")),
];

/// Panics on a malformed built-in definition; the unit tests make sure none is.
pub fn engines() -> Vec<Engine> {
    DEFINITIONS
        .iter()
        .map(|(file, text)| {
            Engine::from_toml(text).unwrap_or_else(|e| panic!("built-in engine {file}: {e}"))
        })
        .collect()
}

/// A SearXNG instance as a fallback engine: asked only when the built-in engines come up short
/// (category `fallback`), or directly with `!sx`. The instance must have `format=json` enabled.
pub fn searxng_instance(base: &str) -> Result<Engine, String> {
    let url = url::Url::parse(base.trim()).map_err(|e| format!("search.instance `{base}`: {e}"))?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(format!(
            "search.instance `{base}`: must be an http(s) address"
        ));
    }
    let origin = url.as_str().trim_end_matches('/').to_string();
    Engine::new(EngineDef {
        name: "searxng".into(),
        shortcut: "sx".into(),
        categories: vec!["fallback".into()],
        weight: 1.0,
        timeout_ms: 4000,
        kind: Kind::Json,
        ua: Ua::Api,
        url: format!("{origin}/search?q={{q}}&format=json&language={{lang}}"),
        results: "results".into(),
        fields: Fields {
            url: "{url}".into(),
            title: "{title|text}".into(),
            content: "{content|text}".into(),
        },
        fix_url: None,
        blocked_title: Vec::new(),
        blocked_body: Vec::new(),
        suggestion: Some("corrections.0".into()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_builtin_definition_loads() {
        let names: Vec<_> = engines().iter().map(|e| e.name().to_string()).collect();
        assert_eq!(
            names,
            [
                "bing",
                "duckduckgo",
                "wikipedia",
                "marginalia",
                "archwiki",
                "stackoverflow",
                "github"
            ]
        );
    }

    #[test]
    fn instance_urls_are_validated_and_normalised() {
        let e = searxng_instance(" https://searx.example/ ").unwrap();
        assert_eq!(
            e.request_url("rust lifetimes", "de"),
            "https://searx.example/search?q=rust%20lifetimes&format=json&language=de"
        );
        assert_eq!(e.def.categories, ["fallback"]);
        let sub = searxng_instance("http://localhost:8888/searx").unwrap();
        assert!(
            sub.request_url("x", "en")
                .starts_with("http://localhost:8888/searx/search?q=x")
        );
        assert!(searxng_instance("ftp://searx.example").is_err());
        assert!(searxng_instance("searx.example").is_err());
        assert!(searxng_instance("").is_err());
    }
}
