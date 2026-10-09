//! The frameless window: no title bar, no header bar. A slim URL bar on top of the page, plus the
//! things a title bar normally provides that a frameless window must supply itself (resize edges,
//! drag-to-move, close/fullscreen keys). Tabs have no strip: a sidebar on the left draws them as a
//! tree (F1 hides it), and the URL bar's dropdown is also a tab switcher.

use crate::{
    dropdown::{Dropdown, pretty_url},
    filters::Filters,
    findbar::FindBar,
    log,
    permissions::PermissionBar,
    sidebar::{self, Sidebar},
    tabs::Tab,
    urlbar::UrlBar,
    web,
};
use bare_core::{
    Bookmarks, Chrome, Config, History, Paths, Target, WindowState, downloads, input,
    suggest::{self, Kind, Suggestion, TabInfo},
    tabs::{TabId, TabTree},
};
use gtk4::{self as gtk, gdk, glib, prelude::*};
use std::{
    cell::{Cell, OnceCell, RefCell},
    collections::{HashMap, VecDeque},
    path::Path,
    rc::Rc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use webkit6::{
    FindOptions, GeolocationPermissionRequest, LoadEvent, NavigationPolicyDecision, NavigationType,
    NetworkSession, PermissionRequest, PolicyDecisionType, ResponsePolicyDecision, WebView,
    prelude::*,
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
/// How long after the window first appears it gets its icon (see `main`).
const ICON_DELAY: Duration = Duration::from_millis(1000);
/// How long after start-up an old history starts being indexed (see `index_history`).
const HISTORY_INDEX_DELAY_SECS: u32 = 2;
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
    sidebar: Sidebar,
    /// Holds the sidebar and the page side by side; its handle resizes the sidebar.
    paned: gtk::Paned,
    /// Whether the user wants the sidebar (F1; with `chrome = "hidden"` it starts hidden), and how
    /// wide; saved with the window size.
    sidebar_wanted: Cell<bool>,
    sidebar_width: Cell<i32>,
    revealer: gtk::Revealer,
    dropdown: Dropdown,
    progress: gtk::ProgressBar,
    link_label: gtk::Label,
    toast: gtk::Label,
    toast_generation: Cell<u32>,
    config: Config,
    paths: Paths,
    tabs: RefCell<HashMap<TabId, Rc<Tab>>>,
    order: RefCell<TabTree>,
    next_id: Cell<TabId>,
    /// Recently closed tabs, newest last: the address and the tab it was under.
    closed: RefCell<VecDeque<(String, Option<TabId>)>>,
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
        log!(
            "startup: GTK ready {} ms after launch",
            crate::STARTED.elapsed().as_millis()
        );
        load_css();
        // Unname the default icon `main` named, so showing the window doesn't wait for the icon theme.
        // SAFETY: GTK copies the name; NULL means none (the safe binding takes only a `&str`).
        unsafe { gtk::ffi::gtk_window_set_default_icon_name(std::ptr::null()) };

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

        let sidebar = Sidebar::new();
        let sidebar_shown = state.sidebar && config.chrome == Chrome::Bar;
        sidebar.widget.set_visible(sidebar_shown);
        let paned = gtk::Paned::builder()
            .orientation(gtk::Orientation::Horizontal)
            .css_classes(["bare-paned"])
            .start_child(&sidebar.widget)
            .end_child(&page)
            .resize_start_child(false)
            .shrink_start_child(false)
            .shrink_end_child(false)
            .position(state.sidebar_width)
            .vexpand(true)
            .build();

        let column = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .build();
        column.append(&revealer);
        column.append(&permission.widget);
        column.append(&paned);
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
            sidebar,
            paned,
            sidebar_wanted: Cell::new(sidebar_shown),
            sidebar_width: Cell::new(state.sidebar_width),
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
        inner.wire_sidebar();
        inner.wire_window(&handles, &page);
        inner.wire_shortcuts();
        inner.wire_find();
        inner.wire_permissions();
        inner.start_discard_timer();
        inner.index_history();
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
            inner.open_tab(urls.first().map(String::as_str), None, true);
            for url in urls.iter().skip(1) {
                inner.open_tab(Some(url), None, false);
            }
            if urls.is_empty() && inner.config.chrome == Chrome::Bar {
                inner.bar.focus();
            }
        } else {
            // Handed to a running Bare: every page opens as a new tab, the first one brought forward.
            for (i, url) in urls.iter().enumerate() {
                inner.open_tab(Some(url), None, i == 0);
            }
        }
        inner.window.present();
        if !self.started.replace(true) {
            // By now the icon theme has loaded in the background, so this costs a millisecond or two.
            let weak = inner.window.downgrade();
            glib::timeout_add_local_once(ICON_DELAY, move || {
                if let Some(w) = weak.upgrade() {
                    w.set_icon_name(Some(crate::APP_ID));
                }
            });
        }
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

    /// The network session, created (and its downloads wired up, and the `bare://` pages registered)
    /// the first time a page is needed.
    fn session(self: &Rc<Self>) -> &NetworkSession {
        self.session.get_or_init(|| {
            web::register_scheme(self.config.home_hint, self.config.search.instance.clone());
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
        self.sync_badge();
        self.link_label.set_visible(false);
        self.sync_progress(&tab);
    }

    /// A history from before 0.2.0 too big to index while opening (seconds, for a big one) is indexed
    /// on another thread, shortly after start-up; searches use the index once it is done.
    fn index_history(self: &Rc<Self>) {
        if !self.history.as_ref().is_some_and(History::needs_index) {
            return;
        }
        let weak = Rc::downgrade(self);
        let path = self.paths.history_file();
        glib::timeout_add_seconds_local_once(HISTORY_INDEX_DELAY_SECS, move || {
            let (tx, rx) = async_channel::bounded(1);
            std::thread::spawn(move || {
                let started = Instant::now();
                let _ = tx.send_blocking(History::build_index(&path).map(|()| started.elapsed()));
            });
            glib::spawn_future_local(async move {
                let Ok(result) = rx.recv().await else { return };
                match result {
                    Ok(took) => {
                        log!("history: index built in {} ms", took.as_millis());
                        if let Some(h) = weak.upgrade().as_ref().and_then(|s| s.history.as_ref()) {
                            h.use_index();
                        }
                    }
                    Err(e) => eprintln!("bare: history: cannot build the search index: {e}"),
                }
            });
        });
    }

    /// The tab count in the URL bar, shown only while the sidebar (which lists the tabs) is hidden.
    fn sync_badge(&self) {
        self.bar
            .set_tab_count(self.order.borrow().len(), !self.sidebar.widget.is_visible());
    }

    /// Redraw the sidebar after the tree changed shape (a tab opened, closed, folded).
    fn sync_tree(&self) {
        let rows = self.order.borrow().rows();
        self.sidebar
            .set(&rows, |id| self.tab_label(id), self.active_id());
        log!(
            "tree: {}",
            rows.iter()
                .map(|r| match r.hidden {
                    0 => format!("{}{}", r.prefix, r.id),
                    n => format!("{}{} +{n}", r.prefix, r.id),
                })
                .collect::<Vec<_>>()
                .join(" | ")
        );
    }

    /// Update one tab's line in the sidebar (its title, or whether it is unloaded).
    fn sync_row(&self, tab: &Tab) {
        self.sidebar.update(tab.id, &label(tab));
    }

    /// What the sidebar shows for tab `id`.
    fn tab_label(&self, id: TabId) -> sidebar::Label {
        match self.tab(id) {
            Some(tab) => label(&tab),
            None => sidebar::Label {
                title: String::new(),
                url: String::new(),
                discarded: false,
            },
        }
    }

    /// F1: show or hide the sidebar. Remembered with the window size.
    fn toggle_sidebar(&self) {
        let show = !self.sidebar.widget.is_visible();
        if !show {
            self.sidebar_width.set(self.paned.position());
        }
        self.sidebar.widget.set_visible(show);
        if show {
            self.paned.set_position(self.sidebar_width.get());
            self.sidebar.reveal_active();
        }
        self.sidebar_wanted.set(show);
        self.sync_badge();
        log!("sidebar: {}", if show { "shown" } else { "hidden" });
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

/// A tab's line in the sidebar: its title, else its address, else "New tab".
fn label(tab: &Tab) -> sidebar::Label {
    let url = tab.url.borrow();
    let title = tab.title.borrow();
    let home = is_home(&url);
    sidebar::Label {
        title: if !title.is_empty() {
            title.clone()
        } else if home {
            "New tab".to_string()
        } else {
            pretty_url(&url)
        },
        url: if home { String::new() } else { url.clone() },
        discarded: tab.view.borrow().is_none() && !home,
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
