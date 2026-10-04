//! Offline tests: every engine parsed from a real recorded response, and the whole pipeline
//! (select, fan-out, parse, back-off, merge) run against a fake network.

use bare_search::builtin;
use bare_search::engine::{Engine, ParseError};
use bare_search::fetch::{FetchError, Fetched, Fetcher, Request};
use bare_search::{Searcher, Status};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

const BING: &str = include_str!("fixtures/bing.html");
const DDG: &str = include_str!("fixtures/ddg.html");
const WIKIPEDIA: &str = include_str!("fixtures/wikipedia.json");
const ARCHWIKI: &str = include_str!("fixtures/archwiki.json");
const STACKEXCHANGE: &str = include_str!("fixtures/stackexchange.json");
const GITHUB: &str = include_str!("fixtures/github.json");
const MARGINALIA: &str = include_str!("fixtures/marginalia.json");
const DDG_ANOMALY: &str = include_str!("fixtures/ddg-anomaly.html");
const SEARXNG: &str = include_str!("fixtures/searxng.json");
const MOJEEK_CAPTCHA: &str = include_str!("fixtures/mojeek-captcha.html");
const BRAVE_429: &str = include_str!("fixtures/brave-429.html");

fn engine(name: &str) -> Engine {
    builtin::engines()
        .into_iter()
        .find(|e| e.name() == name)
        .unwrap_or_else(|| panic!("no engine {name}"))
}

fn parse(name: &str, body: &str) -> bare_search::engine::Parsed {
    let e = engine(name);
    let url = e.request_url("rust lifetimes", "en");
    e.parse(body, &url, "en")
        .unwrap_or_else(|err| panic!("{name}: {err:?}"))
}

fn assert_sane(name: &str, p: &bare_search::engine::Parsed, min: usize) {
    assert!(
        p.results.len() >= min,
        "{name}: only {} results",
        p.results.len()
    );
    for r in &p.results {
        assert!(r.url.starts_with("http"), "{name}: bad url {}", r.url);
        assert!(!r.title.is_empty(), "{name}: empty title for {}", r.url);
        assert!(
            !r.title.contains('<') && !r.content.contains('<'),
            "{name}: markup leaked: {} / {}",
            r.title,
            r.content
        );
    }
    let mut urls: Vec<_> = p.results.iter().map(|r| &r.url).collect();
    urls.sort();
    urls.dedup();
    assert_eq!(
        urls.len(),
        p.results.len(),
        "{name}: duplicate urls within one engine"
    );
}

#[test]
fn bing_fixture_unwraps_redirects() {
    let p = parse("bing", BING);
    assert_sane("bing", &p, 8);
    assert!(
        p.results.iter().all(|r| !r.url.contains("bing.com/ck/")),
        "tracking redirect left in: {:?}",
        p.results.iter().map(|r| &r.url).collect::<Vec<_>>()
    );
    assert_eq!(p.results[0].title, "Lifetimes - Rust By Example");
    assert!(
        p.results[0].url.starts_with("https://doc.rust-lang.org/"),
        "{}",
        p.results[0].url
    );
    assert!(
        p.results.iter().any(|r| !r.content.is_empty()),
        "no snippets parsed"
    );
}

#[test]
fn duckduckgo_fixture_unwraps_redirects_and_skips_ads() {
    let p = parse("duckduckgo", DDG);
    assert_sane("duckduckgo", &p, 8);
    assert!(
        p.results
            .iter()
            .all(|r| !r.url.contains("duckduckgo.com/l/")),
        "uddg redirect left in"
    );
    assert!(
        p.results.iter().all(|r| !r.url.contains("y.js")),
        "ad result got through"
    );
    assert!(
        p.results.iter().any(|r| r.content.len() > 40),
        "no snippets parsed"
    );
}

