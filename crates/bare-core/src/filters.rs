//! Content blocking. Filter lists in Adblock Plus syntax (EasyList and friends) are converted to
//! WebKit's content-blocker JSON, which WebKit compiles once and then applies inside the engine:
//! no scripts, no extension process, far cheaper per request than blocking from JavaScript.
//!
//! WebKit rejects a *whole* rule list if any single rule is invalid, so this converts only a
//! conservative subset it can prove valid, and counts what it skipped instead of guessing:
//! - network filters: `||host^`, `|start`, `end|`, `*` and `^` wildcards; options for resource
//!   types, `third-party`, `domain=`, `match-case`; `@@` exceptions;
//! - element hiding (`##selector`, optionally per domain) for simple CSS selectors only.
//!
//! Not converted (skipped): regex filters, scriptlets, procedural cosmetics (`:has-text`, …),
//! `$redirect`/`$csp`/`$removeparam`/`$badfilter`, and cosmetic exceptions (`#@#`).

use serde::Serialize;
use std::collections::{BTreeMap, HashSet};
use std::path::Path;
use std::time::Duration;

/// WebKit refuses rule lists longer than this.
pub const MAX_RULES: usize = 150_000;
/// Selectors merged into one element-hiding rule (fewer rules, same effect).
const SELECTORS_PER_RULE: usize = 100;

/// A short stable name for a converted list (FNV-1a over its text). WebKit caches the compiled list
/// under this name, so an unchanged list is never compiled twice.
pub fn fingerprint(json: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in json.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("bare-{h:016x}")
}

/// A small baseline of well-known ad and tracking hosts, used until lists are downloaded.
pub const BUILTIN: &str = include_str!("../../../resources/filters-builtin.txt");

/// How old the downloaded lists may get before Bare fetches fresh ones by itself.
pub const MAX_LIST_AGE: Duration = Duration::from_secs(7 * 24 * 3600);
/// How long after an attempt (successful or not) before the next one, so a machine that is offline
/// isn't retried on every launch.
pub const RETRY_AFTER: Duration = Duration::from_secs(6 * 3600);

/// Whether to fetch the lists now. `list_age` is the age of the downloaded list (`None`: there is
/// none yet), `attempt_age` the time since the last attempt (`None`: never tried).
pub fn needs_update(list_age: Option<Duration>, attempt_age: Option<Duration>) -> bool {
    let stale = list_age.is_none_or(|age| age >= MAX_LIST_AGE);
    let rested = attempt_age.is_none_or(|age| age >= RETRY_AFTER);
    stale && rested
}

/// How long ago a file was last written; `None` if it doesn't exist. A modification time in the
/// future (the clock was set back) counts as just now, which errs towards not fetching.
pub fn file_age(path: &Path) -> Option<Duration> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    Some(modified.elapsed().unwrap_or(Duration::ZERO))
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Stats {
    /// Network blocking rules.
    pub network: usize,
    /// Element-hiding rules (after merging selectors).
    pub cosmetic: usize,
    /// `@@` exception rules.
    pub exceptions: usize,
    /// Lines that were rules but use features this converter doesn't support.
    pub skipped: usize,
    /// Rules cut to stay within [`MAX_RULES`].
    pub dropped: usize,
}

impl Stats {
    pub fn total(&self) -> usize {
        self.network + self.cosmetic + self.exceptions
    }
}

pub struct Converted {
    pub json: String,
    pub stats: Stats,
}

#[derive(Serialize, Clone, PartialEq, Eq, Hash, Debug)]
struct Rule {
    trigger: Trigger,
    action: Action,
}

#[derive(Serialize, Clone, PartialEq, Eq, Hash, Debug, Default)]
struct Trigger {
    #[serde(rename = "url-filter")]
    url_filter: String,
    #[serde(
        rename = "url-filter-is-case-sensitive",
        skip_serializing_if = "is_false"
    )]
    case_sensitive: bool,
    #[serde(rename = "resource-type", skip_serializing_if = "Vec::is_empty")]
    resource_type: Vec<&'static str>,
    #[serde(rename = "load-type", skip_serializing_if = "Vec::is_empty")]
    load_type: Vec<&'static str>,
    #[serde(rename = "if-domain", skip_serializing_if = "Vec::is_empty")]
    if_domain: Vec<String>,
    #[serde(rename = "unless-domain", skip_serializing_if = "Vec::is_empty")]
    unless_domain: Vec<String>,
}

