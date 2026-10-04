//! Times the pure parts of a search (parse, merge, render) on the recorded engine responses.
//!   cargo run --release -p bare-search --example bench
use bare_search::{builtin, merge, render, search};
use std::time::{Duration, Instant};

fn timed<T>(label: &str, n: u32, mut f: impl FnMut() -> T) -> T {
    let mut last = f();
    let t = Instant::now();
    for _ in 0..n {
        last = f();
    }
    let per = t.elapsed() / n;
    println!("{label:<44} {per:>10.1?}");
    last
}

fn main() {
    let fx = |name: &str| {
        std::fs::read_to_string(format!("crates/bare-search/tests/fixtures/{name}")).unwrap()
    };
    let cases = [
        ("bing", fx("bing.html")),
        ("duckduckgo", fx("ddg.html")),
        ("wikipedia", fx("wikipedia.json")),
        ("marginalia", fx("marginalia.json")),
    ];
    let engines = builtin::engines();

    let mut lists = Vec::new();
    for (name, body) in &cases {
        let e = engines.iter().find(|e| e.name() == *name).unwrap();
        let url = e.request_url("rust lifetimes", "en");
        let parsed = timed(
            &format!("parse {name} ({} KB)", body.len() / 1024),
            200,
            || e.parse(body, &url, "en").unwrap(),
        );
        lists.push(merge::EngineResults {
            engine: name.to_string(),
            weight: 1.0,
            results: parsed.results,
        });
    }
    timed(
        "build the 7 engine definitions (startup)",
        50,
        builtin::engines,
    );
    let items = timed("merge 4 engines' results", 500, || {
        merge::merge(lists.clone())
    });
    let response = search::Response {
        query: "rust lifetimes".into(),
        items,
        ..Default::default()
    };
    let page = timed(
        &format!("render the results page ({} results)", response.items.len()),
        500,
        || render::page(&response),
    );
    println!(
        "{:<44} {:>10}",
        "results page size",
        format!("{} KB", page.len() / 1024)
    );
    let _ = Duration::ZERO;
}
