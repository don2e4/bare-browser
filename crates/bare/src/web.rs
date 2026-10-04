//! The WebKit side: session (storage, cookie policy), view settings, navigation policy, error pages.

use bare_core::{Paths, input, pages};
use bare_search::Searcher;
use gtk4::{gio, glib};
use std::{cell::Cell, rc::Rc, sync::Arc};
use webkit6::{
    CookieAcceptPolicy, CookiePersistentStorage, LoadEvent, NetworkError, NetworkSession,
    PolicyError, Settings, URISchemeRequest, UserContentManager, WebContext,
    WebProcessTerminationReason, WebView, prelude::*,
};

pub fn new_session(paths: &Paths) -> NetworkSession {
    let data = paths.data.join("web");
    let cache = paths.cache.join("web");
    for dir in [&data, &cache] {
        if let Err(e) = std::fs::create_dir_all(dir) {
            eprintln!("bare: cannot create {}: {e}", dir.display());
        }
    }
    let session = NetworkSession::new(data.to_str(), cache.to_str());
    session.set_itp_enabled(true);
    if let Some(cookies) = session.cookie_manager() {
        cookies.set_accept_policy(CookieAcceptPolicy::NoThirdParty);
        if let Some(file) = data.join("cookies.sqlite").to_str() {
            cookies.set_persistent_storage(file, CookiePersistentStorage::Sqlite);
        }
    }
    session
}

/// A web view plus a flag that is set while the current page is a load-error page (so it isn't
/// recorded in history). Create one per tab; pass the opener as `related` for `window.open`.
pub struct View {
    pub view: WebView,
    pub failed: Rc<Cell<bool>>,
}

pub fn new_view(
    session: &NetworkSession,
    related: Option<&WebView>,
    filters: &UserContentManager,
) -> View {
    let settings = Settings::new();
    settings.set_media_playback_requires_user_gesture(true);
    settings.set_javascript_can_open_windows_automatically(false);
    settings.set_enable_write_console_messages_to_stdout(false);
    // Memory over instant Back: the back/forward page cache keeps whole pages (and the web processes
    // that rendered them) alive after you navigate away. Without it Back reloads from the HTTP cache.
    set_feature(&settings, "UsesBackForwardCache", false);

    let view = match related {
        // A related view shares its opener's web process and network session.
        Some(r) => WebView::builder()
            .related_view(r)
            .hexpand(true)
            .vexpand(true)
            .build(),
        None => WebView::builder()
            .network_session(session)
            .user_content_manager(filters)
            .hexpand(true)
            .vexpand(true)
            .build(),
    };
    view.set_settings(&settings);
    let failed = Rc::new(Cell::new(false));
    wire_errors(&view, failed.clone());
    View { view, failed }
}

/// Serve `bare://…` pages from inside the process (no server, no port). Real URIs, so they get
/// proper history entries. Display-isolated: web pages can't embed or probe them.
pub fn register_scheme(home_hint: bool, searcher: Arc<Searcher>) {
    let Some(context) = WebContext::default() else {
        return;
    };
    if let Some(security) = context.security_manager() {
        security.register_uri_scheme_as_secure("bare");
        security.register_uri_scheme_as_display_isolated("bare");
    }
    context.register_uri_scheme("bare", move |request| {
        let uri = request.uri().unwrap_or_default();
        let page = uri.strip_prefix("bare://").unwrap_or("");
        let page = page
            .split(['?', '#'])
            .next()
            .unwrap_or("")
            .trim_end_matches('/');
        match page {
            "home" => finish(request, &pages::home(home_hint)),
            "search" => serve_search(
                request,
                searcher.clone(),
                input::query_param(&uri, "q").unwrap_or_default(),
            ),
            other => finish(
                request,
                &pages::error(
                    "Page not found",
                    &format!("There is no bare://{other} page."),
                    &uri,
                ),
            ),
        }
    });
}

fn finish(request: &URISchemeRequest, html: &str) {
    let bytes = glib::Bytes::from(html.as_bytes());
    let stream = gio::MemoryInputStream::from_bytes(&bytes);
    request.finish(
        &stream,
        bytes.len() as i64,
        Some("text/html; charset=utf-8"),
    );
}

