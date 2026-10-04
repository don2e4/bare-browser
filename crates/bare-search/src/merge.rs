//! Merging per-engine result lists into one ranking. Dedupe and scoring follow SearXNG's
//! `results.py`: the same page from several engines becomes one result, and
//! `score = (Π engine weights) × (number of engines) × Σ 1/position`, so agreement between engines
//! and a high position in each both push a result up.

use crate::engine::RawResult;
use std::collections::HashMap;
use url::Url;

#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub url: String,
    pub title: String,
    pub content: String,
    /// Engines that returned this result, in the order they were merged.
    pub engines: Vec<String>,
    /// 1-based position in each of those engines' lists.
    pub positions: Vec<usize>,
    pub score: f64,
}

/// One engine's answer, with the weight it was configured with.
#[derive(Debug, Clone)]
pub struct EngineResults {
    pub engine: String,
    pub weight: f64,
    pub results: Vec<RawResult>,
}

pub fn merge(lists: Vec<EngineResults>) -> Vec<Item> {
    let weights: HashMap<String, f64> =
        lists.iter().map(|l| (l.engine.clone(), l.weight)).collect();
    let mut items: Vec<Item> = Vec::new();
    let mut by_key: HashMap<String, usize> = HashMap::new();

    for list in lists {
        for (i, raw) in list.results.into_iter().enumerate() {
            let position = i + 1;
            let Some((key, cleaned)) = normalize(&raw.url) else {
                continue;
            };
            match by_key.get(&key) {
                Some(&idx) => {
                    let item = &mut items[idx];
                    if !item.engines.contains(&list.engine) {
                        item.engines.push(list.engine.clone());
                    }
                    item.positions.push(position);
                    if raw.content.chars().count() > item.content.chars().count() {
                        item.content = raw.content;
                    }
                    // Prefer the https spelling of the same page.
                    if item.url.starts_with("http://") && cleaned.starts_with("https://") {
                        item.url = cleaned;
                    }
                }
                None => {
                    by_key.insert(key, items.len());
                    items.push(Item {
                        url: cleaned,
                        title: raw.title,
                        content: raw.content,
                        engines: vec![list.engine.clone()],
                        positions: vec![position],
                        score: 0.0,
                    });
                }
            }
        }
    }

    for item in &mut items {
        item.score = score(item, &weights);
    }
    // Stable sort: ties keep merge order, i.e. the first engine to list them.
    items.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    items
}

fn score(item: &Item, weights: &HashMap<String, f64>) -> f64 {
    let weight: f64 = item
        .engines
        .iter()
        .map(|e| weights.get(e).copied().unwrap_or(1.0))
        .product::<f64>()
        * item.positions.len() as f64;
    item.positions.iter().map(|&p| weight / p as f64).sum()
}

/// Query parameters that only exist to track you.
fn is_tracking(param: &str) -> bool {
    param.starts_with("utm_")
        || matches!(
            param,
            "fbclid"
                | "gclid"
                | "msclkid"
                | "mc_cid"
                | "mc_eid"
                | "igshid"
                | "yclid"
                | "_hsenc"
                | "_hsmi"
        )
}

/// (dedupe key, cleaned URL). The key ignores scheme, `www.`, a trailing slash and the fragment.
pub fn normalize(url: &str) -> Option<(String, String)> {
    let mut u = Url::parse(url).ok()?;
    let host = u.host_str()?.to_lowercase();

    let kept: Vec<(String, String)> = u
        .query_pairs()
        .filter(|(k, _)| !is_tracking(k))
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    if kept.is_empty() {
        u.set_query(None);
    } else {
        u.query_pairs_mut().clear().extend_pairs(kept.iter());
    }
    u.set_fragment(None);

    let path = u.path().trim_end_matches('/');
    let query = u.query().map(|q| format!("?{q}")).unwrap_or_default();
    let key = format!(
        "{}{}{}",
        host.strip_prefix("www.").unwrap_or(&host),
        path,
        query
    );
    Some((key, u.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(url: &str, content: &str) -> RawResult {
        RawResult {
            url: url.into(),
            title: format!("title of {url}"),
            content: content.into(),
        }
    }
    fn list(engine: &str, weight: f64, urls: &[&str]) -> EngineResults {
        EngineResults {
            engine: engine.into(),
            weight,
            results: urls.iter().map(|u| raw(u, "")).collect(),
        }
    }

    #[test]
    fn same_page_from_two_engines_is_one_result() {
        let items = merge(vec![
            list("a", 1.0, &["https://example.org/x/", "https://other.net/"]),
            list("b", 1.0, &["http://www.example.org/x#frag"]),
        ]);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].engines, ["a", "b"]);
        assert_eq!(items[0].positions, [1, 1]);
        assert_eq!(
            items[0].url, "https://example.org/x/",
            "https spelling wins"
        );
    }

    #[test]
    fn tracking_parameters_are_dropped_but_real_ones_kept() {
        let (key, url) =
            normalize("https://www.Example.org/p?utm_source=x&id=7&fbclid=y#top").unwrap();
        assert_eq!(url, "https://www.example.org/p?id=7");
        assert_eq!(key, "example.org/p?id=7");
        let (_, plain) = normalize("https://example.org/p?utm_source=x").unwrap();
        assert_eq!(plain, "https://example.org/p");
    }

    #[test]
    fn score_matches_the_searxng_formula() {
        // Two engines (weights 1.0 and 2.0) list the page at positions 1 and 2:
        // weight = 1.0 * 2.0 * 2 positions = 4; score = 4/1 + 4/2 = 6.
        let items = merge(vec![
            list("a", 1.0, &["https://x.org/"]),
            list("b", 2.0, &["https://y.org/", "https://x.org/"]),
        ]);
        let x = items.iter().find(|i| i.url == "https://x.org/").unwrap();
        assert!((x.score - 6.0).abs() < 1e-9, "score was {}", x.score);
    }

    #[test]
    fn agreement_beats_a_lone_first_place() {
        let items = merge(vec![
            list("a", 1.0, &["https://lone.org/", "https://agreed.org/"]),
            list("b", 1.0, &["https://agreed.org/"]),
        ]);
        assert_eq!(items[0].url, "https://agreed.org/");
    }

    #[test]
    fn longer_snippet_wins_and_ties_keep_merge_order() {
        let mut a = list("a", 1.0, &["https://x.org/"]);
        a.results[0].content = "short".into();
        let mut b = list("b", 1.0, &["https://x.org/"]);
        b.results[0].content = "a much longer snippet".into();
        let items = merge(vec![a, b]);
        assert_eq!(items[0].content, "a much longer snippet");

        let items = merge(vec![list(
            "a",
            1.0,
            &["https://one.org/", "https://two.org/"],
        )]);
        let order: Vec<_> = items.iter().map(|i| i.url.as_str()).collect();
        assert_eq!(order, ["https://one.org/", "https://two.org/"]);
    }

    #[test]
    fn junk_urls_are_skipped() {
        let items = merge(vec![list("a", 1.0, &["not a url", "https://ok.org/"])]);
        assert_eq!(items.len(), 1);
    }
}
