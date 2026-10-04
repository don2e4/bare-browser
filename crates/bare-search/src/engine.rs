//! Engine definitions (TOML) and the pure half of an engine: build the request URL, parse the
//! response. No network here, so every engine is testable against a recorded response.

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use scraper::{ElementRef, Html, Selector};
use serde::Deserialize;
use serde_json::Value;
use std::time::Duration;
use url::Url;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Html,
    Json,
}

/// Which User-Agent to present: scrapers look like a browser, documented APIs identify as Bare.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Ua {
    #[default]
    Browser,
    Api,
}

/// Result links that are tracking redirects to unwrap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FixUrl {
    BingCk,
    DdgUddg,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Fields {
    pub url: String,
    pub title: String,
    #[serde(default)]
    pub content: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct EngineDef {
    pub name: String,
    pub shortcut: String,
    #[serde(default = "general")]
    pub categories: Vec<String>,
    #[serde(default = "one")]
    pub weight: f64,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
    pub kind: Kind,
    #[serde(default)]
    pub ua: Ua,
    /// Request URL template: `{q}` (percent-encoded query) and `{lang}`.
    pub url: String,
    /// HTML: CSS selector for one result. JSON: dotted path to the results array.
    pub results: String,
    pub fields: Fields,
    #[serde(default)]
    pub fix_url: Option<FixUrl>,
    /// HTML: if the page <title> contains one of these (lower-case), the engine is blocking us.
    #[serde(default)]
    pub blocked_title: Vec<String>,
    /// HTML: if a page with no results contains one of these (lower-case), it is a bot challenge, not
    /// "no results". DuckDuckGo serves its challenge with HTTP 202 and no <title>.
    #[serde(default)]
    pub blocked_body: Vec<String>,
    /// JSON: dotted path to a "did you mean" string.
    #[serde(default)]
    pub suggestion: Option<String>,
}

fn general() -> Vec<String> {
    vec!["general".into()]
}
fn one() -> f64 {
    1.0
}
fn default_timeout_ms() -> u64 {
    3000
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawResult {
    pub url: String,
    pub title: String,
    pub content: String,
}

#[derive(Debug, Default)]
pub struct Parsed {
    pub results: Vec<RawResult>,
    pub suggestions: Vec<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ParseError {
    /// The engine is refusing us (captcha, rate limit): back off for a long while.
    Blocked(String),
    /// The response wasn't what the definition expects: the engine may have changed.
    Malformed(String),
}

#[derive(Clone)]
struct Field {
    sel: Selector,
    attr: Option<String>,
}

#[derive(Clone)]
struct HtmlSpec {
    results: Selector,
    url: Field,
    title: Field,
    content: Option<Field>,
}

#[derive(Clone)]
pub struct Engine {
    pub def: EngineDef,
    html: Option<HtmlSpec>,
}

impl Engine {
    pub fn new(def: EngineDef) -> Result<Self, String> {
        let html = match def.kind {
            Kind::Html => Some(HtmlSpec {
                results: sel(&def.results).map_err(|e| format!("{}: results: {e}", def.name))?,
                url: field(&def.fields.url)
                    .map_err(|e| format!("{}: fields.url: {e}", def.name))?,
                title: field(&def.fields.title)
                    .map_err(|e| format!("{}: fields.title: {e}", def.name))?,
                content: if def.fields.content.is_empty() {
                    None
                } else {
                    Some(
                        field(&def.fields.content)
                            .map_err(|e| format!("{}: fields.content: {e}", def.name))?,
                    )
                },
            }),
            Kind::Json => None,
        };
        Ok(Self { def, html })
    }

    pub fn from_toml(text: &str) -> Result<Self, String> {
        Self::new(toml::from_str(text).map_err(|e| e.to_string())?)
    }

    pub fn name(&self) -> &str {
        &self.def.name
    }

    pub fn timeout(&self) -> Duration {
        Duration::from_millis(self.def.timeout_ms)
    }

    /// `{q}` percent-encoded query, `{lang}` e.g. `en`, `{mkt}` market e.g. `en-US`, `{cc}` country e.g. `us`.
    pub fn request_url(&self, query: &str, lang: &str) -> String {
        let (mkt, cc) = market(lang);
        self.def
            .url
            .replace("{q}", &bare_core::input::percent_encode(query, false))
            .replace("{lang}", lang)
            .replace("{mkt}", &mkt)
            .replace("{cc}", &cc)
    }

    pub fn parse(&self, body: &str, request_url: &str, lang: &str) -> Result<Parsed, ParseError> {
        match &self.html {
            Some(spec) => self.parse_html(spec, body, request_url),
            None => self.parse_json(body, lang),
        }
    }

    fn parse_html(
        &self,
        spec: &HtmlSpec,
        body: &str,
        request_url: &str,
    ) -> Result<Parsed, ParseError> {
        let doc = Html::parse_document(body);
        if let Ok(title_sel) = Selector::parse("title")
            && let Some(title) = doc.select(&title_sel).next()
        {
            let title = text_of(title).to_lowercase();
            if let Some(marker) = self
                .def
                .blocked_title
                .iter()
                .chain(&BLOCK_MARKERS.map(String::from))
                .find(|m| title.contains(m.as_str()))
            {
                return Err(ParseError::Blocked(format!(
                    "page title mentions \"{marker}\""
                )));
            }
        }
        let base = Url::parse(request_url).map_err(|e| ParseError::Malformed(e.to_string()))?;
        let mut out = Parsed::default();
        for el in doc.select(&spec.results) {
            let href = extract(el, &spec.url);
            let title = extract(el, &spec.title);
            let content = spec
                .content
                .as_ref()
                .map(|f| extract(el, f))
                .unwrap_or_default();
            if let Some(url) = self.finish_url(&href, Some(&base))
                && !title.is_empty()
            {
                out.results.push(RawResult {
                    url,
                    title,
                    content,
                });
            }
        }
        if out.results.is_empty() {
            let lower = body.to_lowercase();
            if let Some(marker) = self
                .def
                .blocked_body
                .iter()
                .find(|m| lower.contains(m.as_str()))
            {
                return Err(ParseError::Blocked(format!(
                    "bot challenge page (\"{marker}\")"
                )));
            }
        }
        Ok(out)
    }

    fn parse_json(&self, body: &str, lang: &str) -> Result<Parsed, ParseError> {
        let root: Value = serde_json::from_str(body)
            .map_err(|e| ParseError::Malformed(format!("not JSON: {e}")))?;
        let Some(Value::Array(items)) = lookup(&root, &self.def.results) else {
            let msg = root
                .get("message")
                .or_else(|| root.get("error_message"))
                .and_then(Value::as_str)
                .unwrap_or("");
            if msg.to_lowercase().contains("rate limit")
                || root.get("error_id").and_then(Value::as_i64) == Some(502)
            {
                return Err(ParseError::Blocked(msg.to_string()));
            }
            return Err(ParseError::Malformed(format!(
                "no array at `{}`",
                self.def.results
            )));
        };
        let mut out = Parsed::default();
        for item in items {
            let url = render(&self.def.fields.url, item, lang);
            let title = clean(&render(&self.def.fields.title, item, lang));
            let content = clean(&render(&self.def.fields.content, item, lang));
            if let Some(url) = self.finish_url(&url, None)
                && !title.is_empty()
            {
                out.results.push(RawResult {
                    url,
                    title,
                    content,
                });
            }
        }
        if let Some(path) = &self.def.suggestion
            && let Some(Value::String(s)) = lookup(&root, path)
            && !s.is_empty()
        {
            out.suggestions.push(s.clone());
        }
        Ok(out)
    }

    /// Resolve against the request URL, unwrap tracking redirects, require http(s).
    fn finish_url(&self, href: &str, base: Option<&Url>) -> Option<String> {
        if href.is_empty() {
            return None;
        }
        let url = match base {
            Some(b) => b.join(href).ok()?,
            None => Url::parse(href).ok()?,
        };
        let url = match self.def.fix_url {
            Some(FixUrl::BingCk) => unwrap_bing(&url)?,
            Some(FixUrl::DdgUddg) => unwrap_ddg(&url)?,
            None => url,
        };
        matches!(url.scheme(), "http" | "https").then(|| url.to_string())
    }
}

/// (market, country) for a language: `en` -> (`en-US`, `us`). Some engines (Bing) guess a market from
/// the IP address when none is given, and then answer a different question entirely.
pub fn market(lang: &str) -> (String, String) {
    let lang = lang.to_lowercase();
    if let Some((l, r)) = lang.split_once('-') {
        return (format!("{l}-{}", r.to_uppercase()), r.to_lowercase());
    }
    let region = match lang.as_str() {
        "de" => "DE",
        "fr" => "FR",
        "es" => "ES",
        "it" => "IT",
        "pt" => "BR",
        "nl" => "NL",
        "pl" => "PL",
        "ru" => "RU",
        "ja" => "JP",
        "zh" => "CN",
        "ko" => "KR",
        "sv" => "SE",
        "tr" => "TR",
        "cs" => "CZ",
        "da" => "DK",
        "fi" => "FI",
        "nb" | "no" => "NO",
        "hu" => "HU",
        _ => "US",
    };
    (format!("{lang}-{region}"), region.to_lowercase())
}

/// Titles that mean "you are a robot", for every HTML engine.
const BLOCK_MARKERS: [&str; 3] = ["captcha", "access denied", "too many requests"];

fn unwrap_bing(url: &Url) -> Option<Url> {
    if url.host_str() != Some("www.bing.com") || url.path() != "/ck/a" {
        return Some(url.clone());
    }
    let (_, u) = url.query_pairs().find(|(k, _)| k == "u")?;
    let encoded = u.strip_prefix("a1")?;
    let bytes = URL_SAFE_NO_PAD.decode(encoded.trim_end_matches('=')).ok()?;
    Url::parse(std::str::from_utf8(&bytes).ok()?).ok()
}

fn unwrap_ddg(url: &Url) -> Option<Url> {
    if url.host_str() != Some("duckduckgo.com") || url.path() != "/l/" {
        return Some(url.clone());
    }
    let (_, target) = url.query_pairs().find(|(k, _)| k == "uddg")?;
    Url::parse(&target).ok()
}

// ---- HTML helpers ---------------------------------------------------------------------------

fn sel(s: &str) -> Result<Selector, String> {
    Selector::parse(s).map_err(|e| format!("bad selector `{s}`: {e}"))
}

/// `css selector` or `css selector@attribute`.
fn field(spec: &str) -> Result<Field, String> {
    match spec.rsplit_once('@') {
        Some((css, attr)) => Ok(Field {
            sel: sel(css.trim())?,
            attr: Some(attr.trim().to_string()),
        }),
        None => Ok(Field {
            sel: sel(spec)?,
            attr: None,
        }),
    }
}

fn extract(el: ElementRef, f: &Field) -> String {
    let Some(found) = el.select(&f.sel).next() else {
        return String::new();
    };
    match &f.attr {
        Some(a) => found.value().attr(a).unwrap_or("").trim().to_string(),
        None => text_of(found),
    }
}

fn text_of(el: ElementRef) -> String {
    clean(&el.text().collect::<String>())
}

/// Collapse whitespace; drop dangling separators left by empty template fields.
fn clean(s: &str) -> String {
    let joined = s.split_whitespace().collect::<Vec<_>>().join(" ");
    joined
        .trim_matches(|c: char| c == '·' || c.is_whitespace())
        .to_string()
}

// ---- JSON helpers -----------------------------------------------------------------------------

fn lookup<'a>(v: &'a Value, path: &str) -> Option<&'a Value> {
    let mut cur = v;
    for part in path.split('.').filter(|p| !p.is_empty()) {
        cur = match cur {
            Value::Array(a) => a.get(part.parse::<usize>().ok()?)?,
            _ => cur.get(part)?,
        };
    }
    Some(cur)
}

fn value_text(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        Value::Array(a) => a.iter().map(value_text).collect::<Vec<_>>().join(", "),
        other => other.to_string(),
    }
}

