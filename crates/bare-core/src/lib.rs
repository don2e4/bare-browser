//! GUI-free logic for Bare. Everything here is pure (or touches only the filesystem)
//! so it can be tested without a display or WebKit.

pub mod bookmarks;
pub mod config;
pub mod display;
pub mod downloads;
pub mod filters;
pub mod history;
pub mod input;
pub mod pages;
pub mod paths;
pub mod state;
pub mod suggest;
pub mod tabs;

pub use bookmarks::Bookmarks;
pub use config::{Chrome, Config};
pub use history::History;
pub use input::{Target, resolve};
pub use paths::Paths;
pub use state::WindowState;