#[test]
fn wikipedia_fixture_builds_article_urls_and_suggestion() {
    let p = parse("wikipedia", WIKIPEDIA);
    assert_sane("wikipedia", &p, 8);
    assert_eq!(p.results[0].title, "Rust (programming language)");
    assert_eq!(
        p.results[0].url,
        "https://en.wikipedia.org/wiki/Rust_(programming_language)"
    );
    assert!(
        !p.results[0].content.contains("searchmatch"),
        "snippet markup leaked"
    );
    assert_eq!(p.suggestions, ["rush lifetime"]);
}

#[test]
fn archwiki_fixture() {
    let e = engine("archwiki");
    let p = e
        .parse(ARCHWIKI, &e.request_url("kernel parameters", "en"), "en")
        .unwrap();
    assert_sane("archwiki", &p, 3);
    assert_eq!(p.results[0].title, "Kernel parameters");
    assert_eq!(
        p.results[0].url,
        "https://wiki.archlinux.org/title/Kernel_parameters"
    );
}

#[test]
fn stackexchange_fixture_decodes_entities() {
    let p = parse("stackoverflow", STACKEXCHANGE);
    assert_sane("stackoverflow", &p, 5);
    assert!(
        p.results
            .iter()
            .all(|r| r.url.starts_with("https://stackoverflow.com/"))
    );
    assert!(
        p.results
            .iter()
            .all(|r| !r.title.contains("&#") && !r.title.contains("&amp;")),
        "entity left in a title"
    );
    assert!(
        p.results[0].content.contains("votes") && p.results[0].content.contains("answers"),
        "{}",
        p.results[0].content
    );
}

#[test]
fn github_fixture() {
    let p = parse("github", GITHUB);
    assert_sane("github", &p, 5);
    assert!(p.results[0].url.starts_with("https://github.com/"));
    assert!(
        p.results[0].content.starts_with('★'),
        "{}",
        p.results[0].content
    );
}

#[test]
fn marginalia_fixture() {
    let p = parse("marginalia", MARGINALIA);
    assert_sane("marginalia", &p, 5);
}

#[test]
fn captcha_pages_are_blocked_not_empty() {
    let e = engine("bing");
    let err = e
        .parse(MOJEEK_CAPTCHA, "https://www.bing.com/search?q=x", "en")
        .unwrap_err();
    assert!(matches!(err, ParseError::Blocked(_)), "{err:?}");
    // And a rate-limit page served with 200: still Blocked via its title.
    let ddg = engine("duckduckgo");
    let err = ddg
        .parse(
            "<html><head><title>Unusual traffic from your network</title></head></html>",
            "https://html.duckduckgo.com/html/?q=x",
            "en",
        )
        .unwrap_err();
    assert!(matches!(err, ParseError::Blocked(_)));
}

#[test]
fn duckduckgos_bot_challenge_is_blocked_not_empty() {
    // Regression: the challenge arrives as HTTP 202 with no <title> and no results, which used to
    // read as "no results" and so was never backed off from.
    let e = engine("duckduckgo");
    let err = e
        .parse(
            DDG_ANOMALY,
            &e.request_url("linux kernel parameters", "en"),
            "en",
        )
        .unwrap_err();
    assert!(matches!(err, ParseError::Blocked(_)), "{err:?}");
    // A genuine results page that merely mentions bots must not trip it.
    assert!(
        e.parse(DDG, &e.request_url("rust lifetimes", "en"), "en")
            .is_ok()
    );
}

#[test]
fn json_api_errors_are_classified() {
    let gh = engine("github");
    let limited = r#"{"message":"API rate limit exceeded for 1.2.3.4.","documentation_url":"x"}"#;
    assert!(matches!(
        gh.parse(
            limited,
            "https://api.github.com/search/repositories?q=x",
            "en"
        ),
        Err(ParseError::Blocked(_))
    ));
    assert!(matches!(
        gh.parse("{\"unexpected\": true}", "https://api.github.com/x", "en"),
        Err(ParseError::Malformed(_))
    ));
    assert!(matches!(
        gh.parse("<html>", "https://api.github.com/x", "en"),
        Err(ParseError::Malformed(_))
    ));
}

