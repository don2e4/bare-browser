//! `bare --update-filters`: download EasyList and EasyPrivacy, convert them for WebKit, and store
//! the result. Runs without a window. A running Bare starts this itself (as a child process, so the
//! download and the conversion's memory never touch the browser) when the lists are missing or a
//! week old, and loads the result when it finishes.

use bare_core::{Paths, filters};
use std::time::{Duration, Instant};

const LISTS: [(&str, &str); 2] = [
    (
        "EasyList",
        "https://easylist-downloads.adblockplus.org/easylist.txt",
    ),
    (
        "EasyPrivacy",
        "https://easylist-downloads.adblockplus.org/easyprivacy.txt",
    ),
];

const MAX_LIST_BYTES: u64 = 32 * 1024 * 1024;

/// What to fetch, and how many rules the result must have to be believed. Normally EasyList and
/// EasyPrivacy; `BARE_FILTER_LISTS=url,url` replaces them (for the tests, which serve their own).
fn lists() -> (Vec<(String, String)>, usize) {
    match std::env::var("BARE_FILTER_LISTS") {
        Ok(urls) if !urls.trim().is_empty() => (
            urls.split(',')
                .map(|u| (u.trim().to_string(), u.trim().to_string()))
                .collect(),
            1,
        ),
        _ => (
            LISTS
                .iter()
                .map(|(n, u)| (n.to_string(), u.to_string()))
                .collect(),
            1000,
        ),
    }
}

pub fn run(paths: &Paths) -> Result<(), String> {
    // Stamp the attempt first: a failed one must not be repeated on the next launch.
    let _ = std::fs::write(paths.filters_attempt_file(), "");
    let (lists, min_rules) = lists();
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(10)))
        .timeout_global(Some(Duration::from_secs(90)))
        .build()
        .into();
    let mut texts = Vec::new();
    for (name, url) in &lists {
        print!("downloading {name}... ");
        let started = Instant::now();
        let mut response = agent
            .get(url.as_str())
            .header(
                "User-Agent",
                concat!("Bare/", env!("CARGO_PKG_VERSION"), " (filter update)"),
            )
            .call()
            .map_err(|e| format!("{name}: {e}"))?;
        let text = response
            .body_mut()
            .with_config()
            .limit(MAX_LIST_BYTES)
            .read_to_string()
            .map_err(|e| format!("{name}: {e}"))?;
        println!("{} lines, {:.1?}", text.lines().count(), started.elapsed());
        texts.push(text);
    }
    let converted = filters::convert(texts.iter().map(String::as_str));
    let s = &converted.stats;
    println!(
        "converted to {} rules (network {}, element hiding {}, exceptions {}); skipped {} unsupported lines{}",
        s.total(),
        s.network,
        s.cosmetic,
        s.exceptions,
        s.skipped,
        if s.dropped > 0 {
            format!(", dropped {} over WebKit's limit", s.dropped)
        } else {
            String::new()
        }
    );
    if s.total() < min_rules {
        return Err(
            "the downloaded lists produced almost no rules; keeping the current ones".into(),
        );
    }
    paths.ensure();
    let file = paths.filters_file();
    let tmp = file.with_extension("json.tmp");
    std::fs::write(&tmp, &converted.json).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, &file).map_err(|e| format!("{}: {e}", file.display()))?;
    std::fs::write(
        paths.filters_id_file(),
        filters::fingerprint(&converted.json),
    )
    .map_err(|e| format!("{}: {e}", paths.filters_id_file().display()))?;
    println!(
        "saved {}\nRestart Bare to use the new lists.",
        file.display()
    );
    Ok(())
}
