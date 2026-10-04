//! The results page: the "Dense" concept. One line per result; the selected row expands to show
//! the address, snippet and which engines found it. Plain HTML + one stylesheet; the only script is
//! a few lines for j/k. A strict CSP means the page can load nothing and talk to nothing.

use crate::search::{Response, Status};
use bare_core::input::percent_encode;
use bare_core::pages::escape;
use std::fmt::Write;
use url::Url;

/// Offer "did you mean" only when there are fewer results than this.
const SUGGEST_BELOW: usize = 8;

const CSS: &str = include_str!("../../../resources/results.css");

const SCRIPT: &str = "(function(){var r=[].slice.call(document.querySelectorAll('a.row'));if(!r.length)return;\
function go(d){var i=r.indexOf(document.activeElement);i=i<0?0:Math.max(0,Math.min(r.length-1,i+d));r[i].focus();r[i].scrollIntoView({block:'nearest'});}\
document.addEventListener('keydown',function(e){\
if(e.key==='Enter'&&(e.shiftKey||e.ctrlKey)){var a=document.activeElement;if(a&&a.classList&&a.classList.contains('row'))a.target='_blank';return;}\
if(e.ctrlKey||e.altKey||e.metaKey)return;\
if(e.key==='j'||e.key==='ArrowDown'){e.preventDefault();go(1);}else if(e.key==='k'||e.key==='ArrowUp'){e.preventDefault();go(-1);}});\
r[0].focus({preventScroll:true});})();";

pub fn page(r: &Response) -> String {
    let mut h = String::with_capacity(16 * 1024);
    let _ = write!(
        h,
        "<!doctype html><html lang=\"en\"><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
         <meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; style-src 'unsafe-inline'; script-src 'unsafe-inline'\">\
         <title>{}</title><style>{CSS}</style>",
        if r.query.is_empty() {
            "Bare Search".to_string()
        } else {
            escape(&r.query)
        }
    );

    h.push_str("<header class=\"status\">");
    if r.query.is_empty() {
        h.push_str("<span class=\"n\">bare search</span>");
    } else {
        let _ = write!(
            h,
            "<span class=\"n\">{} result{}</span>",
            r.items.len(),
            if r.items.len() == 1 { "" } else { "s" }
        );
        let _ = write!(
            h,
            "<span>{} of {} engines</span><span>{:.2} s</span>",
            r.answered(),
            r.asked(),
            r.elapsed.as_secs_f64()
        );
        for rep in &r.reports {
            if let Some((word, why)) = problem(&rep.status) {
                let _ = write!(
                    h,
                    "<span class=\"warn\" title=\"{}\">{} {}</span>",
                    escape(&why),
                    escape(&rep.engine),
                    word
                );
            }
        }
    }
    h.push_str("</header>");

    // A correction is for searches that went badly; beside a page of good results it is just noise.
    if let Some(s) = r
        .suggestions
        .first()
        .filter(|_| r.items.len() < SUGGEST_BELOW)
    {
        let _ = write!(
            h,
            "<p class=\"suggest\">Did you mean <a href=\"bare://search?q={}\">{}</a>?</p>",
            percent_encode(s, false),
            escape(s)
        );
    }

    if r.query.is_empty() {
        h.push_str("<p class=\"empty\">Type something in the address bar to search.</p>");
    } else if r.items.is_empty() {
        let msg = if r.answered() == 0 && r.asked() > 0 {
            "No search engine answered. Are you online?"
        } else {
            "Nothing found. Try fewer or different words."
        };
        let _ = write!(h, "<p class=\"empty\">{msg}</p>");
    } else {
        let total = r.answered();
        h.push_str("<main><ol class=\"results\">");
        for (i, item) in r.items.iter().enumerate() {
            let host = Url::parse(&item.url)
                .ok()
                .and_then(|u| {
                    u.host_str()
                        .map(|s| s.strip_prefix("www.").unwrap_or(s).to_string())
                })
                .unwrap_or_default();
            let found = item.engines.len();
            let dots = format!(
                "{}{}",
                "●".repeat(found),
                "○".repeat(total.saturating_sub(found))
            );
            let shown_url = item
                .url
                .strip_prefix("https://")
                .or_else(|| item.url.strip_prefix("http://"))
                .unwrap_or(&item.url);
            let _ = write!(
                h,
                "<li class=\"r\"><a class=\"row\" href=\"{url}\" rel=\"noreferrer\" title=\"{engines}\">\
                 <span class=\"rank\">{rank:02}</span><span class=\"dom\">{host}</span><span class=\"ttl\">{title}</span>\
                 <span class=\"dots\">{dots} {found}</span></a>\
                 <div class=\"detail\"><div class=\"url\">{shown}</div>{snip}<div class=\"eng\">{engines}</div></div></li>",
                url = escape(&item.url),
                engines = escape(&item.engines.join("  ")),
                rank = i + 1,
                host = escape(&host),
                title = escape(&item.title),
                shown = escape(shown_url),
                snip = if item.content.is_empty() {
                    String::new()
                } else {
                    format!("<div class=\"snip\">{}</div>", escape(&item.content))
                },
            );
        }
        h.push_str("</ol></main>");
    }

    let _ = write!(
        h,
        "<footer class=\"keys\"><span><kbd>j</kbd><kbd>k</kbd>move</span><span><kbd>\u{21b5}</kbd>open</span>\
         <span><kbd>\u{21e7}\u{21b5}</kbd>new tab</span><span><kbd>Ctrl+L</kbd>refine</span></footer><script>{SCRIPT}</script>"
    );
    h
}