/// Searching takes seconds, so it runs on its own thread and the page is handed back to the main
/// loop when it is ready; the window stays responsive meanwhile.
fn serve_search(request: &URISchemeRequest, searcher: Arc<Searcher>, query: String) {
    let (tx, rx) = async_channel::bounded::<String>(1);
    std::thread::spawn(move || {
        let response = searcher.search(&query);
        crate::log!(
            "search: {} results from {} of {} engines",
            response.items.len(),
            response.answered(),
            response.asked()
        );
        let _ = tx.send_blocking(bare_search::render::page(&response));
    });
    let request = request.clone();
    glib::spawn_future_local(async move {
        let html = rx.recv().await.unwrap_or_else(|_| {
            pages::error(
                "Search failed",
                "The search stopped unexpectedly.",
                "bare://search",
            )
        });
        finish(&request, &html);
    });
}

/// Flip one of WebKit's feature flags by identifier (they have no typed setters).
fn set_feature(settings: &Settings, identifier: &str, enabled: bool) {
    let Some(list) = Settings::all_features() else {
        return;
    };
    for i in 0..list.length() {
        if let Some(feature) = list.get(i)
            && feature.identifier().as_deref() == Some(identifier)
        {
            settings.set_feature_enabled(&feature, enabled);
            return;
        }
    }
    eprintln!("bare: WebKit has no feature `{identifier}`");
}

fn wire_errors(view: &WebView, failed: Rc<Cell<bool>>) {
    // Showing an error page is itself a load (`load_alternate_html`), and its Started event must not
    // clear the flag that says "this page is an error page". Real navigations clear it.
    let error_page_pending = Rc::new(Cell::new(false));
    view.connect_load_changed({
        let failed = failed.clone();
        let error_page_pending = error_page_pending.clone();
        move |_, event| {
            if event == LoadEvent::Started {
                if error_page_pending.replace(false) {
                    return;
                }
                failed.set(false);
            }
        }
    });
    view.connect_load_failed({
        let failed = failed.clone();
        let error_page_pending = error_page_pending.clone();
        move |view, _event, uri, error| {
            // Navigating away mid-load, or a load turned into a download, is not an error to show.
            if error.matches(NetworkError::Cancelled)
                || error.matches(PolicyError::FrameLoadInterruptedByPolicyChange)
            {
                return true;
            }
            failed.set(true);
            error_page_pending.set(true);
            let title = if error.matches(NetworkError::FileDoesNotExist) {
                "File not found"
            } else {
                "Can't load this page"
            };
            view.load_alternate_html(&pages::error(title, error.message(), uri), uri, Some(uri));
            true
        }
    });
    // The page's process died (crash, or out of memory). Say so instead of leaving a blank tab. Ending
    // it on purpose (discarding a tab) is not an error.
    view.connect_web_process_terminated({
        let failed = failed.clone();
        let error_page_pending = error_page_pending.clone();
        move |view, reason| {
            if reason == WebProcessTerminationReason::TerminatedByApi {
                return;
            }
            let Some(uri) = view.uri() else { return };
            failed.set(true);
            error_page_pending.set(true);
            let detail = match reason {
                WebProcessTerminationReason::ExceededMemoryLimit => {
                    "The page used too much memory and was stopped."
                }
                _ => "The page's process stopped unexpectedly.",
            };
            view.load_alternate_html(
                &pages::error(
                    "This page stopped working",
                    &format!("{detail} Reload to try again."),
                    &uri,
                ),
                &uri,
                Some(&uri),
            );
        }
    });
    view.connect_load_failed_with_tls_errors(move |view, uri, _cert, flags| {
        failed.set(true);
        error_page_pending.set(true);
        let detail = format!("{} Bare won't load the page.", tls_reason(flags));
        view.load_alternate_html(
            &pages::error("Connection isn't secure", &detail, uri),
            uri,
            Some(uri),
        );
        true
    });
}

fn tls_reason(flags: gio::TlsCertificateFlags) -> &'static str {
    use gio::TlsCertificateFlags as F;
    if flags.contains(F::UNKNOWN_CA) {
        "The site's certificate was issued by an authority this computer doesn't trust."
    } else if flags.contains(F::BAD_IDENTITY) {
        "The site's certificate doesn't match its address."
    } else if flags.contains(F::EXPIRED) {
        "The site's certificate has expired."
    } else if flags.contains(F::NOT_ACTIVATED) {
        "The site's certificate isn't valid yet. Check this computer's clock."
    } else if flags.contains(F::REVOKED) {
        "The site's certificate has been revoked."
    } else {
        "The site's certificate couldn't be verified."
    }
}