#[derive(Serialize, Clone, PartialEq, Eq, Hash, Debug)]
struct Action {
    #[serde(rename = "type")]
    kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    selector: Option<String>,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// Every resource type a rule without a type option applies to. (`popup` only on request.)
const ALL_TYPES: [&str; 12] = [
    "document",
    "image",
    "style-sheet",
    "script",
    "font",
    "raw",
    "svg-document",
    "media",
    "ping",
    "fetch",
    "websocket",
    "other",
];

type DomainKey = (Vec<String>, Vec<String>);

/// Convert filter lists to WebKit content-blocker JSON.
pub fn convert<'a>(lists: impl IntoIterator<Item = &'a str>) -> Converted {
    let mut stats = Stats::default();
    let mut blocks: Vec<Rule> = Vec::new();
    let mut exceptions: Vec<Rule> = Vec::new();
    let mut seen: HashSet<Rule> = HashSet::new();
    let mut generic: Vec<String> = Vec::new();
    let mut specific: BTreeMap<DomainKey, Vec<String>> = BTreeMap::new();
    let mut hidden_seen: HashSet<(DomainKey, String)> = HashSet::new();

    for list in lists {
        for line in list.lines() {
            match parse_line(line) {
                Line::Ignore => {}
                Line::Skip => stats.skipped += 1,
                Line::Block(rule) => {
                    if seen.insert(rule.clone()) {
                        blocks.push(rule);
                    }
                }
                Line::Exception(rule) => {
                    if seen.insert(rule.clone()) {
                        exceptions.push(rule);
                    }
                }
                Line::Hide { domains, selectors } => {
                    for sel in selectors {
                        if !hidden_seen.insert((domains.clone(), sel.clone())) {
                            continue;
                        }
                        if domains.0.is_empty() && domains.1.is_empty() {
                            generic.push(sel);
                        } else {
                            specific.entry(domains.clone()).or_default().push(sel);
                        }
                    }
                }
            }
        }
    }

    let mut css: Vec<Rule> = Vec::new();
    for chunk in generic.chunks(SELECTORS_PER_RULE) {
        css.push(hide_rule(&(Vec::new(), Vec::new()), chunk));
    }
    let generic_count = css.len();
    for (domains, sels) in &specific {
        for chunk in sels.chunks(SELECTORS_PER_RULE) {
            css.push(hide_rule(domains, chunk));
        }
    }

    // Stay within WebKit's limit: cut domain-specific hiding first, then generic hiding, then the
    // tail of the network rules. Exceptions are tiny and keep everything else honest, so they stay.
    let mut dropped = 0;
    let room = MAX_RULES.saturating_sub(exceptions.len());
    if blocks.len() + css.len() > room {
        let excess = blocks.len() + css.len() - room;
        let cut_css = excess.min(css.len());
        css.truncate(css.len() - cut_css);
        let cut_blocks = excess - cut_css;
        blocks.truncate(blocks.len() - cut_blocks);
        dropped = excess;
    }
    let _ = generic_count;

    stats.network = blocks.len();
    stats.cosmetic = css.len();
    stats.exceptions = exceptions.len();
    stats.dropped = dropped;

    let mut rules = blocks;
    rules.extend(css);
    rules.extend(exceptions); // `ignore-previous-rules` only affects rules before it
    let json = serde_json::to_string(&rules).unwrap_or_else(|_| "[]".to_string());
    Converted { json, stats }
}

fn hide_rule(domains: &DomainKey, selectors: &[String]) -> Rule {
    let (if_domain, unless_domain) = domains.clone();
    Rule {
        trigger: Trigger {
            url_filter: ".*".into(),
            // WebKit forbids both lists in one trigger; the allow-list is the more specific intent.
            unless_domain: if if_domain.is_empty() {
                unless_domain
            } else {
                Vec::new()
            },
            if_domain,
            ..Trigger::default()
        },
        action: Action {
            kind: "css-display-none",
            selector: Some(selectors.join(", ")),
        },
    }
}

enum Line {
    Ignore,
    Skip,
    Block(Rule),
    Exception(Rule),
    Hide {
        domains: DomainKey,
        selectors: Vec<String>,
    },
}

