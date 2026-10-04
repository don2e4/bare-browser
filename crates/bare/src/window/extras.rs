//! Find in page, the location-permission prompt, and downloads.

use super::*;

impl Inner {
    // ---- find in page -------------------------------------------------------------------------

    pub(super) fn open_find(&self) {
        if self.active_view().is_none() {
            return;
        }
        self.find.open();
        if !self.find.text().is_empty() {
            self.find_text_changed();
        }
    }

    /// Close the find box and clear the highlights in the page.
    pub(super) fn close_find(&self) {
        if !self.find.is_open() {
            return;
        }
        if let Some(finder) = self.active_view().and_then(|v| v.find_controller()) {
            finder.search_finish();
        }
        self.find.close();
    }

    pub(super) fn find_text_changed(&self) {
        let Some(finder) = self.active_view().and_then(|v| v.find_controller()) else {
            return;
        };
        let text = self.find.text();
        if text.is_empty() {
            finder.search_finish();
            self.find.set_count(None);
        } else {
            finder.search(
                &text,
                (FindOptions::CASE_INSENSITIVE | FindOptions::WRAP_AROUND).bits(),
                1000,
            );
        }
    }

    pub(super) fn find_step(&self, forward: bool) {
        if self.find.text().is_empty() {
            self.open_find();
            return;
        }
        if let Some(finder) = self.active_view().and_then(|v| v.find_controller()) {
            if forward {
                finder.search_next();
            } else {
                finder.search_previous();
            }
        }
    }

    /// Reports a match count for tab `id` (shown only while that tab is the one on screen).
    pub(super) fn find_result(self: &Rc<Self>, id: TabId) -> impl Fn(u32) + 'static {
        let weak = Rc::downgrade(self);
        move |count| {
            if let Some(s) = weak.upgrade()
                && s.active_id() == Some(id)
            {
                log!("find: {count} matches");
                s.find.set_count(Some(count));
            }
        }
    }

    pub(super) fn wire_find(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.find.entry().connect_changed({
            let weak = weak.clone();
            move |_| {
                if let Some(s) = weak.upgrade() {
                    s.find_text_changed();
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
                match key {
                    gdk::Key::Escape => {
                        s.close_find();
                        if let Some(v) = s.active_view() {
                            v.grab_focus();
                        }
                    }
                    gdk::Key::Return | gdk::Key::KP_Enter => {
                        s.find_step(!state.contains(gdk::ModifierType::SHIFT_MASK));
                    }
                    _ => return glib::Propagation::Proceed,
                }
                glib::Propagation::Stop
            }
        });
        self.find.entry().add_controller(keys);
        self.find.next.connect_clicked({
            let weak = weak.clone();
            move |_| {
                if let Some(s) = weak.upgrade() {
                    s.find_step(true);
                }
            }
        });
        self.find.previous.connect_clicked(move |_| {
            if let Some(s) = weak.upgrade() {
                s.find_step(false);
            }
        });
    }

    // ---- permissions ------------------------------------------------------------------------------

    pub(super) fn ask_location(&self, tab: TabId, uri: &str, request: PermissionRequest) {
        self.resolve_permission(false);
        let host = bare_core::display::host_range(uri).map_or(uri, |r| &uri[r]);
        let who = if host.is_empty() { "This page" } else { host };
        self.permission
            .ask(&format!("{who} wants to know your location."));
        *self.pending_permission.borrow_mut() = Some((tab, request));
        log!("permission: asking about location for tab {tab}");
    }

    /// Answer the waiting request (refusing is the default for anything that interrupts it).
    pub(super) fn resolve_permission(&self, allow: bool) {
        let pending = self.pending_permission.borrow_mut().take();
        if let Some((_, request)) = pending {
            if allow {
                request.allow();
            } else {
                request.deny();
            }
            log!("permission: {}", if allow { "allowed" } else { "blocked" });
        }
        self.permission.hide();
    }

    /// The tab that was asking navigated away or closed: its question no longer applies.
    pub(super) fn withdraw_permission(&self, tab: TabId) {
        let asking = self
            .pending_permission
            .borrow()
            .as_ref()
            .is_some_and(|(t, _)| *t == tab);
        if asking {
            self.resolve_permission(false);
        }
    }

    pub(super) fn wire_permissions(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.permission.allow.connect_clicked({
            let weak = weak.clone();
            move |_| {
                if let Some(s) = weak.upgrade() {
                    s.resolve_permission(true);
                    if let Some(v) = s.active_view() {
                        v.grab_focus();
                    }
                }
            }
        });
        self.permission.block.connect_clicked({
            let weak = weak.clone();
            move |_| {
                if let Some(s) = weak.upgrade() {
                    s.resolve_permission(false);
                    if let Some(v) = s.active_view() {
                        v.grab_focus();
                    }
                }
            }
        });
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        keys.connect_key_pressed(move |_, key, _, _| {
            if key == gdk::Key::Escape
                && let Some(s) = weak.upgrade()
            {
                s.resolve_permission(false);
                if let Some(v) = s.active_view() {
                    v.grab_focus();
                }
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        self.permission.widget.add_controller(keys);
    }

    // ---- downloads --------------------------------------------------------------------------------

    /// Downloads are saved straight into the downloads folder; a toast says when one starts and ends.
    pub(super) fn wire_downloads(self: &Rc<Self>, session: &NetworkSession) {
        let weak = Rc::downgrade(self);
        session.connect_download_started(move |_, download| {
            let weak = weak.clone();
            download.connect_decide_destination({
                let weak = weak.clone();
                move |download, suggested| {
                    let Some(s) = weak.upgrade() else {
                        return false;
                    };
                    let dir = s.paths.downloads.clone();
                    if let Err(e) = std::fs::create_dir_all(&dir) {
                        eprintln!("bare: cannot create {}: {e}", dir.display());
                        s.show_toast("Can't save: the downloads folder is unavailable");
                        download.cancel();
                        return true;
                    }
                    let name = downloads::sanitize_filename(suggested);
                    let path = downloads::unique_path(&dir, &name, |p| p.exists());
                    download.set_destination(&path.to_string_lossy());
                    let shown = path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or(name);
                    log!("download: {}", path.display());
                    s.show_toast(&format!("Downloading {shown}"));
                    true
                }
            });
            download.connect_finished({
                let weak = weak.clone();
                move |download| {
                    if let Some(s) = weak.upgrade() {
                        let name = download
                            .destination()
                            .and_then(|d| {
                                Path::new(d.as_str())
                                    .file_name()
                                    .map(|n| n.to_string_lossy().into_owned())
                            })
                            .unwrap_or_default();
                        log!("download finished: {name}");
                        s.show_toast(&format!("Saved {name} to Downloads"));
                    }
                }
            });
            download.connect_failed(move |_, error| {
                if let Some(s) = weak.upgrade() {
                    log!("download failed: {error}");
                    s.show_toast("Download failed");
                }
            });
        });
    }
}
