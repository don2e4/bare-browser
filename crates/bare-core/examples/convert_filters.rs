//! Convert filter-list files and report what happened:
//!   cargo run --release -p bare-core --example convert_filters -- easylist.txt easyprivacy.txt [-o out.json]
use std::time::Instant;

fn main() {
    let mut files = Vec::new();
    let mut out = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        if a == "-o" {
            out = args.next();
        } else {
            files.push(a);
        }
    }
    let texts: Vec<String> = files
        .iter()
        .map(|f| std::fs::read_to_string(f).unwrap_or_else(|e| panic!("{f}: {e}")))
        .collect();
    let started = Instant::now();
    let c = bare_core::filters::convert(texts.iter().map(String::as_str));
    let took = started.elapsed();
    let lines: usize = texts.iter().map(|t| t.lines().count()).sum();
    println!(
        "{lines} lines -> {} rules in {took:.0?}  (network {}, element hiding {}, exceptions {}; skipped {}, dropped {}); {:.1} MB of JSON",
        c.stats.total(),
        c.stats.network,
        c.stats.cosmetic,
        c.stats.exceptions,
        c.stats.skipped,
        c.stats.dropped,
        c.json.len() as f64 / 1_048_576.0
    );
    if let Some(path) = out {
        std::fs::write(&path, &c.json).unwrap();
        println!("wrote {path}");
    }
}