fn parse_line(raw: &str) -> Line {
    let line = raw.trim();
    if line.is_empty() || line.starts_with('!') || line.starts_with('[') {
        return Line::Ignore;
    }
    if ["#@#", "#?#", "#$#", "#%#", "#@?#", "#@$#"]
        .iter()
        .any(|m| line.contains(m))
    {
        return Line::Skip;
    }
    if let Some((doms, sel)) = line.split_once("##")
        && doms
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || ".,-_~*".contains(c))
    {
        return parse_hide(doms, sel);
    }
    parse_network(line)
}

fn parse_hide(doms: &str, sel: &str) -> Line {
    let Some(domains) = parse_domains(doms, ',') else {
        return Line::Skip;
    };
    let parts = split_selectors(sel);
    if parts.is_empty() || !parts.iter().all(|p| valid_selector(p)) {
        return Line::Skip;
    }
    Line::Hide {
        domains,
        selectors: parts,
    }
}

/// `a.com,~b.com` -> (`["*a.com"]`, `["*b.com"]`). `None` if any entry is something WebKit can't take.
fn parse_domains(list: &str, sep: char) -> Option<DomainKey> {
    let (mut yes, mut no) = (Vec::new(), Vec::new());
    for entry in list.split(sep).map(str::trim).filter(|e| !e.is_empty()) {
        let (neg, host) = match entry.strip_prefix('~') {
            Some(h) => (true, h),
            None => (false, entry),
        };
        let valid = !host.is_empty()
            && host.contains('.')
            && host
                .split('.')
                .all(|l| !l.is_empty() && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'));
        if !valid {
            return None;
        }
        let item = format!("*{}", host.to_ascii_lowercase());
        if neg { no.push(item) } else { yes.push(item) }
    }
    yes.sort();
    no.sort();
    Some((yes, no))
}

/// Split `a, b[x="1,2"], c` at top-level commas.
fn split_selectors(sel: &str) -> Vec<String> {
    let mut out = Vec::new();
    let (mut depth, mut quote, mut start) = (0i32, None::<char>, 0);
    for (i, c) in sel.char_indices() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(c),
            (None, '[') => depth += 1,
            (None, ']') => depth -= 1,
            (None, ',') if depth == 0 => {
                out.push(sel[start..i].trim().to_string());
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(sel[start..].trim().to_string());
    out.retain(|s| !s.is_empty());
    out
}

/// A strict CSS selector grammar: type/`*`, `#id`, `.class`, `[attr]` / `[attr op "value"]`, joined by
/// descendant, `>`, `+` and `~`. No pseudo-classes or escapes: if WebKit disliked one selector it
/// would reject every rule in the list, so anything unusual is out.
pub fn valid_selector(s: &str) -> bool {
    let b = s.trim().as_bytes();
    if b.is_empty() || b.len() > 500 {
        return false;
    }
    let ident = |i: &mut usize| -> bool {
        let start = *i;
        if !matches!(b.get(*i), Some(c) if c.is_ascii_alphabetic() || *c == b'_' || *c == b'-') {
            return false;
        }
        while matches!(b.get(*i), Some(c) if c.is_ascii_alphanumeric() || *c == b'_' || *c == b'-')
        {
            *i += 1;
        }
        *i > start
    };
    let attribute = |i: &mut usize| -> bool {
        *i += 1; // [
        if !ident(i) {
            return false;
        }
        if b.get(*i) == Some(&b']') {
            *i += 1;
            return true;
        }
        match b.get(*i) {
            Some(b'=') => *i += 1,
            Some(b'~' | b'|' | b'^' | b'$' | b'*') if b.get(*i + 1) == Some(&b'=') => *i += 2,
            _ => return false,
        }
        match b.get(*i) {
            Some(&q @ (b'"' | b'\'')) => {
                *i += 1;
                while let Some(&c) = b.get(*i) {
                    if c == q {
                        break;
                    }
                    if c == b'\\' || c == b'\n' {
                        return false;
                    }
                    *i += 1;
                }
                if b.get(*i) != Some(&q) {
                    return false;
                }
                *i += 1;
            }
            _ => {
                let start = *i;
                while matches!(b.get(*i), Some(c) if c.is_ascii_alphanumeric() || b"_-.".contains(c))
                {
                    *i += 1;
                }
                if *i == start {
                    return false;
                }
            }
        }
        if b.get(*i) == Some(&b']') {
            *i += 1;
            true
        } else {
            false
        }
    };

    let mut i = 0;
    loop {
        let start = i;
        if b.get(i) == Some(&b'*') {
            i += 1;
        } else {
            ident(&mut i);
        }
        loop {
            match b.get(i) {
                Some(b'#' | b'.') => {
                    i += 1;
                    if !ident(&mut i) {
                        return false;
                    }
                }
                Some(b'[') => {
                    if !attribute(&mut i) {
                        return false;
                    }
                }
                _ => break,
            }
        }
        if i == start {
            return false;
        }
        let before = i;
        while b.get(i) == Some(&b' ') {
            i += 1;
        }
        match b.get(i) {
            None => return true,
            Some(b'>' | b'+' | b'~') => {
                i += 1;
                while b.get(i) == Some(&b' ') {
                    i += 1;
                }
                if i >= b.len() {
                    return false;
                }
            }
            Some(_) if i > before => {} // descendant combinator
            Some(_) => return false,
        }
    }
}

#[derive(Default)]
struct Options {
    types: Vec<&'static str>,
    not_types: Vec<&'static str>,
    load: Option<&'static str>,
    domains: DomainKey,
    case_sensitive: bool,
}

fn parse_network(line: &str) -> Line {
    let (exception, body) = match line.strip_prefix("@@") {
        Some(rest) => (true, rest),
        None => (false, line),
    };
    // Options follow the last `$`, if what follows looks like options.
    let (pattern, opts_text) = match body.rfind('$') {
        Some(i)
            if body[i + 1..]
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "~-_=|,.*".contains(c)) =>
        {
            (&body[..i], Some(&body[i + 1..]))
        }
        _ => (body, None),
    };
    if pattern.starts_with('/') && pattern.ends_with('/') && pattern.len() > 1 {
        return Line::Skip; // a regular expression filter
    }
    let Some(opts) = parse_options(opts_text.unwrap_or("")) else {
        return Line::Skip;
    };
    let lowered;
    let pattern = if opts.case_sensitive {
        pattern
    } else {
        lowered = pattern.to_ascii_lowercase();
        &lowered
    };
    let Some(url_filter) = pattern_to_regex(pattern) else {
        return Line::Skip;
    };

    let resource_type = if !opts.types.is_empty() {
        opts.types
            .iter()
            .copied()
            .filter(|t| !opts.not_types.contains(t))
            .collect()
    } else if !opts.not_types.is_empty() {
        ALL_TYPES
            .iter()
            .copied()
            .filter(|t| !opts.not_types.contains(t))
            .collect()
    } else {
        Vec::new()
    };
    let (if_domain, unless_domain) = opts.domains;
    let rule = Rule {
        trigger: Trigger {
            url_filter,
            case_sensitive: opts.case_sensitive,
            resource_type,
            load_type: opts.load.into_iter().collect(),
            unless_domain: if if_domain.is_empty() {
                unless_domain
            } else {
                Vec::new()
            },
            if_domain,
        },
        action: Action {
            kind: if exception {
                "ignore-previous-rules"
            } else {
                "block"
            },
            selector: None,
        },
    };
    if exception {
        Line::Exception(rule)
    } else {
        Line::Block(rule)
    }
}

fn parse_options(text: &str) -> Option<Options> {
    let mut o = Options::default();
    for opt in text.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        let (neg, name) = match opt.strip_prefix('~') {
            Some(n) => (true, n),
            None => (false, opt),
        };
        if let Some(list) = name.strip_prefix("domain=") {
            o.domains = parse_domains(list, '|')?;
            continue;
        }
        match name {
            "third-party" | "3p" => o.load = Some(if neg { "first-party" } else { "third-party" }),
            "first-party" | "1p" => o.load = Some(if neg { "third-party" } else { "first-party" }),
            "match-case" => o.case_sensitive = true,
            "important" => {}
            _ => {
                let types = resource_types(name)?;
                let list = if neg { &mut o.not_types } else { &mut o.types };
                list.extend(types);
            }
        }
    }
    Some(o)
}