#[test]
fn an_empty_result_list_is_empty_not_an_error() {
    let p = engine("wikipedia")
        .parse(
            r#"{"query":{"search":[]}}"#,
            "https://en.wikipedia.org/w/api.php",
            "en",
        )
        .unwrap();
    assert!(p.results.is_empty());
}

// ---- the whole pipeline over a fake network ---------------------------------------------------

#[derive(Clone)]
enum Reply {
    Body(u16, &'static str),
    Slow(Duration, &'static str),
    Down,
}

struct FakeNet {
    replies: HashMap<&'static str, Reply>,
    calls: Arc<AtomicUsize>,
}

impl Fetcher for FakeNet {
    fn get(&self, req: &Request) -> Result<Fetched, FetchError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let reply = self
            .replies
            .iter()
            .find(|(host, _)| req.url.contains(*host))
            .map(|(_, r)| r.clone())
            .unwrap_or(Reply::Down);
        match reply {
            Reply::Body(status, body) => Ok(Fetched {
                status,
                body: body.to_string(),
            }),
            Reply::Slow(d, body) => {
                std::thread::sleep(d);
                Ok(Fetched {
                    status: 200,
                    body: body.to_string(),
                })
            }
            Reply::Down => Err(FetchError::Network("connection refused".into())),
        }
    }
}

fn searcher(replies: &[(&'static str, Reply)]) -> (Searcher, Arc<AtomicUsize>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let net = FakeNet {
        replies: replies.iter().cloned().collect(),
        calls: calls.clone(),
    };
    (Searcher::new(builtin::engines(), Box::new(net)), calls)
}

fn all_ok() -> Vec<(&'static str, Reply)> {
    vec![
        ("bing.com", Reply::Body(200, BING)),
        ("duckduckgo.com", Reply::Body(200, DDG)),
        ("wikipedia.org", Reply::Body(200, WIKIPEDIA)),
        ("marginalia.nu", Reply::Body(200, MARGINALIA)),
        ("archlinux.org", Reply::Body(200, ARCHWIKI)),
        ("stackexchange.com", Reply::Body(200, STACKEXCHANGE)),
        ("github.com", Reply::Body(200, GITHUB)),
    ]
}

#[test]
fn a_normal_search_merges_the_general_engines_only() {
    let (s, calls) = searcher(&all_ok());
    let r = s.search("rust lifetimes");
    assert_eq!(
        calls.load(Ordering::SeqCst),
        4,
        "bing, duckduckgo, wikipedia, marginalia; not the IT specialists"
    );
    assert_eq!(r.query, "rust lifetimes");
    assert_eq!((r.answered(), r.asked()), (4, 4));
    assert!(r.items.len() > 20);
    assert!(
        r.items.windows(2).all(|w| w[0].score >= w[1].score),
        "not sorted by score"
    );
    let mut urls: Vec<_> = r.items.iter().map(|i| &i.url).collect();
    let before = urls.len();
    urls.sort();
    urls.dedup();
    assert_eq!(urls.len(), before, "duplicate urls after merge");
    assert!(
        r.items.iter().any(|i| i.engines.len() > 1),
        "no result was found by more than one engine"
    );
    assert_eq!(r.suggestions, ["rush lifetime"]);
}

#[test]
fn bangs_pick_engines_and_categories() {
    let (s, calls) = searcher(&all_ok());
    let r = s.search("!w rust lifetimes");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(r.reports.len(), 1);
    assert_eq!(r.reports[0].engine, "wikipedia");

    calls.store(0, Ordering::SeqCst);
    let r = s.search("!it rust lifetimes");
    assert_eq!(
        calls.load(Ordering::SeqCst),
        3,
        "archwiki, stackoverflow, github"
    );
    assert!(r.items.iter().any(|i| i.url.contains("stackoverflow.com")));
    assert!(r.items.iter().any(|i| i.url.contains("github.com")));
}

#[test]
fn language_prefix_reaches_the_request() {
    struct Spy(Arc<std::sync::Mutex<Vec<String>>>);
    impl Fetcher for Spy {
        fn get(&self, req: &Request) -> Result<Fetched, FetchError> {
            self.0.lock().unwrap().push(req.url.clone());
            Ok(Fetched {
                status: 200,
                body: r#"{"query":{"search":[]}}"#.into(),
            })
        }
    }
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let s = Searcher::new(builtin::engines(), Box::new(Spy(seen.clone())));
    s.search("!w :de rust");
    assert!(
        seen.lock().unwrap()[0].starts_with("https://de.wikipedia.org/"),
        "{:?}",
        seen.lock().unwrap()
    );
}

#[test]
fn an_empty_query_asks_nobody() {
    let (s, calls) = searcher(&all_ok());
    assert!(s.search("   ").items.is_empty());
    assert!(s.search("!w").items.is_empty());
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn a_blocked_engine_is_reported_then_left_alone() {
    let mut net = all_ok();
    net.retain(|(h, _)| *h != "bing.com");
    net.push(("bing.com", Reply::Body(429, BRAVE_429)));
    let (s, calls) = searcher(&net);

    let r = s.search("rust lifetimes");
    let bing = r.reports.iter().find(|x| x.engine == "bing").unwrap();
    assert!(
        matches!(bing.status, Status::Blocked(_)),
        "{:?}",
        bing.status
    );
    assert_eq!((r.answered(), r.asked()), (3, 4));
    assert!(
        !r.items.is_empty(),
        "the other engines still produce a page"
    );

    // Bookkeeping happens on the engine's own thread; give it a moment, then search again.
    std::thread::sleep(Duration::from_millis(100));
    calls.store(0, Ordering::SeqCst);
    let r = s.search("rust lifetimes again");
    let bing = r.reports.iter().find(|x| x.engine == "bing").unwrap();
    assert!(
        matches!(bing.status, Status::Suspended(secs) if secs > 500),
        "{:?}",
        bing.status
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        3,
        "a blocked engine must not be asked again"
    );
}

#[test]
fn a_challenged_engine_is_blocked_and_then_left_alone() {
    let mut net = all_ok();
    net.retain(|(h, _)| *h != "duckduckgo.com");
    net.push(("duckduckgo.com", Reply::Body(202, DDG_ANOMALY)));
    let (s, calls) = searcher(&net);
    let r = s.search("linux kernel parameters");
    let ddg = r.reports.iter().find(|x| x.engine == "duckduckgo").unwrap();
    assert!(
        matches!(&ddg.status, Status::Blocked(m) if m.contains("challenge")),
        "{:?}",
        ddg.status
    );
    std::thread::sleep(Duration::from_millis(100));
    calls.store(0, Ordering::SeqCst);
    let r = s.search("another query");
    assert!(matches!(
        r.reports
            .iter()
            .find(|x| x.engine == "duckduckgo")
            .unwrap()
            .status,
        Status::Suspended(_)
    ));
    assert_eq!(
        calls.load(Ordering::SeqCst),
        3,
        "must not hammer a challenging engine"
    );
}

#[test]
fn a_down_engine_does_not_break_the_page() {
    let mut net = all_ok();
    net.retain(|(h, _)| *h != "duckduckgo.com");
    net.push(("duckduckgo.com", Reply::Down));
    let (s, _) = searcher(&net);
    let r = s.search("rust lifetimes");
    let ddg = r.reports.iter().find(|x| x.engine == "duckduckgo").unwrap();
    assert!(
        matches!(&ddg.status, Status::Failed(m) if m.contains("refused")),
        "{:?}",
        ddg.status
    );
    assert!(!r.items.is_empty());
}

#[test]
fn a_slow_engine_is_not_waited_for() {
    let mut net = all_ok();
    net.retain(|(h, _)| *h != "marginalia.nu");
    net.push((
        "marginalia.nu",
        Reply::Slow(Duration::from_millis(1500), MARGINALIA),
    ));
    let (mut s, _) = searcher(&net);
    s.grace = Duration::from_millis(100);

    let t = Instant::now();
    let r = s.search("rust lifetimes");
    assert!(
        t.elapsed() < Duration::from_millis(900),
        "waited {:?}",
        t.elapsed()
    );
    let slow = r.reports.iter().find(|x| x.engine == "marginalia").unwrap();
    assert_eq!(slow.status, Status::Late);
    assert!(!r.items.is_empty());
    assert!(
        r.items
            .iter()
            .all(|i| !i.engines.contains(&"marginalia".to_string())),
        "late results must not appear"
    );

    // The slow engine isn't penalised for being slow: it answered, it just wasn't waited for.
    std::thread::sleep(Duration::from_millis(1700));
    let r = s.search("rust lifetimes");
    let slow = r.reports.iter().find(|x| x.engine == "marginalia").unwrap();
    assert!(
        !matches!(slow.status, Status::Suspended(_)),
        "{:?}",
        slow.status
    );
}

// ---- SearXNG fallback --------------------------------------------------------------------------

fn with_instance(replies: &[(&'static str, Reply)]) -> (Searcher, Arc<AtomicUsize>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let net = FakeNet {
        replies: replies.iter().cloned().collect(),
        calls: calls.clone(),
    };
    let mut s = Searcher::new(builtin::engines(), Box::new(net));
    s.add_fallback(builtin::searxng_instance("https://sx.example").unwrap());
    (s, calls)
}

#[test]
fn the_fallback_instance_is_not_asked_when_the_builtin_engines_deliver() {
    let mut net = all_ok();
    net.push(("sx.example", Reply::Body(200, SEARXNG)));
    let (s, calls) = with_instance(&net);
    let r = s.search("rust lifetimes");
    assert_eq!(
        calls.load(Ordering::SeqCst),
        4,
        "only the four general engines"
    );
    assert!(r.reports.iter().all(|x| x.engine != "searxng"));
}

#[test]
fn the_fallback_instance_steps_in_when_the_builtin_engines_fail() {
    let net = vec![
        ("bing.com", Reply::Body(429, "")),
        ("duckduckgo.com", Reply::Body(202, DDG_ANOMALY)),
        ("wikipedia.org", Reply::Down),
        ("marginalia.nu", Reply::Down),
        ("sx.example", Reply::Body(200, SEARXNG)),
    ];
    let (s, calls) = with_instance(&net);
    let r = s.search("rust lifetimes");
    assert_eq!(
        calls.load(Ordering::SeqCst),
        5,
        "four failures, then the instance"
    );
    let sx = r
        .reports
        .iter()
        .find(|x| x.engine == "searxng")
        .expect("instance was asked");
    assert_eq!(sx.status, Status::Ok(3));
    assert_eq!(r.items.len(), 3);
    assert_eq!(
        r.items[0].url,
        "https://doc.rust-lang.org/book/ch10-03-lifetime-syntax.html"
    );
    assert_eq!(
        r.items[0].content, "Lifetimes are another kind of generic that we've already been using.",
        "html in snippets is stripped"
    );
    assert_eq!(r.suggestions, ["rust lifetime"]);
}

#[test]
fn a_fallback_instance_with_json_disabled_is_blocked_not_trusted() {
    let net = vec![("sx.example", Reply::Body(403, "<h1>Forbidden</h1>"))];
    let (s, _) = with_instance(&net);
    let r = s.search("rust lifetimes");
    let sx = r.reports.iter().find(|x| x.engine == "searxng").unwrap();
    assert!(matches!(sx.status, Status::Blocked(_)), "{:?}", sx.status);
}

#[test]
fn bang_sx_asks_only_the_instance() {
    let mut net = all_ok();
    net.push(("sx.example", Reply::Body(200, SEARXNG)));
    let (s, calls) = with_instance(&net);
    let r = s.search("!sx rust lifetimes");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(r.reports.len(), 1);
    assert_eq!(r.items.len(), 3);
}

#[test]
fn explicit_bangs_never_trigger_the_fallback() {
    let net = vec![
        ("wikipedia.org", Reply::Down),
        ("sx.example", Reply::Body(200, SEARXNG)),
    ];
    let (s, calls) = with_instance(&net);
    let r = s.search("!w rust");
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "the user asked for Wikipedia, so the instance isn't told"
    );
    assert!(r.items.is_empty());
}
