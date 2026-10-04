//! Bare Search: a metasearch pipeline ported from SearXNG, run in-process. No server, no port.
//!
//! query syntax → pick engines → ask them all at once → parse → back off the failing ones →
//! dedupe and score. Everything but the HTTP call (`fetch`) is pure and tested against recorded
//! responses.

pub mod builtin;
pub mod engine;
pub mod fetch;
pub mod merge;
pub mod query;
pub mod render;
pub mod search;
pub mod suspend;

pub use merge::Item;
pub use search::{EngineReport, Response, Searcher, Status};
