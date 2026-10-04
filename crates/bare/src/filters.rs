//! Content blocking, applied inside WebKit. The lists (a built-in baseline, or the full lists that
//! `bare --update-filters` downloads) are compiled by WebKit once into a binary form that it caches
//! on disk, then every tab shares them. A cached list loads in milliseconds; only the first
//! compile of a big list takes seconds, and it happens in the background.
//!
//! Bare keeps the full lists fresh itself: shortly after start-up, if they are missing or a week old,
//! it runs `bare --update-filters` as a child process and loads the result without a restart.

use crate::log;
use bare_core::{Paths, filters};
use gtk4::glib;
use std::{
    cell::Cell,
    process::{Command, Stdio},
    rc::Rc,
    time::Duration,
    time::Instant,
};
use webkit6::{UserContentFilter, UserContentFilterStore, UserContentManager};

pub struct Filters {
    manager: UserContentManager,
    /// `None` when blocking is off.
    store: Option<UserContentFilterStore>,
    ready: Rc<Cell<bool>>,
}

impl Filters {
    /// Start loading the filters in the background. With `enabled == false` nothing is blocked.
    pub fn start(paths: &Paths, enabled: bool) -> Self {
        let manager = UserContentManager::new();
        let ready = Rc::new(Cell::new(!enabled));
        let store = enabled
            .then(|| UserContentFilterStore::new(&paths.cache.join("filters").to_string_lossy()));
        if let Some(store) = store.clone() {
            let (m, r, p) = (manager.clone(), ready.clone(), paths.clone());
            glib::spawn_future_local(async move {
                swap_in(&store, &m, &p).await;
                r.set(true);
            });
        }
        Self {
            manager,
            store,
            ready,
        }
    }

    pub fn manager(&self) -> &UserContentManager {
        &self.manager
    }

    /// Let the main loop run until the filters are in place, but no longer than `max`. A cached list is
    /// ready within a few milliseconds, which is what makes it worth holding the first page for; a
    /// first-time compile isn't waited for.
    pub fn wait_ready(&self, max: Duration) {
        let deadline = Instant::now() + max;
        let context = glib::MainContext::default();
        while !self.ready.get() && Instant::now() < deadline {
            if !context.iteration(false) {
                std::thread::sleep(Duration::from_millis(1));
            }
        }
    }

    /// Fetch the full lists if they are missing or a week old (see `filters::needs_update`). The fetch
    /// and the conversion run in a child process (`bare --update-filters`), so the browser itself never
    /// holds a download or the converter's memory; when the child is done the new list is compiled and
    /// swapped in. Pages loaded after that are blocked by it; the baseline protects until then.
    pub fn maybe_update(&self, paths: &Paths) {
        let Some(store) = self.store.clone() else {
            return;
        };
        if !filters::needs_update(
            filters::file_age(&paths.filters_file()),
            filters::file_age(&paths.filters_attempt_file()),
        ) {
            log!("filters: no update needed");
            return;
        }
        let exe = match std::env::current_exe() {
            Ok(exe) => exe,
            Err(e) => {
                log!("filters: update failed ({e})");
                return;
            }
        };
        log!("filters: update started");
        let (tx, rx) = async_channel::bounded::<Result<(), String>>(1);
        std::thread::spawn(move || {
            let result = match Command::new(exe)
                .arg("--update-filters")
                .stdin(Stdio::null())
                .output()
            {
                Ok(out) if out.status.success() => Ok(()),
                Ok(out) => Err(String::from_utf8_lossy(&out.stderr)
                    .lines()
                    .last()
                    .unwrap_or("it failed")
                    .to_string()),
                Err(e) => Err(e.to_string()),
            };
            let _ = tx.send_blocking(result);
        });
        let (manager, paths) = (self.manager.clone(), paths.clone());
        glib::spawn_future_local(async move {
            match rx.recv().await {
                Ok(Ok(())) => {
                    log!("filters: updated");
                    swap_in(&store, &manager, &paths).await;
                }
                Ok(Err(e)) => log!("filters: update failed ({e})"),
                Err(_) => {}
            }
        });
    }
}