/// How an engine's trouble reads in the status line, with the detail for the tooltip.
fn problem(s: &Status) -> Option<(String, String)> {
    match s {
        Status::Ok(_) | Status::Empty => None,
        Status::Blocked(why) => Some(("blocked".into(), why.clone())),
        Status::Failed(why) => Some(("failed".into(), why.clone())),
        Status::Late => Some(("slow".into(), "still working; not waited for".into())),
        Status::Suspended(secs) => {
            let left = if *secs >= 90 {
                format!("{}m", (secs + 30) / 60)
            } else {
                format!("{secs}s")
            };
            Some((
                format!("paused {left}"),
                "failed recently; will be asked again later".into(),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::merge::Item;
    use crate::search::EngineReport;
    use std::time::Duration;

    fn item(url: &str, title: &str, engines: &[&str]) -> Item {
        Item {
            url: url.into(),
            title: title.into(),
            content: "a snippet".into(),
            engines: engines.iter().map(|s| s.to_string()).collect(),
            positions: vec![1; engines.len()],
            score: 1.0,
        }
    }
    fn report(engine: &str, status: Status) -> EngineReport {
        EngineReport {
            engine: engine.into(),
            status,
            elapsed: Duration::from_millis(300),
        }
    }
    fn response(items: Vec<Item>, reports: Vec<EngineReport>) -> Response {
        Response {
            query: "rust lifetimes".into(),
            items,
            reports,
            suggestions: vec![],
            elapsed: Duration::from_millis(840),
        }
    }

    #[test]
    fn renders_rows_with_rank_domain_and_dots() {
        let r = response(
            vec![
                item(
                    "https://www.doc.rust-lang.org/book/ch10-03.html",
                    "Validating References",
                    &["bing", "duckduckgo", "wikipedia"],
                ),
                item("https://example.org/x", "Other", &["bing"]),
            ],
            vec![
                report("bing", Status::Ok(2)),
                report("duckduckgo", Status::Ok(1)),
                report("wikipedia", Status::Ok(1)),
                report("marginalia", Status::Empty),
            ],
        );
        let h = page(&r);
        assert!(h.contains("<title>rust lifetimes</title>"));
        assert!(h.contains(">2 results<") && h.contains("4 of 4 engines") && h.contains("0.84 s"));
        assert!(
            h.contains(
                "<span class=\"rank\">01</span><span class=\"dom\">doc.rust-lang.org</span>"
            ),
            "www. should be dropped"
        );
        assert!(h.contains("●●●○ 3"), "3 of 4 engines found it");
        assert!(h.contains("●○○○ 1"));
        assert!(
            h.contains("doc.rust-lang.org/book/ch10-03.html</div>"),
            "detail shows the address without the scheme"
        );
    }

    #[test]
    fn problems_show_in_the_status_line() {
        let r = response(
            vec![item("https://a.org/", "A", &["bing"])],
            vec![
                report("bing", Status::Ok(1)),
                report("duckduckgo", Status::Blocked("HTTP 429".into())),
                report("wikipedia", Status::Suspended(540)),
                report("marginalia", Status::Late),
            ],
        );
        let h = page(&r);
        assert!(h.contains(">duckduckgo blocked<") && h.contains("title=\"HTTP 429\""));
        assert!(h.contains(">wikipedia paused 9m<"));
        assert!(h.contains(">marginalia slow<"));
        assert!(h.contains("1 of 4 engines"));
    }

    #[test]
    fn hostile_content_is_escaped() {
        let mut it = item(
            "https://x.org/\"><script>alert(1)</script>",
            "<img src=x onerror=alert(1)>",
            &["<b>bing</b>"],
        );
        it.content = "<script>steal()</script>".into();
        let mut r = response(vec![it], vec![report("bing", Status::Ok(1))]);
        r.query = "</title><script>alert(2)</script>".into();
        r.suggestions = vec!["\"><script>alert(3)</script>".into()];
        let h = page(&r);
        // The only <script> in the page is ours, at the end.
        assert_eq!(h.matches("<script>").count(), 1, "{h}");
        assert!(!h.contains("<img src=x") && !h.contains("onerror=alert(1)>"));
        assert!(h.contains("&lt;script&gt;steal()"));
    }

    #[test]
    fn shift_and_ctrl_enter_open_a_new_tab() {
        let h = page(&response(
            vec![item("https://a.org/", "A", &["bing"])],
            vec![report("bing", Status::Ok(1))],
        ));
        assert!(h.contains("new tab"), "the hint is shown");
        // The mechanism: Shift/Ctrl+Enter marks the focused link target=_blank before the browser follows it.
        assert!(
            h.contains("e.key==='Enter'&&(e.shiftKey||e.ctrlKey)")
                && h.contains("a.target='_blank'")
        );
    }

    #[test]
    fn page_can_load_nothing() {
        let h = page(&response(
            vec![item("https://a.org/", "A", &["bing"])],
            vec![report("bing", Status::Ok(1))],
        ));
        assert!(h.contains("Content-Security-Policy") && h.contains("default-src 'none'"));
        assert!(
            !h.contains(" src=")
                && !h.contains("<link")
                && !h.contains("@import")
                && !h.contains("url("),
            "page references an external resource"
        );
        assert!(h.contains("rel=\"noreferrer\""));
    }

    #[test]
    fn empty_states_say_why() {
        let none_answered = page(&response(
            vec![],
            vec![
                report("bing", Status::Failed("timed out".into())),
                report("duckduckgo", Status::Failed("timed out".into())),
            ],
        ));
        assert!(none_answered.contains("No search engine answered"));
        let nothing = page(&response(vec![], vec![report("bing", Status::Empty)]));
        assert!(nothing.contains("Nothing found"));
        let blank = page(&Response::default());
        assert!(blank.contains("Type something") && blank.contains("<title>Bare Search</title>"));
    }

    #[test]
    fn did_you_mean_links_back_to_search() {
        let mut r = response(
            vec![item("https://a.org/", "A", &["bing"])],
            vec![report("bing", Status::Ok(1))],
        );
        r.suggestions = vec!["rush lifetime".into()];
        assert!(page(&r).contains("<a href=\"bare://search?q=rush%20lifetime\">rush lifetime</a>"));
    }
}