/// `{path|filter|filter}` substitution over one JSON item; `{lang}` is the search language.
/// Filters: `text` (HTML to plain text), `wiki` (title to wiki path), `enc` (percent-encode).
fn render(template: &str, item: &Value, lang: &str) -> String {
    let mut out = String::new();
    let mut rest = template;
    while let Some(start) = rest.find('{') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let Some(end) = after.find('}') else {
            out.push_str(&rest[start..]);
            return out;
        };
        let mut parts = after[..end].split('|');
        let path = parts.next().unwrap_or("");
        let mut value = if path == "lang" {
            lang.to_string()
        } else {
            lookup(item, path).map(value_text).unwrap_or_default()
        };
        for filter in parts {
            value = match filter {
                "text" => html_to_text(&value),
                "wiki" => wiki_path(&value),
                "enc" => bare_core::input::percent_encode(&value, false),
                _ => value,
            };
        }
        out.push_str(&value);
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}

pub fn html_to_text(s: &str) -> String {
    let frag = Html::parse_fragment(s);
    clean(&frag.root_element().text().collect::<String>())
}

/// "Rust (programming language)" -> "Rust_(programming_language)", percent-encoded like Wikipedia does.
fn wiki_path(title: &str) -> String {
    let mut out = String::new();
    for b in title.replace(' ', "_").bytes() {
        if b.is_ascii_alphanumeric() || b"-._~()',:!*;@$/".contains(&b) {
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

    #[test]
    fn template_rendering() {
        let item: Value = serde_json::json!({"title": "Rust (programming language)", "n": 5, "tags": ["a", "b"], "d": null, "h": "x &amp; <b>y</b>"});
        assert_eq!(
            render("https://{lang}.example/{title|wiki}", &item, "de"),
            "https://de.example/Rust_(programming_language)"
        );
        assert_eq!(render("{n} votes · {tags}", &item, "en"), "5 votes · a, b");
        assert_eq!(render("{h|text}", &item, "en"), "x & y");
        assert_eq!(render("{missing}|{d}|", &item, "en"), "||");
    }

    #[test]
    fn clean_drops_dangling_separators() {
        assert_eq!(clean("★ 12 ·  "), "★ 12");
        assert_eq!(clean(" · hello   world "), "hello world");
    }

    #[test]
    fn bing_redirects_are_unwrapped() {
        let u = Url::parse(
            "https://www.bing.com/ck/a?!&&p=abc&u=a1aHR0cHM6Ly9ydXN0LmZhY2VwdW5jaC5jb20v&ntb=1",
        )
        .unwrap();
        assert_eq!(
            unwrap_bing(&u).unwrap().as_str(),
            "https://rust.facepunch.com/"
        );
        let direct = Url::parse("https://example.org/x").unwrap();
        assert_eq!(unwrap_bing(&direct).unwrap(), direct);
        let bad = Url::parse("https://www.bing.com/ck/a?u=zzz").unwrap();
        assert!(unwrap_bing(&bad).is_none());
    }

    #[test]
    fn ddg_redirects_are_unwrapped() {
        let u = Url::parse(
            "https://duckduckgo.com/l/?uddg=https%3A%2F%2Fdoc.rust%2Dlang.org%2Fx&rut=1",
        )
        .unwrap();
        assert_eq!(
            unwrap_ddg(&u).unwrap().as_str(),
            "https://doc.rust-lang.org/x"
        );
    }

    #[test]
    fn bing_always_gets_a_market() {
        let e = Engine::from_toml(include_str!("../engines/bing.toml")).unwrap();
        assert_eq!(
            e.request_url("rust lifetimes", "en"),
            "https://www.bing.com/search?q=rust%20lifetimes&setlang=en&cc=us&mkt=en-US"
        );
        assert_eq!(market("pt-br"), ("pt-BR".to_string(), "br".to_string()));
        assert_eq!(market("xx"), ("xx-US".to_string(), "us".to_string()));
    }

    #[test]
    fn request_urls_encode_the_query() {
        let e = Engine::from_toml(include_str!("../engines/wikipedia.toml")).unwrap();
        assert_eq!(
            e.request_url("c++ & rust", "de"),
            "https://de.wikipedia.org/w/api.php?action=query&list=search&srsearch=c%2B%2B%20%26%20rust&format=json&srlimit=5&utf8=1"
        );
    }
}