/// Where the active list comes from: the downloaded one if present, else the built-in baseline.
struct Source {
    id: String,
    user: bool,
}

fn source(paths: &Paths) -> Source {
    if paths.filters_file().exists() {
        let id = std::fs::read_to_string(paths.filters_id_file())
            .map(|s| s.trim().to_string())
            .ok()
            .filter(|s| !s.is_empty())
            .or_else(|| {
                std::fs::read_to_string(paths.filters_file())
                    .ok()
                    .map(|j| filters::fingerprint(&j))
            });
        if let Some(id) = id {
            return Source { id, user: true };
        }
    }
    Source {
        id: filters::fingerprint(&filters::convert([filters::BUILTIN]).json),
        user: false,
    }
}

fn json_for(paths: &Paths, src: &Source) -> String {
    if src.user
        && let Ok(json) = std::fs::read_to_string(paths.filters_file())
    {
        return json;
    }
    filters::convert([filters::BUILTIN]).json
}

/// A compiled list, and whether compiling it was needed (a cached one wasn't).
struct Loaded {
    filter: UserContentFilter,
    id: String,
    compiled: bool,
}

/// Make the current list the only one in force. The new one is compiled (or read from WebKit's
/// cache) first and exchanged for the old one in a single step, so nothing is ever unprotected;
/// if it can't be had, the old one stays.
async fn swap_in(store: &UserContentFilterStore, manager: &UserContentManager, paths: &Paths) {
    let Some(loaded) = load(store, paths).await else {
        return;
    };
    manager.remove_all_filters();
    manager.add_filter(&loaded.filter);
    if loaded.compiled {
        forget_previous(store, paths, &loaded.id).await;
    }
}

async fn load(store: &UserContentFilterStore, paths: &Paths) -> Option<Loaded> {
    let src = source(paths);
    let started = Instant::now();
    if let Ok(filter) = store.load_future(&src.id).await {
        log!(
            "filters: {} loaded from cache in {} ms",
            src.id,
            started.elapsed().as_millis()
        );
        return Some(Loaded {
            filter,
            id: src.id,
            compiled: false,
        });
    }
    // Not compiled yet (first run, or the list changed).
    let json = json_for(paths, &src);
    let bytes = glib::Bytes::from_owned(json.into_bytes());
    match store.save_future(&src.id, &bytes).await {
        Ok(filter) => {
            log!(
                "filters: {} compiled in {} ms",
                src.id,
                started.elapsed().as_millis()
            );
            Some(Loaded {
                filter,
                id: src.id,
                compiled: true,
            })
        }
        Err(e) if src.user => {
            // A downloaded list WebKit refuses must not leave the browser unprotected.
            eprintln!(
                "bare: WebKit rejected the downloaded filter list ({e}); using the built-in baseline"
            );
            let builtin = filters::convert([filters::BUILTIN]).json;
            let id = filters::fingerprint(&builtin);
            let filter = store
                .save_future(&id, &glib::Bytes::from_owned(builtin.into_bytes()))
                .await
                .ok()?;
            log!("filters: fell back to built-in {id}");
            Some(Loaded {
                filter,
                id,
                compiled: true,
            })
        }
        Err(e) => {
            eprintln!("bare: content blocking unavailable: {e}");
            None
        }
    }
}

/// The compiled copy of a superseded list is dead weight on disk.
async fn forget_previous(store: &UserContentFilterStore, paths: &Paths, current: &str) {
    let marker = paths.cache.join("filters-current");
    if let Ok(previous) = std::fs::read_to_string(&marker) {
        let previous = previous.trim();
        if !previous.is_empty() && previous != current {
            let _ = store.remove_future(previous).await;
        }
    }
    let _ = std::fs::write(marker, current);
}
