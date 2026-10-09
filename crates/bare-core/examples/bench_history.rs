//! Times history suggestions on a large synthetic history, and recording a visit (both run on the
//! UI thread) in a database file.
//!   cargo run --release -p bare-core --example bench_history
use bare_core::{History, suggest};
use std::time::Instant;

fn main() {
    let h = History::open_in_memory().unwrap();
    let now = 1_800_000_000;
    let t = Instant::now();
    for i in 0..200_000i64 {
        h.record_visit(
            &format!(
                "https://site{}.example.org/articles/{}/page-{i}",
                i % 997,
                i % 31
            ),
            &format!(
                "Article number {i} about topic {} on site {}",
                i % 503,
                i % 997
            ),
            now - (i % 90) * 86_400,
        )
        .unwrap();
    }
    println!("inserted 200,000 rows in {:.1?}", t.elapsed());
    for q in [
        "",
        "a",
        "article",
        "site12",
        "topic 77 site",
        "page-123456",
        "zzzz-no-match",
    ] {
        let toks = suggest::tokens(q);
        let t = Instant::now();
        let n = 20;
        for _ in 0..n {
            let _ = h.search(&toks, 20, now).unwrap();
        }
        println!("search {q:<16} {:>9.1?} per keystroke", t.elapsed() / n);
    }

    let dir = std::env::temp_dir().join(format!("bare-bench-history-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let h = History::open(&dir.join("history.sqlite")).unwrap();
    let n = 200;
    let t = Instant::now();
    for i in 0..n {
        h.record_visit(&format!("https://example.org/{i}"), "A page", now)
            .unwrap();
    }
    println!(
        "record a visit (file)  {:>9.1?} per page load",
        t.elapsed() / n
    );
    drop(h);
    std::fs::remove_dir_all(&dir).unwrap();
}
