//! The network edge. Everything else in this crate is pure; tests swap this out for recorded responses.

use crate::engine::Ua;
use std::time::Duration;

pub struct Request {
    pub url: String,
    pub ua: Ua,
    pub lang: String,
    pub timeout: Duration,
}

pub struct Fetched {
    pub status: u16,
    pub body: String,
}

#[derive(Debug, PartialEq, Eq)]
pub enum FetchError {
    Timeout,
    Network(String),
}

pub trait Fetcher: Send + Sync {
    fn get(&self, req: &Request) -> Result<Fetched, FetchError>;
}

const BROWSER_UA: &str = "Mozilla/5.0 (X11; Linux x86_64; rv:140.0) Gecko/20100101 Firefox/140.0";
const API_UA: &str = concat!("Bare/", env!("CARGO_PKG_VERSION"), " (metasearch client)");
const MAX_BODY: u64 = 3 * 1024 * 1024;

pub struct UreqFetcher {
    agent: ureq::Agent,
}

impl UreqFetcher {
    pub fn new() -> Self {
        // Statuses are data, not errors: a 429 body is how we learn an engine is blocking us.
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .build()
            .into();
        Self { agent }
    }
}

impl Default for UreqFetcher {
    fn default() -> Self {
        Self::new()
    }
}

impl Fetcher for UreqFetcher {
    fn get(&self, req: &Request) -> Result<Fetched, FetchError> {
        let (ua, accept) = match req.ua {
            Ua::Browser => (
                BROWSER_UA,
                "text/html,application/xhtml+xml;q=0.9,*/*;q=0.5",
            ),
            Ua::Api => (API_UA, "application/json"),
        };
        let accept_language = format!("{},en;q=0.7", req.lang);
        let mut resp = self
            .agent
            .get(&req.url)
            .config()
            .timeout_global(Some(req.timeout))
            .build()
            .header("User-Agent", ua)
            .header("Accept", accept)
            .header("Accept-Language", &accept_language)
            .call()
            .map_err(map_err)?;
        let status = resp.status().as_u16();
        let body = resp
            .body_mut()
            .with_config()
            .limit(MAX_BODY)
            .read_to_string()
            .map_err(map_err)?;
        Ok(Fetched { status, body })
    }
}

fn map_err(e: ureq::Error) -> FetchError {
    match e {
        ureq::Error::Timeout(_) => FetchError::Timeout,
        other => FetchError::Network(other.to_string()),
    }
}
