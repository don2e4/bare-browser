//! One tab: a web view that can be unloaded. A tab that has sat in the background too long is
//! *discarded*: its view is destroyed (the memory goes back to the system) and only what is needed
//! to bring it back is kept: the address, the title, and WebKit's session state (history and
//! scroll position).

use bare_core::tabs::TabId;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Instant,
};
use webkit6::{WebView, WebViewSessionState};

pub struct Tab {
    pub id: TabId,
    /// `None` while discarded.
    pub view: RefCell<Option<WebView>>,
    /// Set while the current page is a load-error page. One per view, so replaced with the view.
    pub failed: RefCell<Rc<Cell<bool>>>,
    pub url: RefCell<String>,
    pub title: RefCell<String>,
    pub session: RefCell<Option<WebViewSessionState>>,
    pub last_active: Cell<Instant>,
    pub progress: Cell<f64>,
    pub loading: Cell<bool>,
    /// Shares its web process with another tab (opened by or via `window.open`), so ending the
    /// process would take that tab down too.
    pub shares_process: Cell<bool>,
}

impl Tab {
    pub fn new(id: TabId, url: &str) -> Self {
        Self {
            id,
            view: RefCell::new(None),
            failed: RefCell::new(Rc::new(Cell::new(false))),
            url: RefCell::new(url.to_string()),
            title: RefCell::new(String::new()),
            session: RefCell::new(None),
            last_active: Cell::new(Instant::now()),
            progress: Cell::new(0.0),
            loading: Cell::new(false),
            shares_process: Cell::new(false),
        }
    }
}
