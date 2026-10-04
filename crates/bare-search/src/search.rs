//! The orchestrator: pick engines, ask them all at once, parse, back off the ones that fail, merge.
//!
//! Slow engines don't hold the page hostage: once one engine has produced results, the others get
//! `grace` more time and then the search returns without them. Their threads run on, finish within
//! their own timeout, and record success or failure for next time.

use crate::engine::{Engine, ParseError, RawResult};
use crate::fetch::{FetchError, Fetcher, Request, UreqFetcher};
use crate::merge::{EngineResults, Item, merge};
use crate::query;
use crate::suspend::{Failure, Suspend};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// Answered with this many results.
    Ok(usize),
    /// Answered, with nothing for this query.
    Empty,
    /// Refusing us (captcha, rate limit).
    Blocked(String),
    /// Timed out, network error, or an unexpected response.
    Failed(String),
    /// Not asked: it failed recently. Seconds until it is asked again.
    Suspended(u64),
    /// Still working when the page was returned; not waited for.
    Late,
}

#[derive(Debug, Clone)]
pub struct EngineReport {
    pub engine: String,
    pub status: Status,
    pub elapsed: Duration,
}

#[derive(Debug, Clone, Default)]
pub struct Response {
    pub query: String,
    pub items: Vec<Item>,
    pub reports: Vec<EngineReport>,
    pub suggestions: Vec<String>,
    pub elapsed: Duration,
}

impl Response {
    /// Engines that answered (even with nothing).
    pub fn answered(&self) -> usize {
        self.reports
            .iter()
            .filter(|r| matches!(r.status, Status::Ok(_) | Status::Empty))
            .count()
    }
    /// Engines that were part of this search.
    pub fn asked(&self) -> usize {
        self.reports.len()
    }
}

pub struct Searcher {
    engines: Arc<Vec<Engine>>,
    fetcher: Arc<dyn Fetcher>,
    suspend: Arc<Suspend>,
    pub default_lang: String,
    /// How long to keep waiting for the rest once one engine has produced results.
    pub grace: Duration,
}

#[derive(Default)]
struct Wave {
    reports: Vec<EngineReport>,
    lists: Vec<EngineResults>,
    suggestions: Vec<String>,
}

/// Ask the fallback engines when the normal ones produced fewer results than this.
const FALLBACK_BELOW: usize = 5;

struct Outcome {
    status: Status,
    results: Vec<RawResult>,
    suggestions: Vec<String>,
    elapsed: Duration,
}

impl Outcome {
    fn new(status: Status) -> Self {
        Self {
            status,
            results: Vec::new(),
            suggestions: Vec::new(),
            elapsed: Duration::ZERO,
        }
    }
    fn failed(why: &str) -> Self {
        Self::new(Status::Failed(why.to_string()))
    }
}

impl Searcher {
    /// The built-in engines over real HTTP.
    pub fn builtin() -> Self {
        Self::new(crate::builtin::engines(), Box::new(UreqFetcher::new()))
    }

    pub fn new(engines: Vec<Engine>, fetcher: Box<dyn Fetcher>) -> Self {
        Self {
            engines: Arc::new(engines),
            fetcher: Arc::from(fetcher),
            suspend: Arc::new(Suspend::default()),
            default_lang: "en".into(),
            grace: Duration::from_millis(1200),
        }
    }

    /// The built-in engines, plus a SearXNG instance as a fallback if one is configured. A bad
    /// instance address is reported on stderr and ignored.
    pub fn builtin_with_instance(instance: Option<&str>) -> Self {
        let mut s = Self::builtin();
        if let Some(base) = instance.filter(|b| !b.trim().is_empty()) {
            match crate::builtin::searxng_instance(base) {
                Ok(engine) => s.add_fallback(engine),
                Err(e) => eprintln!("bare: ignoring {e}"),
            }
        }
        s
    }

    pub fn engines(&self) -> &[Engine] {
        &self.engines
    }

    /// Engines for a query: `!name`/`!shortcut`/`!category` selectors if given, else the general ones.
    pub fn select(&self, selectors: &[String]) -> Vec<&Engine> {
        self.select_indices(selectors)
            .into_iter()
            .map(|i| &self.engines[i])
            .collect()
    }

    fn select_indices(&self, selectors: &[String]) -> Vec<usize> {
        self.engines
            .iter()
            .enumerate()
            .filter(|(_, e)| {
                if selectors.is_empty() {
                    e.def.categories.iter().any(|c| c == "general")
                } else {
                    selectors.iter().any(|s| {
                        *s == e.def.name || *s == e.def.shortcut || e.def.categories.contains(s)
                    })
                }
            })
            .map(|(i, _)| i)
            .collect()
    }

    /// Also ask these engines (category `fallback`) when the normal ones come up short.
    pub fn add_fallback(&mut self, engine: Engine) {
        let engines = Arc::make_mut(&mut self.engines);
        engines.push(engine);
    }

    pub fn search(&self, raw: &str) -> Response {
        let started = Instant::now();
        let parsed = query::parse(raw);
        let mut response = Response {
            query: parsed.text.clone(),
            ..Response::default()
        };
        if parsed.text.is_empty() {
            return response;
        }
        let lang = parsed
            .lang
            .clone()
            .unwrap_or_else(|| self.default_lang.clone());

        let mut wave = self.run_wave(&self.select_indices(&parsed.selectors), &parsed.text, &lang);
        let mut items = merge(wave.lists.clone());

        // Not enough from the normal engines (blocked, offline, odd query): ask the fallbacks, unless
        // the user chose engines explicitly.
        if parsed.selectors.is_empty() && items.len() < FALLBACK_BELOW {
            let fallbacks = self.fallback_indices();
            if !fallbacks.is_empty() {
                let second = self.run_wave(&fallbacks, &parsed.text, &lang);
                wave.reports.extend(second.reports);
                wave.lists.extend(second.lists);
                wave.suggestions.extend(second.suggestions);
                items = merge(wave.lists.clone());
            }
        }

        response.items = items;
        response.reports = wave.reports;
        response.suggestions = wave.suggestions;
        response.suggestions.dedup();
        response.elapsed = started.elapsed();
        response
    }

