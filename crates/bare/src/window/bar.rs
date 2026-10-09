//! The URL bar: suggestions, navigation, history and bookmarks.

use super::*;

impl Inner {
    // ---- history and bookmarks ----------------------------------------------------------------

    pub(super) fn remember_visit(&self, url: &str) {
        let Some(h) = &self.history else { return };
        let url = url.split('#').next().unwrap_or(url);
        if !recordable(url) {
            return;
        }
        if let Err(e) = h.record_visit(url, "", now_secs()) {
            eprintln!("bare: history: {e}");
        }
    }

    pub(super) fn remember_title(&self, url: &str, title: &str) {
        if let Some(h) = &self.history {
            let url = url.split('#').next().unwrap_or(url);
            if recordable(url) {
                let _ = h.set_title(url, title);
            }
        }
    }

    pub(super) fn toggle_bookmark(self: &Rc<Self>) {
        let Some(tab) = self.active_tab() else { return };
        let url = tab.url.borrow().clone();
        if !recordable(&url) {
            self.show_toast("This page can't be bookmarked");
            return;
        }
        let title = tab.title.borrow().clone();
        let added = self.bookmarks.borrow_mut().toggle(&url, &title);
        match self.bookmarks.borrow().save(&self.paths.bookmarks_file()) {
            Ok(()) => self.show_toast(if added {
                "Bookmarked"
            } else {
                "Bookmark removed"
            }),
            Err(e) => {
                eprintln!("bare: cannot save bookmarks: {e}");
                self.show_toast("Couldn't save bookmarks");
            }
        }
    }

    // ---- URL bar ------------------------------------------------------------------------------

    /// The bar's text changed: refresh the list, if the bar is the thing being edited.
    pub(super) fn refresh_dropdown(&self) {
        if self.bar.focused() {
            self.build_dropdown();
        } else {
            self.dropdown.hide();
        }
    }

    /// Fill the list for what is typed (or, if nothing has been typed since focusing, for "nothing":
    /// the tab switcher). Doesn't ask whether the bar has focus: when this runs from the focus handler
    /// the bar's own bookkeeping may not have caught up yet.
    pub(super) fn build_dropdown(&self) {
        let text = if self.bar_dirty.get() {
            self.bar.entry().text().to_string()
        } else {
            String::new()
        };
        let active = self.active_id();
        let tabs: Vec<TabInfo> = self
            .order
            .borrow()
            .mru()
            .iter()
            .filter_map(|id| self.tab(*id))
            .map(|t| TabInfo {
                id: t.id,
                title: t.title.borrow().clone(),
                url: t.url.borrow().clone(),
                current: Some(t.id) == active,
            })
            .collect();
        let hits = self
            .history
            .as_ref()
            .and_then(|h| h.search(&suggest::tokens(&text), 20, now_secs()).ok())
            .unwrap_or_default();
        let list = suggest::suggest(&text, &tabs, &self.bookmarks.borrow(), &hits, SUGGESTIONS);
        log!(
            "dropdown for {text:?}: {:?}",
            list.iter()
                .map(|s| (&s.kind, s.title.as_str()))
                .collect::<Vec<_>>()
        );
        self.dropdown.set(list);
    }

    /// Enter in the URL bar: the highlighted suggestion if the user moved to one, else what was typed.
    pub(super) fn activate_bar(self: &Rc<Self>, new_tab: bool) {
        let text = self.bar.entry().text().to_string();
        let chosen = self.dropdown.chosen();
        self.dropdown.hide();
        match chosen {
            Some(s) => self.open_suggestion(s, new_tab),
            None => match resolve_url(&text) {
                Some(url) => self.navigate(&url, new_tab),
                None => {
                    if let Some(v) = self.active_view() {
                        v.grab_focus();
                    }
                }
            },
        }
    }

