//! Run the metasearch pipeline from the terminal (an example, not a binary, so that a plain
//! `cargo run` at the workspace root runs the browser). `cargo bs` is an alias for this example.
//!   cargo bs rust lifetimes            ranked results, plus what each engine did
//!   cargo bs '!w rust'                 only Wikipedia (also: !gh !so !aw, or a category: !it)
//!   cargo bs engines                   list the engines
//!   cargo bs probe                     ask every engine a few canned queries; report health

use bare_search::{Searcher, Status};
use std::time::Duration;

const HELP: &str = "usage: cargo bs <query> | engines | probe\n  query prefixes: !engine !category :lang   e.g. cargo bs '!it :en tokio runtime'\n";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None | Some("-h") | Some("--help") => print!("{HELP}"),
        Some("engines") => engines(),
        Some("probe") => probe(),
        Some(_) => search(&args.join(" ")),
    }
}

fn engines() {
    for e in Searcher::builtin().engines() {
        println!(
            "{:<14} !{:<4} {:<10} weight {:<4} timeout {} ms",
            e.name(),
            e.def.shortcut,
            e.def.categories.join(","),
            e.def.weight,
            e.def.timeout_ms
        );
    }
}

fn search(query: &str) {
    let r = Searcher::builtin().search(query);
    if r.query.is_empty() {
        eprintln!("nothing to search for");
        std::process::exit(2);
    }
    println!(
        "{} — {} results from {} of {} engines in {:.2} s\n",
        r.query,
        r.items.len(),
        r.answered(),
        r.asked(),
        r.elapsed.as_secs_f64()
    );
    for (i, item) in r.items.iter().enumerate() {
        println!("{:>2}. {}", i + 1, item.title);
        println!("    {}", item.url);
        if !item.content.is_empty() {
            println!("    {}", truncate(&item.content, 150));
        }
        println!(
            "    [{}] score {:.2}\n",
            item.engines.join(", "),
            item.score
        );
    }
    if !r.suggestions.is_empty() {
        println!("did you mean: {}\n", r.suggestions.join(", "));
    }
    for rep in &r.reports {
        println!(
            "  {:<14} {:>5} ms  {}",
            rep.engine,
            rep.elapsed.as_millis(),
            describe(&rep.status)
        );
    }
}

fn probe() {
    let searcher = Searcher::builtin();
    let queries = [
        "rust lifetimes",
        "linux kernel parameters",
        "how to tie a tie",
    ];
    println!(
        "{:<14} {:>3} {:>5} {:>7} {:>6}   notes",
        "engine", "ok", "empty", "blocked", "failed"
    );
    for e in searcher.engines() {
        let (mut ok, mut empty, mut blocked, mut failed) = (0, 0, 0, 0);
        let mut notes = Vec::new();
        let mut times = Vec::new();
        for q in queries {
            let rep = searcher.probe(e, q);
            times.push(rep.elapsed);
            match &rep.status {
                Status::Ok(_) => ok += 1,
                Status::Empty => empty += 1,
                Status::Blocked(w) => {
                    blocked += 1;
                    notes.push(w.clone());
                }
                Status::Failed(w) => {
                    failed += 1;
                    notes.push(w.clone());
                }
                Status::Suspended(_) | Status::Late => {}
            }
            std::thread::sleep(Duration::from_millis(1200)); // be polite: these are other people's servers
        }
        times.sort();
        notes.dedup();
        println!(
            "{:<14} {:>3} {:>5} {:>7} {:>6}   median {} ms {}",
            e.name(),
            ok,
            empty,
            blocked,
            failed,
            times[times.len() / 2].as_millis(),
            notes.join("; ")
        );
    }
}

fn describe(s: &Status) -> String {
    match s {
        Status::Ok(n) => format!("ok, {n} results"),
        Status::Empty => "answered, no results".into(),
        Status::Blocked(w) => format!("BLOCKED: {w}"),
        Status::Failed(w) => format!("failed: {w}"),
        Status::Suspended(s) => format!("suspended, {s} s left"),
        Status::Late => "still working; not waited for".into(),
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(max).collect::<String>().trim_end())
    }
}
