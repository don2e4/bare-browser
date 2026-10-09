//! Tab lifecycle: opening, switching, closing, discarding idle tabs and restoring them.

use super::*;

impl Inner {
    // ---- tabs -------------------------------------------------------------------------------

    /// Open a tab on `url` (the home page if `None`): under `parent` (the tab whose link opened it),
    /// or as a new top-level tab.
    pub(super) fn open_tab(
        self: &Rc<Self>,
        url: Option<&str>,
        parent: Option<TabId>,
        foreground: bool,
    ) -> TabId {
        let url = url.filter(|u| !is_home(u)).unwrap_or(HOME);
        let id = self.next_id.get();
        self.next_id.set(id + 1);
        let tab = Rc::new(Tab::new(id, url));
        self.tabs.borrow_mut().insert(id, tab.clone());
        self.order.borrow_mut().open(id, parent);
        if !is_home(url) {
            self.create_view(&tab, None).load_uri(url);
        }
        log!("open tab {id} {url}");
        self.sync_tree();
        if foreground {
            self.switch_to(id);
        } else {
            self.sync_chrome();
        }
        id
    }

    /// A page called `window.open` after a click: its view must share the opener's process.
    pub(super) fn open_related(self: &Rc<Self>, opener_view: &WebView, opener: TabId) -> WebView {
        let id = self.next_id.get();
        self.next_id.set(id + 1);
        let tab = Rc::new(Tab::new(id, ""));
        tab.shares_process.set(true);
        if let Some(opener_tab) = self.tab(opener) {
            opener_tab.shares_process.set(true);
        }
        self.tabs.borrow_mut().insert(id, tab.clone());
        self.order.borrow_mut().open(id, Some(opener));
        let view = self.create_view(&tab, Some(opener_view));
        self.sync_tree();
        self.switch_to(id);
        view
    }