    pub(super) fn open_suggestion(self: &Rc<Self>, s: Suggestion, new_tab: bool) {
        self.dropdown.hide();
        log!("suggestion {:?}", s.kind);
        match s.kind {
            Kind::Tab(id) => self.switch_to(id),
            Kind::Go | Kind::Search => {
                if let Some(url) = resolve_url(&s.url) {
                    self.navigate(&url, new_tab);
                }
            }
            Kind::Bookmark | Kind::History => self.navigate(&s.url, new_tab),
        }
    }

    pub(super) fn navigate(self: &Rc<Self>, url: &str, new_tab: bool) {
        if new_tab {
            self.open_tab(Some(url), None, true);
            return;
        }
        let Some(tab) = self.active_tab() else { return };
        if is_home(url) {
            self.blank(&tab);
            return;
        }
        *tab.url.borrow_mut() = url.to_string();
        // A blank tab gets its web view (and process) only now.
        let view = tab.view.borrow().clone();
        let view = view.unwrap_or_else(|| {
            let v = self.create_view(&tab, None);
            self.stack.set_visible_child_name(&tab.id.to_string());
            v
        });
        view.load_uri(url);
        view.grab_focus();
        self.bar.set_url(url);
    }

    pub(super) fn wire_bar(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        let entry = self.bar.entry();

        let focus = gtk::EventControllerFocus::new();
        focus.connect_enter({
            let weak = weak.clone();
            move |_| {
                if let Some(s) = weak.upgrade() {
                    s.bar_dirty.set(false);
                    s.build_dropdown();
                }
            }
        });
        focus.connect_leave({
            let weak = weak.clone();
            move |_| {
                let Some(s) = weak.upgrade() else { return };
                s.dropdown.hide();
                if s.config.chrome == Chrome::Hidden || s.window.is_fullscreen() {
                    s.revealer.set_reveal_child(false);
                }
            }
        });
        entry.add_controller(focus);

        entry.connect_changed({
            let weak = weak.clone();
            move |_| {
                if let Some(s) = weak.upgrade()
                    && s.bar.focused()
                {
                    s.bar_dirty.set(true);
                    s.refresh_dropdown();
                }
            }
        });

        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        keys.connect_key_pressed({
            let weak = weak.clone();
            move |_, key, _, state| {
                let Some(s) = weak.upgrade() else {
                    return glib::Propagation::Proceed;
                };
                let ctrl = state.contains(gdk::ModifierType::CONTROL_MASK);
                match key {
                    gdk::Key::Escape => {
                        s.dropdown.hide();
                        // Focus leaving the bar restores the page URL. An empty tab has no page to
                        // hand focus to, so just drop it.
                        if let Some(v) = s.active_view() {
                            v.grab_focus();
                        } else {
                            gtk::prelude::GtkWindowExt::set_focus(&s.window, None::<&gtk::Widget>);
                        }
                    }
                    gdk::Key::Down | gdk::Key::KP_Down => s.dropdown.move_selection(1),
                    gdk::Key::Up | gdk::Key::KP_Up => s.dropdown.move_selection(-1),
                    gdk::Key::n if ctrl => s.dropdown.move_selection(1),
                    gdk::Key::p if ctrl => s.dropdown.move_selection(-1),
                    gdk::Key::Return | gdk::Key::KP_Enter | gdk::Key::ISO_Enter => {
                        s.activate_bar(state.contains(gdk::ModifierType::ALT_MASK));
                    }
                    _ => return glib::Propagation::Proceed,
                }
                glib::Propagation::Stop
            }
        });
        entry.add_controller(keys);

        self.dropdown.connect_activate({
            let weak = weak.clone();
            move |suggestion, new_tab| {
                if let Some(s) = weak.upgrade() {
                    s.open_suggestion(suggestion, new_tab);
                }
            }
        });

        // Clicking the tab count opens the switcher: the bar with nothing typed lists the tabs.
        let click = gtk::GestureClick::builder()
            .button(gdk::BUTTON_PRIMARY)
            .build();
        click.connect_pressed(move |_, _, _, _| {
            if let Some(s) = weak.upgrade() {
                s.focus_bar();
            }
        });
        self.bar.badge().add_controller(click);
    }
}
