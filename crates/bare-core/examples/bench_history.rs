//! Times history suggestions on a large synthetic history.
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
    for q in ["", "a", "site12", "topic 77 site", "zzzz-no-match"] {
        let toks = suggest::tokens(q);
        let t = Instant::now();
        let n = 20;
        for _ in 0..n {
            let _ = h.search(&toks, 20, now).unwrap();
        }
        println!("search {q:<16} {:>9.1?} per keystroke", t.elapsed() / n);
    }
}