    /// Build a tab's web view, wire it to the chrome, and put it in the stack.
    pub(super) fn create_view(
        self: &Rc<Self>,
        tab: &Rc<Tab>,
        related: Option<&WebView>,
    ) -> WebView {
        let made = web::new_view(self.session(), related, self.filters.manager());
        *tab.failed.borrow_mut() = made.failed.clone();
        let view = made.view;
        let id = tab.id;
        self.stack.add_named(&view, Some(&id.to_string()));
        *tab.view.borrow_mut() = Some(view.clone());

        view.connect_uri_notify(self.on_view(id, |s, t, w| {
            // A view that hasn't begun loading reports no URI; keep the address the tab was opened with.
            let Some(uri) = w.uri().map(|u| u.to_string()).filter(|u| !u.is_empty()) else {
                return;
            };
            *t.url.borrow_mut() = uri.clone();
            if s.active_id() == Some(t.id) {
                s.bar.set_url(&uri);
            }
            if t.title.borrow().is_empty() {
                s.sync_row(t);
            }
        }));
        view.connect_title_notify(self.on_view(id, |s, t, w| {
            let title = w.title().map(|u| u.to_string()).unwrap_or_default();
            *t.title.borrow_mut() = title.clone();
            s.sync_row(t);
            if s.active_id() == Some(t.id) {
                s.window
                    .set_title(Some(if title.is_empty() { "Bare" } else { &title }));
            }
            if !t.failed.borrow().get() {
                s.remember_title(&t.url.borrow(), &title);
            }
        }));
        view.connect_estimated_load_progress_notify(self.on_view(id, |s, t, w| {
            t.progress.set(w.estimated_load_progress());
            if s.active_id() == Some(t.id) {
                s.progress.set_fraction(t.progress.get());
            }
        }));
        let weak = Rc::downgrade(self);
        view.connect_load_changed(move |w, event| {
            let Some(s) = weak.upgrade() else { return };
            let Some(t) = s.tab(id) else { return };
            match event {
                LoadEvent::Started => {
                    t.loading.set(true);
                    t.progress.set(0.0);
                    s.withdraw_permission(id);
                }
                LoadEvent::Committed => {
                    if !t.failed.borrow().get() {
                        s.remember_visit(&w.uri().map(|u| u.to_string()).unwrap_or_default());
                    }
                }
                LoadEvent::Finished => {
                    t.loading.set(false);
                    let title = w.title().map(|u| u.to_string()).unwrap_or_default();
                    if !t.failed.borrow().get() {
                        s.remember_title(&t.url.borrow(), &title);
                    }
                }
                _ => {}
            }
            if s.active_id() == Some(id) {
                s.sync_progress(&t);
            }
        });
        let weak = Rc::downgrade(self);
        view.connect_mouse_target_changed(move |_, hit, _| {
            let Some(s) = weak.upgrade() else { return };
            if s.active_id() != Some(id) {
                return;
            }
            match hit.link_uri().filter(|u| !u.is_empty()) {
                Some(uri) => {
                    s.link_label.set_text(&uri);
                    s.link_label.set_visible(true);
                }
                None => s.link_label.set_visible(false),
            }
        });

        // Links that want a new window (target=_blank, middle- or ctrl-click) open as tabs.
        let weak = Rc::downgrade(self);
        view.connect_decide_policy(move |_, decision, kind| {
            let Some(s) = weak.upgrade() else {
                return false;
            };
            match kind {
                // A middle- or Ctrl+click on an ordinary link reaches us as a plain navigation, with
                // the button and keys attached: WebKit leaves it to the browser to make that a tab.
                PolicyDecisionType::NavigationAction => {
                    let Some(action) = decision
                        .downcast_ref::<NavigationPolicyDecision>()
                        .and_then(|d| d.navigation_action())
                    else {
                        return false;
                    };
                    let behind = action.navigation_type() == NavigationType::LinkClicked
                        && (action.mouse_button() == 2
                            || action.modifiers() & gdk::ModifierType::CONTROL_MASK.bits() != 0);
                    match action.request().and_then(|r| r.uri()) {
                        Some(uri) if behind && openable(&uri) => {
                            s.open_tab(Some(&uri), Some(id), false);
                            decision.ignore();
                            true
                        }
                        _ => false,
                    }
                }
                PolicyDecisionType::NewWindowAction => {
                    let action = decision
                        .downcast_ref::<NavigationPolicyDecision>()
                        .and_then(|d| d.navigation_action());
                    if let Some(action) = action
                        && let Some(uri) = action.request().and_then(|r| r.uri())
                        && openable(&uri)
                    {
                        let background = action.mouse_button() == 2
                            || action.modifiers() & gdk::ModifierType::CONTROL_MASK.bits() != 0;
                        s.open_tab(Some(&uri), Some(id), !background);
                    }
                    decision.ignore();
                    true
                }
                // Something that can't be shown, or that the server marked as an attachment, is saved.
                PolicyDecisionType::Response => {
                    let Some(d) = decision.downcast_ref::<ResponsePolicyDecision>() else {
                        return false;
                    };
                    let attachment = d
                        .response()
                        .and_then(|r| r.http_headers())
                        .and_then(|h| h.one("Content-Disposition"))
                        .is_some_and(|v| downloads::is_attachment(&v));
                    if attachment || !d.is_mime_type_supported() {
                        decision.download();
                        return true;
                    }
                    false
                }
                _ => false,
            }
        });
        // window.open() right after a click opens a tab; scripts opening popups on their own don't.
        let weak = Rc::downgrade(self);
        view.connect_create(move |opener, action| {
            let s = weak.upgrade()?;
            if !action.is_user_gesture() {
                return None;
            }
            Some(s.open_related(opener, id).upcast())
        });
        // A page asking for something: only the location is ever offered to the user.
        let weak = Rc::downgrade(self);
        view.connect_permission_request(move |view, request| {
            match weak.upgrade() {
                Some(s)
                    if request.is::<GeolocationPermissionRequest>()
                        && s.active_id() == Some(id) =>
                {
                    let uri = view.uri().map(|u| u.to_string()).unwrap_or_default();
                    s.ask_location(id, &uri, request.clone());
                }
                _ => request.deny(),
            }
            true
        });
        if let Some(finder) = view.find_controller() {
            let found = self.find_result(id);
            finder.connect_found_text(move |_, count| found(count));
            let missing = self.find_result(id);
            finder.connect_failed_to_find_text(move |_| missing(0));
        }
        // window.close()
        let weak = Rc::downgrade(self);
        view.connect_close(move |_| {
            if let Some(s) = weak.upgrade() {
                s.close_tab(id);
            }
        });
        view
    }