fn resource_types(name: &str) -> Option<&'static [&'static str]> {
    Some(match name {
        "script" => &["script"],
        "image" => &["image", "svg-document"],
        "stylesheet" | "css" => &["style-sheet"],
        "font" => &["font"],
        "media" => &["media"],
        "object" | "other" => &["other"],
        "xmlhttprequest" | "xhr" => &["raw", "fetch"],
        "subdocument" | "frame" => &["document"],
        "ping" => &["ping"],
        "websocket" => &["websocket"],
        "popup" => &["popup"],
        _ => return None,
    })
}

/// ABP pattern -> the regular-expression subset WebKit accepts (no alternation, no lookaround).
fn pattern_to_regex(pattern: &str) -> Option<String> {
    let mut p = pattern;
    let mut out = String::new();
    let anchored_start = if let Some(rest) = p.strip_prefix("||") {
        out.push_str("^[^:]+://+([^:/]+\\.)?");
        p = rest;
        true
    } else if let Some(rest) = p.strip_prefix('|') {
        out.push('^');
        p = rest;
        true
    } else {
        false
    };
    let anchored_end = p.ends_with('|');
    if anchored_end {
        p = &p[..p.len() - 1];
    }
    if !anchored_start {
        p = p.trim_start_matches('*');
    }
    if !anchored_end {
        p = p.trim_end_matches('*');
    }
    // Too little literal text matches far too much (`/a`, `.js`).
    let literal = p.chars().filter(|c| c.is_ascii_alphanumeric()).count();
    if literal < 3 && !(anchored_start && literal >= 1 && p.contains('.')) {
        return None;
    }
    for c in p.chars() {
        match c {
            '*' => out.push_str(".*"),
            '^' => out.push_str("[^a-zA-Z0-9_.%-]"),
            c if c.is_ascii_alphanumeric() || "-_%/=&:,;@!'~#".contains(c) => out.push(c),
            '.' | '+' | '?' | '(' | ')' | '[' | ']' | '{' | '}' | '$' | '|' | '\\' => {
                out.push('\\');
                out.push(c);
            }
            _ => return None,
        }
    }
    if anchored_end {
        out.push('$');
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn rules(list: &str) -> Vec<Value> {
        let c = convert([list]);
        serde_json::from_str::<Vec<Value>>(&c.json).unwrap()
    }
    fn one(list: &str) -> Value {
        let mut r = rules(list);
        assert_eq!(r.len(), 1, "{list} -> {r:?}");
        r.remove(0)
    }

    #[test]
    fn lists_refresh_when_missing_or_a_week_old() {
        let hours = |h: u64| Some(Duration::from_secs(h * 3600));
        // No list yet: fetch.
        assert!(needs_update(None, None));
        // A fresh list: leave it alone, however long ago we last tried.
        assert!(!needs_update(hours(1), None));
        assert!(!needs_update(hours(24 * 7 - 1), hours(1000)));
        // A week old: fetch.
        assert!(needs_update(hours(24 * 7), None));
        assert!(needs_update(hours(24 * 30), hours(7)));
    }

    #[test]
    fn a_recent_attempt_holds_off_the_next_one() {
        let hours = |h: u64| Some(Duration::from_secs(h * 3600));
        // Offline machine: no list, but we tried an hour ago.
        assert!(!needs_update(None, hours(1)));
        assert!(!needs_update(
            hours(24 * 30),
            Some(RETRY_AFTER - Duration::from_secs(1))
        ));
        assert!(needs_update(None, Some(RETRY_AFTER)));
    }

    #[test]
    fn file_age_is_none_for_a_missing_file_and_small_for_a_new_one() {
        let dir = std::env::temp_dir().join(format!("bare-age-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("list");
        assert_eq!(file_age(&f), None);
        std::fs::write(&f, "x").unwrap();
        assert!(file_age(&f).unwrap() < Duration::from_secs(60));
        std::fs::remove_dir_all(&dir).unwrap();
    }
    fn filter(list: &str) -> String {
        one(list)["trigger"]["url-filter"]
            .as_str()
            .unwrap()
            .to_string()
    }

    #[test]
    fn domain_anchored_pattern() {
        assert_eq!(
            filter("||example.com^"),
            r"^[^:]+://+([^:/]+\.)?example\.com[^a-zA-Z0-9_.%-]"
        );
        assert_eq!(one("||example.com^")["action"]["type"], "block");
    }

    #[test]
    fn plain_anchored_and_wildcard_patterns() {
        assert_eq!(filter("/ads/banner."), r"/ads/banner\.");
        assert_eq!(filter("|https://foo.com/x|"), r"^https://foo\.com/x$");
        assert_eq!(filter("*/ad.js"), r"/ad\.js");
        assert_eq!(filter("/track?id=*&x"), r"/track\?id=.*&x");
        assert_eq!(
            filter("||CDN.Example.COM/Ads/"),
            r"^[^:]+://+([^:/]+\.)?cdn\.example\.com/ads/",
            "case-insensitive by default"
        );
    }

    #[test]
    fn options_become_triggers() {
        let r = one("||ads.example.com^$script,third-party");
        assert_eq!(r["trigger"]["resource-type"], serde_json::json!(["script"]));
        assert_eq!(
            r["trigger"]["load-type"],
            serde_json::json!(["third-party"])
        );

        let r = one("/banner/ad$~image,~script");
        let types: Vec<_> = r["trigger"]["resource-type"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert!(
            !types.contains(&"image") && !types.contains(&"script") && types.contains(&"font"),
            "{types:?}"
        );

        let r = one("||x.com^$domain=a.com|b.org");
        assert_eq!(
            r["trigger"]["if-domain"],
            serde_json::json!(["*a.com", "*b.org"])
        );
        let r = one("||x.com^$domain=~a.com");
        assert_eq!(r["trigger"]["unless-domain"], serde_json::json!(["*a.com"]));
        let r = one("||x.com^$domain=a.com|~b.com");
        assert!(
            r["trigger"].get("unless-domain").is_none(),
            "WebKit forbids both lists"
        );
        assert_eq!(
            one("||x.com^$match-case")["trigger"]["url-filter-is-case-sensitive"],
            true
        );
        assert_eq!(
            one("||x.com^$xhr")["trigger"]["resource-type"],
            serde_json::json!(["raw", "fetch"])
        );
        assert_eq!(
            one("||x.com^$1p")["trigger"]["load-type"],
            serde_json::json!(["first-party"])
        );
    }

    #[test]
    fn exceptions_come_last_and_ignore_previous_rules() {
        let r = rules("@@||ok.example.com^\n||example.com^\n##.ad");
        let kinds: Vec<_> = r
            .iter()
            .map(|x| x["action"]["type"].as_str().unwrap())
            .collect();
        assert_eq!(
            kinds,
            ["block", "css-display-none", "ignore-previous-rules"]
        );
    }

    #[test]
    fn unsupported_lines_are_skipped_and_counted() {
        let list = "||a.com^$redirect=nooptext\n||b.com^$csp=script-src 'self'\n||c.com^$badfilter\n/ads\\d+/\n||d.com^$removeparam=utm\n\
                    ##a:has-text(Sponsored)\nexample.com#@#.ad\nexample.com##+js(aopr, x)\nexample.com#?#div:-abp-has(.x)\n||é.com^\n/a\n.js\n";
        let c = convert([list]);
        assert_eq!(c.stats.total(), 0, "{}", c.json);
        assert_eq!(c.stats.skipped, 12);
        assert_eq!(c.json, "[]");
    }

    #[test]
    fn comments_blanks_and_headers_are_ignored_not_skipped() {
        let c = convert(["[Adblock Plus 2.0]\n! Title: x\n\n   \n||ok.example^\n"]);
        assert_eq!((c.stats.network, c.stats.skipped), (1, 0));
    }

    #[test]
    fn element_hiding_is_merged_and_scoped() {
        let r = rules(
            "##.ad-banner\n###sponsor\n##div.promo > a[href^=\"http://ads.\"]\nexample.com,~sub.example.com##.x\nexample.org##.y\nexample.org##.z",
        );
        assert_eq!(r.len(), 3, "{r:?}");
        let generic = &r[0];
        assert_eq!(generic["trigger"]["url-filter"], ".*");
        assert_eq!(
            generic["action"]["selector"],
            ".ad-banner, #sponsor, div.promo > a[href^=\"http://ads.\"]"
        );
        let scoped: Vec<_> = r[1..]
            .iter()
            .map(|x| {
                (
                    x["trigger"]["if-domain"].clone(),
                    x["action"]["selector"].as_str().unwrap().to_string(),
                )
            })
            .collect();
        assert!(
            scoped.contains(&(serde_json::json!(["*example.org"]), ".y, .z".to_string())),
            "{scoped:?}"
        );
        assert!(
            scoped.contains(&(serde_json::json!(["*example.com"]), ".x".to_string())),
            "negatives dropped when positives exist"
        );
    }

    #[test]
    fn negative_only_domains_become_unless_domain() {
        let r = one("~news.example.com##.ad");
        assert_eq!(
            r["trigger"]["unless-domain"],
            serde_json::json!(["*news.example.com"])
        );
    }

    #[test]
    fn selector_grammar_accepts_simple_and_rejects_risky() {
        for ok in [
            ".ad",
            "#top",
            "div",
            "*",
            "div.a.b#c",
            "a[href]",
            "a[href=\"x\"]",
            "a[href^='http://ads.']",
            "a[data-x=y]",
            "ul > li",
            "h1 + p",
            "h1 ~ p",
            "div p span",
            ".a .b > .c",
        ] {
            assert!(valid_selector(ok), "should accept {ok}");
        }
        for bad in [
            "",
            "a:hover",
            "a:not(.b)",
            "a::before",
            ".a,.b",
            "div >",
            "> div",
            "a[href",
            "a[href=]",
            "a[=x]",
            ".",
            "#",
            "1abc",
            "a\\:b",
            "a[x=\"un\\\"closed\"]",
            "a b>",
            "div{x}",
            "a;b",
        ] {
            assert!(!valid_selector(bad), "should reject {bad}");
        }
    }

    #[test]
    fn comma_lists_split_only_at_top_level_and_all_must_be_valid() {
        assert_eq!(
            split_selectors("a, b[x=\"1,2\"], c"),
            ["a", "b[x=\"1,2\"]", "c"]
        );
        assert_eq!(rules("##.a, .b").len(), 1);
        assert_eq!(
            convert(["##.a, .b:hover"]).stats.skipped,
            1,
            "one bad selector skips the whole line"
        );
    }

    #[test]
    fn duplicates_are_removed_across_lists() {
        let c = convert(["||ads.example^\n##.ad", "||ads.example^\n##.ad"]);
        assert_eq!((c.stats.network, c.stats.cosmetic), (1, 1));
    }

    #[test]
    fn the_rule_limit_cuts_cosmetics_before_network_rules_and_keeps_exceptions() {
        let mut list = String::new();
        for i in 0..MAX_RULES {
            list.push_str(&format!("||host{i}.example^\n"));
        }
        for i in 0..2000 {
            list.push_str(&format!("site{i}.example.org##.ad\n"));
        }
        list.push_str("@@||ok.example^\n");
        let c = convert([list.as_str()]);
        assert_eq!(c.stats.total(), MAX_RULES);
        assert_eq!(c.stats.exceptions, 1);
        assert_eq!(c.stats.cosmetic, 0, "cosmetics are cut first");
        assert_eq!(c.stats.dropped, 2001);
        assert_eq!(
            serde_json::from_str::<Vec<Value>>(&c.json).unwrap().len(),
            MAX_RULES
        );
    }

    #[test]
    fn every_url_filter_stays_within_the_characters_webkit_accepts() {
        let patterns = [
            "||a.com^",
            "|http://x.y/z|",
            "/foo/bar?x=1&y=2",
            "||ex.com/a(b)[c]{d}+e",
            "/path;x=1,y@z!'q~#frag",
            "/aaa|bbb",
            "*foo*bar*",
            "||ex.com^*/ad/*^",
        ];
        for p in patterns {
            let f = filter(p);
            assert!(f.chars().all(|c| c.is_ascii_graphic()), "{p} -> {f}");
            assert!(
                !f.contains("(?") && !f.contains('|') || f.contains("\\|"),
                "{p} -> {f}: no alternation"
            );
        }
    }

    #[test]
    fn fingerprints_are_stable_and_sensitive() {
        assert_eq!(fingerprint("[]"), fingerprint("[]"));
        assert_ne!(fingerprint("[]"), fingerprint("[ ]"));
        assert_eq!(fingerprint(""), "bare-cbf29ce484222325");
    }

    #[test]
    fn the_builtin_baseline_converts_completely() {
        let c = convert([BUILTIN]);
        assert_eq!(
            c.stats.skipped, 0,
            "every line of the built-in list must be supported"
        );
        assert!(c.stats.network >= 50, "{:?}", c.stats);
        assert!(
            c.json.len() < 64 * 1024,
            "the built-in list should stay tiny, was {} bytes",
            c.json.len()
        );
        for r in serde_json::from_str::<Vec<Value>>(&c.json).unwrap() {
            assert_eq!(
                r["trigger"]["load-type"],
                serde_json::json!(["third-party"]),
                "built-in rules only apply to third parties: {r}"
            );
        }
    }
}
