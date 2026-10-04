//! The frameless window: no title bar, no header bar. A slim URL bar on top of the page, plus the
//! things a title bar normally provides that a frameless window must supply itself (resize edges,
//! drag-to-move, close/fullscreen keys). Tabs have no strip: a count in the URL bar shows that
//! there are several, and the URL bar's dropdown is the tab switcher.

use crate::{
    dropdown::Dropdown, filters::Filters, findbar::FindBar, log, permissions::PermissionBar,
    tabs::Tab, urlbar::UrlBar, web,
};
use bare_core::{
    Bookmarks, Chrome, Config, History, Paths, Target, WindowState, downloads, input,
    suggest::{self, Kind, Suggestion, TabInfo},
    tabs::{TabId, TabOrder},
};
use bare_search::Searcher;
use gtk4::{self as gtk, gdk, glib, prelude::*};
use std::{
    cell::{Cell, OnceCell, RefCell},
    collections::HashMap,
    path::Path,
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use webkit6::{
    FindOptions, GeolocationPermissionRequest, LoadEvent, NavigationPolicyDecision, NetworkSession,
    PermissionRequest, PolicyDecisionType, ResponsePolicyDecision, WebView, prelude::*,
};

mod bar;
mod extras;
mod frame;
mod home;
mod tabs;

use self::frame::{add_alt_drag_move, add_resize_handles, load_css};
use self::home::build_home;

/// Closed tabs remembered for Ctrl+Shift+T.
const KEEP_CLOSED: usize = 10;
const SUGGESTIONS: usize = 8;
/// How long after start-up Bare checks whether its filter lists need refreshing.
const FILTER_UPDATE_DELAY_SECS: u32 = 5;

/// The new-tab page. It is drawn natively and a tab showing it has no web view, so an empty tab costs
/// no web process (and Bare starts without spawning one).
const HOME: &str = "bare://home";

fn is_home(url: &str) -> bool {
    url == HOME
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

pub struct Browser {
    inner: Rc<Inner>,
    started: Cell<bool>,
}

struct Inner {
    window: gtk::ApplicationWindow,
    /// Created with the first web view: it starts WebKit's network process, which a Bare that only
    /// shows the new-tab page has no use for.
    session: OnceCell<NetworkSession>,
    stack: gtk::Stack,
    bar: UrlBar,
    revealer: gtk::Revealer,
    dropdown: Dropdown,
    progress: gtk::ProgressBar,
    link_label: gtk::Label,
    toast: gtk::Label,
    toast_generation: Cell<u32>,
    config: Config,
    paths: Paths,
    tabs: RefCell<HashMap<TabId, Rc<Tab>>>,
    order: RefCell<TabOrder>,
    next_id: Cell<TabId>,
    closed: RefCell<Vec<String>>,
    bookmarks: RefCell<Bookmarks>,
    history: Option<History>,
    filters: Filters,
    /// The user has edited the bar since focusing it (so the dropdown filters by what they typed).
    bar_dirty: Cell<bool>,
    find: FindBar,
    permission: PermissionBar,
    /// A page is waiting to hear whether it may use the location: `(tab, request)`.
    pending_permission: RefCell<Option<(TabId, PermissionRequest)>>,
}

impl Browser {
    pub fn new(app: &gtk::Application, paths: &Paths, config: &Config) -> Rc<Self> {
        load_css();

        let state = WindowState::load(&paths.window_file());
        let window = gtk::ApplicationWindow::builder()
            .application(app)
            .title("Bare")
            .default_width(state.width)
            .default_height(state.height)
            .decorated(false) // the whole point: no title bar
            .css_classes(["bare"])
            .build();
        if state.maximized {
            window.maximize();
        }

        let searcher = Arc::new(Searcher::builtin_with_instance(
            config.search.instance.as_deref(),
        ));
        web::register_scheme(config.home_hint, searcher);
        let filters = Filters::start(paths, config.filters);
        let bar = UrlBar::new();
        let dropdown = Dropdown::new();

        let auto_hide = config.chrome == Chrome::Hidden;
        let revealer = gtk::Revealer::builder()
            .transition_type(gtk::RevealerTransitionType::SlideDown)
            .transition_duration(120)
            .reveal_child(!auto_hide)
            .child(&bar.widget)
            .build();

        let progress = gtk::ProgressBar::builder()
            .css_classes(["bare-progress"])
            .valign(gtk::Align::Start)
            .can_target(false)
            .build();
        let link_label = gtk::Label::builder()
            .css_classes(["bare-status"])
            .halign(gtk::Align::Start)
            .valign(gtk::Align::End)
            .max_width_chars(90)
            .ellipsize(gtk::pango::EllipsizeMode::Middle)
            .can_target(false)
            .visible(false)
            .build();
        let toast = gtk::Label::builder()
            .css_classes(["bare-toast"])
            .halign(gtk::Align::Center)
            .valign(gtk::Align::End)
            .can_target(false)
            .visible(false)
            .build();

        let stack = gtk::Stack::builder()
            .transition_type(gtk::StackTransitionType::None)
            .hexpand(true)
            .vexpand(true)
            .build();
        stack.add_named(&build_home(config), Some("home"));
        let page = gtk::Overlay::builder().child(&stack).vexpand(true).build();
        let find = FindBar::new();
        let permission = PermissionBar::new();
        let overlays: [&gtk::Widget; 4] = [
            progress.upcast_ref(),
            link_label.upcast_ref(),
            toast.upcast_ref(),
            find.widget.upcast_ref(),
        ];
        for w in overlays {
            page.add_overlay(w);
            page.set_measure_overlay(w, false);
        }

        let column = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .build();
        column.append(&revealer);
        column.append(&permission.widget);
        column.append(&page);
        if config.border {
            column.add_css_class("bare-frame");
        }

        let root = gtk::Overlay::builder().child(&column).build();
        root.add_overlay(&dropdown.widget);
        root.set_measure_overlay(&dropdown.widget, false);
        let handles = add_resize_handles(&root);
        add_alt_drag_move(&root);
        window.set_child(Some(&root));

        let history = if config.history {
            History::open(&paths.history_file())
                .map_err(|e| eprintln!("bare: history disabled: {e}"))
                .ok()
        } else {
            None
        };

        let inner = Rc::new(Inner {
            window: window.clone(),
            session: OnceCell::new(),
            stack,
            bar,
            revealer,
            dropdown,
            progress,
            link_label,
            toast,
            toast_generation: Cell::new(0),
            config: config.clone(),
            paths: paths.clone(),
            tabs: RefCell::default(),
            order: RefCell::default(),
            next_id: Cell::new(1),
            closed: RefCell::default(),
            bookmarks: RefCell::new(Bookmarks::load(&paths.bookmarks_file())),
            history,
            filters,
            bar_dirty: Cell::new(false),
            find,
            permission,
            pending_permission: RefCell::new(None),
        });

        inner.wire_bar();
        inner.wire_window(&handles, &page);
        inner.wire_shortcuts();
        inner.wire_find();
        inner.wire_permissions();
        inner.start_discard_timer();
        if config.filter_updates {
            // Once the first page has had the start-up to itself.
            let weak = Rc::downgrade(&inner);
            glib::timeout_add_seconds_local_once(FILTER_UPDATE_DELAY_SECS, move || {
                if let Some(inner) = weak.upgrade() {
                    inner.filters.maybe_update(&inner.paths);
                }
            });
        }

        Rc::new(Self {
            inner,
            started: Cell::new(false),
        })
    }

    /// Present the window. First time: open the requested page, or the home page. Later calls
    /// (`bare some.url` while it runs): open the page in a new tab.
    pub fn show(&self, start: Vec<Target>) {
        let inner = &self.inner;
        let urls: Vec<String> = start.into_iter().map(target_url).collect();
        if !self.started.get() {
            if !urls.is_empty() {
                // A cached filter list is ready in milliseconds; don't let the first page beat it.
                inner.filters.wait_ready(Duration::from_millis(250));
            }
            // The first page is the tab you see; any others open behind it, in order.
            let mut previous = inner.open_tab(urls.first().map(String::as_str), None, true);
            for url in urls.iter().skip(1) {
                previous = inner.open_tab(Some(url), Some(previous), false);
            }
            if urls.is_empty() && inner.config.chrome == Chrome::Bar {
                inner.bar.focus();
            }
            self.started.set(true);
        } else {
            // Handed to a running Bare: every page opens as a new tab, the first one brought forward.
            let mut previous = inner.active_id();
            for (i, url) in urls.iter().enumerate() {
                previous = Some(inner.open_tab(Some(url), previous, i == 0));
            }
        }
        inner.window.present();
        log!(
            "startup: window presented {} ms after launch",
            crate::STARTED.elapsed().as_millis()
        );
    }
}

impl Inner {
    // ---- lookups ----------------------------------------------------------------------------

    fn tab(&self, id: TabId) -> Option<Rc<Tab>> {
        self.tabs.borrow().get(&id).cloned()
    }

    fn active_id(&self) -> Option<TabId> {
        self.order.borrow().active()
    }

    fn active_tab(&self) -> Option<Rc<Tab>> {
        self.active_id().and_then(|id| self.tab(id))
    }

    fn active_view(&self) -> Option<WebView> {
        self.active_tab().and_then(|t| t.view.borrow().clone())
    }

    /// The network session, created (and its downloads wired up) the first time a page is needed.
    fn session(self: &Rc<Self>) -> &NetworkSession {
        self.session.get_or_init(|| {
            let session = web::new_session(&self.paths);
            self.wire_downloads(&session);
            session
        })
    }

    // ---- chrome -----------------------------------------------------------------------------

    /// Make the URL bar, title, progress line and tab count show the active tab.
    fn sync_chrome(&self) {
        let Some(tab) = self.active_tab() else { return };
        self.bar.set_url(&tab.url.borrow());
        let title = tab.title.borrow();
        self.window
            .set_title(Some(if title.is_empty() { "Bare" } else { &title }));
        self.bar.set_tab_count(self.order.borrow().len());
        self.link_label.set_visible(false);
        self.sync_progress(&tab);
    }

    fn sync_progress(&self, tab: &Tab) {
        self.progress.set_fraction(tab.progress.get());
        if tab.loading.get() {
            self.progress.add_css_class("loading");
        } else {
            self.progress.remove_css_class("loading");
        }
    }

    fn show_toast(self: &Rc<Self>, msg: &str) {
        self.toast.set_text(msg);
        self.toast.set_visible(true);
        let generation = self.toast_generation.get() + 1;
        self.toast_generation.set(generation);
        let weak = Rc::downgrade(self);
        glib::timeout_add_local_once(Duration::from_millis(1600), move || {
            if let Some(s) = weak.upgrade()
                && s.toast_generation.get() == generation
            {
                s.toast.set_visible(false);
            }
        });
    }

    fn focus_bar(&self) {
        self.revealer.set_reveal_child(true);
        self.bar.focus();
    }
}

fn target_url(target: Target) -> String {
    match target {
        Target::Url(u) => u,
        Target::Search(q) => input::search_url(&q),
    }
}

fn resolve_url(text: &str) -> Option<String> {
    input::resolve(text).map(target_url)
}

/// Pages worth remembering and bookmarking: real sites, files and searches; not the home page.
fn recordable(url: &str) -> bool {
    url.starts_with("http://")
        || url.starts_with("https://")
        || url.starts_with("file://")
        || url.starts_with("bare://search")
}

/// Addresses a "new window" link may open as a tab.
fn openable(uri: &str) -> bool {
    uri.starts_with("http://")
        || uri.starts_with("https://")
        || uri.starts_with("file://")
        || uri.starts_with("bare://")
}