    /// A `notify` handler for tab `id` that gets the browser, the tab and the view.
    pub(super) fn on_view(
        self: &Rc<Self>,
        id: TabId,
        f: impl Fn(&Rc<Inner>, &Rc<Tab>, &WebView) + 'static,
    ) -> impl Fn(&WebView) + 'static {
        let weak = Rc::downgrade(self);
        move |w| {
            if let Some(s) = weak.upgrade()
                && let Some(t) = s.tab(id)
            {
                f(&s, &t, w);
            }
        }
    }

    pub(super) fn switch_to(self: &Rc<Self>, id: TabId) {
        let Some(tab) = self.tab(id) else { return };
        if self.active_id() != Some(id) {
            self.close_find();
            self.resolve_permission(false);
        }
        let blank = tab.view.borrow().is_none() && is_home(&tab.url.borrow());
        if tab.view.borrow().is_none() && !blank {
            self.restore(&tab);
        }
        // Switching to a tab folded away in the sidebar unfolds it.
        let hidden = self.order.borrow().shown_as(id) != id;
        self.order.borrow_mut().activate(id);
        if hidden {
            self.sync_tree();
        } else {
            self.sidebar.set_active(id);
        }
        tab.last_active.set(Instant::now());
        self.stack.set_visible_child_name(&if blank {
            "home".to_string()
        } else {
            id.to_string()
        });
        self.sync_chrome();
        if blank {
            if self.config.chrome == Chrome::Bar {
                self.bar.focus();
            }
        } else if let Some(v) = tab.view.borrow().as_ref() {
            v.grab_focus();
        }
        log!("switch to tab {id}");
    }

    /// Bring a discarded tab back: a new view with the old history and scroll position.
    pub(super) fn restore(self: &Rc<Self>, tab: &Rc<Tab>) {
        let view = self.create_view(tab, None);
        let url = tab.url.borrow().clone();
        match tab.session.borrow_mut().take() {
            Some(state) => {
                view.restore_session_state(&state);
                match view.back_forward_list().and_then(|l| l.current_item()) {
                    Some(item) => view.go_to_back_forward_list_item(&item),
                    None => view.load_uri(&url),
                }
            }
            None => view.load_uri(&url),
        }
        self.sync_row(tab);
        log!("restore tab {}", tab.id);
    }

    pub(super) fn close_tab(self: &Rc<Self>, id: TabId) {
        let Some(tab) = self.tab(id) else { return };
        self.withdraw_permission(id);
        let url = tab.url.borrow().clone();
        if !url.is_empty() && url != "bare://home" && url != "about:blank" {
            let mut closed = self.closed.borrow_mut();
            closed.push_back((url, self.order.borrow().parent(id)));
            if closed.len() > KEEP_CLOSED {
                closed.pop_front();
            }
        }
        if let Some(v) = tab.view.borrow_mut().take() {
            self.stack.remove(&v);
        }
        self.tabs.borrow_mut().remove(&id);
        trim_heap();
        let next = self.order.borrow_mut().close(id);
        log!("close tab {id}");
        if self.order.borrow().is_empty() {
            self.window.close();
            return;
        }
        self.sync_tree();
        if let Some(n) = next {
            self.switch_to(n);
        } else {
            self.sync_chrome();
        }
    }

    /// Ctrl+Shift+T: back under the tab it was under, if that one is still open.
    pub(super) fn reopen_closed(self: &Rc<Self>) {
        let closed = self.closed.borrow_mut().pop_back();
        match closed {
            Some((url, parent)) => {
                self.open_tab(Some(&url), parent, true);
            }
            None => self.show_toast("No closed tabs"),
        }
    }

    /// Fold or unfold the tabs under `id` in the sidebar. Folding away the tab you are on moves you
    /// to the folded one.
    pub(super) fn fold(self: &Rc<Self>, id: TabId) {
        let folded = self.order.borrow_mut().toggle_fold(id);
        log!("{} tab {id}", if folded { "fold" } else { "unfold" });
        self.sync_tree();
        if let Some(active) = self.active_id()
            && self.order.borrow().shown_as(active) != active
        {
            self.switch_to(id);
        }
    }

    pub(super) fn wire_sidebar(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.sidebar.connect_action(move |id, action| {
            let Some(s) = weak.upgrade() else { return };
            match action {
                sidebar::Action::Switch => s.switch_to(id),
                sidebar::Action::Close => s.close_tab(id),
                sidebar::Action::Fold => s.fold(id),
            }
        });
    }

    pub(super) fn start_discard_timer(self: &Rc<Self>) {
        let secs = std::env::var("BARE_DISCARD_SECS")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(u64::from(self.config.discard_minutes) * 60);
        if secs == 0 {
            return;
        }
        let weak = Rc::downgrade(self);
        // Check four times per idle period, so a tab is discarded at most 25% late.
        glib::timeout_add_seconds_local((secs / 4).clamp(1, 60) as u32, move || {
            match weak.upgrade() {
                Some(s) => {
                    s.discard_idle(Duration::from_secs(secs));
                    glib::ControlFlow::Continue
                }
                None => glib::ControlFlow::Break,
            }
        });
    }

    /// Unload background tabs that have been idle for `idle`. Tabs playing audio or still loading stay.
    pub(super) fn discard_idle(&self, idle: Duration) {
        let active = self.active_id();
        let tabs: Vec<Rc<Tab>> = self.tabs.borrow().values().cloned().collect();
        let mut discarded = false;
        for tab in tabs {
            if Some(tab.id) == active || tab.last_active.get().elapsed() < idle {
                continue;
            }
            let Some(view) = tab.view.borrow().clone() else {
                continue;
            };
            if view.is_playing_audio() || view.is_loading() {
                continue;
            }
            *tab.session.borrow_mut() = view.session_state();
            *tab.view.borrow_mut() = None;
            self.stack.remove(&view);
            // Dropping the view alone leaves WebKit's process cache holding the web process (and its
            // sandbox) for a long time, which defeats the point. End it now, unless another tab
            // lives in the same process.
            if !tab.shares_process.get() {
                view.terminate_web_process();
            }
            self.sync_row(&tab);
            log!("discard tab {}", tab.id);
            discarded = true;
        }
        if discarded {
            trim_heap();
        }
    }
    /// Turn a tab back into an empty new tab: its web view goes, and with it the web process.
    pub(super) fn blank(self: &Rc<Self>, tab: &Rc<Tab>) {
        if let Some(view) = tab.view.borrow_mut().take() {
            self.stack.remove(&view);
            if !tab.shares_process.get() {
                view.terminate_web_process();
            }
        }
        *tab.url.borrow_mut() = HOME.to_string();
        tab.title.borrow_mut().clear();
        *tab.session.borrow_mut() = None;
        tab.progress.set(0.0);
        tab.loading.set(false);
        self.sync_row(tab);
        if self.active_id() == Some(tab.id) {
            self.switch_to(tab.id);
        }
    }
}

/// Give freed heap memory back to the system. glibc keeps what a dropped web view freed for reuse,
/// so without this a discarded or closed tab still counts against Bare's own process. WebKit does
/// the same in its processes when memory runs short.
fn trim_heap() {
    #[cfg(target_env = "gnu")]
    {
        unsafe extern "C" {
            fn malloc_trim(pad: usize) -> std::ffi::c_int;
        }
        // SAFETY: malloc_trim only releases free memory; it is safe to call at any time.
        unsafe { malloc_trim(0) };
    }
}