    fn fallback_indices(&self) -> Vec<usize> {
        self.engines
            .iter()
            .enumerate()
            .filter(|(_, e)| e.def.categories.iter().any(|c| c == "fallback"))
            .map(|(i, _)| i)
            .collect()
    }

    /// Ask these engines at once. Suspended ones are reported, not asked.
    fn run_wave(&self, indices: &[usize], text: &str, lang: &str) -> Wave {
        let started = Instant::now();
        let mut wave = Wave::default();
        let mut runnable: Vec<usize> = Vec::new();
        for &idx in indices {
            let name = self.engines[idx].name();
            match self.suspend.remaining(name, Instant::now()) {
                Some(left) => wave.reports.push(EngineReport {
                    engine: name.into(),
                    status: Status::Suspended(left.as_secs() + 1),
                    elapsed: Duration::ZERO,
                }),
                None => runnable.push(idx),
            }
        }

        let (tx, rx) = mpsc::channel::<(usize, Outcome)>();
        for (slot, &idx) in runnable.iter().enumerate() {
            let (engines, fetcher, suspend, tx) = (
                self.engines.clone(),
                self.fetcher.clone(),
                self.suspend.clone(),
                tx.clone(),
            );
            let (text, lang) = (text.to_string(), lang.to_string());
            std::thread::spawn(move || {
                let engine = &engines[idx];
                let outcome = ask(fetcher.as_ref(), engine, &text, &lang);
                record(&suspend, engine.name(), &outcome.status);
                let _ = tx.send((slot, outcome));
            });
        }
        drop(tx);

        let longest = runnable
            .iter()
            .map(|&i| self.engines[i].timeout())
            .max()
            .unwrap_or_default();
        let hard = started + longest + Duration::from_millis(300);
        let mut settle: Option<Instant> = None;
        let mut got: Vec<Option<Outcome>> = runnable.iter().map(|_| None).collect();
        let mut pending = runnable.len();
        while pending > 0 {
            let limit = settle.map_or(hard, |s| s.min(hard));
            match rx.recv_timeout(limit.saturating_duration_since(Instant::now())) {
                Ok((slot, outcome)) => {
                    if matches!(outcome.status, Status::Ok(_)) && settle.is_none() {
                        settle = Some(Instant::now() + self.grace);
                    }
                    got[slot] = Some(outcome);
                    pending -= 1;
                }
                Err(_) => break, // out of time, or every sender is gone
            }
        }

        for (&idx, outcome) in runnable.iter().zip(got) {
            let engine = &self.engines[idx];
            let outcome = outcome.unwrap_or_else(|| Outcome::new(Status::Late));
            wave.suggestions.extend(outcome.suggestions);
            if !outcome.results.is_empty() {
                wave.lists.push(EngineResults {
                    engine: engine.name().into(),
                    weight: engine.def.weight,
                    results: outcome.results,
                });
            }
            wave.reports.push(EngineReport {
                engine: engine.name().into(),
                status: outcome.status,
                elapsed: outcome.elapsed,
            });
        }
        wave
    }

    /// Ask one engine directly, ignoring suspension and recording nothing: for health probes.
    pub fn probe(&self, engine: &Engine, text: &str) -> EngineReport {
        let outcome = ask(self.fetcher.as_ref(), engine, text, &self.default_lang);
        EngineReport {
            engine: engine.name().into(),
            status: outcome.status,
            elapsed: outcome.elapsed,
        }
    }
}

fn record(suspend: &Suspend, engine: &str, status: &Status) {
    match status {
        Status::Ok(_) | Status::Empty => suspend.record_ok(engine),
        Status::Blocked(_) => {
            suspend.record_failure(engine, Failure::Blocked, Instant::now());
        }
        Status::Failed(_) => {
            suspend.record_failure(engine, Failure::Broken, Instant::now());
        }
        Status::Suspended(_) | Status::Late => {}
    }
}

fn ask(fetcher: &dyn Fetcher, engine: &Engine, text: &str, lang: &str) -> Outcome {
    let begun = Instant::now();
    let url = engine.request_url(text, lang);
    let request = Request {
        url: url.clone(),
        ua: engine.def.ua,
        lang: lang.to_string(),
        timeout: engine.timeout(),
    };
    let mut outcome = match fetcher.get(&request) {
        Err(FetchError::Timeout) => Outcome::failed("timed out"),
        Err(FetchError::Network(e)) => Outcome::failed(&e),
        Ok(f) if f.status == 403 || f.status == 429 => {
            Outcome::new(Status::Blocked(format!("HTTP {}", f.status)))
        }
        Ok(f) if f.status >= 400 => Outcome::failed(&format!("HTTP {}", f.status)),
        Ok(f) => match engine.parse(&f.body, &url, lang) {
            Ok(parsed) if parsed.results.is_empty() => Outcome {
                suggestions: parsed.suggestions,
                ..Outcome::new(Status::Empty)
            },
            Ok(parsed) => Outcome {
                status: Status::Ok(parsed.results.len()),
                results: parsed.results,
                suggestions: parsed.suggestions,
                elapsed: Duration::ZERO,
            },
            Err(ParseError::Blocked(why)) => Outcome::new(Status::Blocked(why)),
            Err(ParseError::Malformed(why)) => Outcome::failed(&why),
        },
    };
    outcome.elapsed = begun.elapsed();
    outcome
}
