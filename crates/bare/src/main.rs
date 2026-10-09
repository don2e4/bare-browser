mod dropdown;
mod filters;
mod findbar;
mod permissions;
mod sidebar;
mod tabs;
mod update;
mod urlbar;
mod web;
mod window;
mod wm;

use bare_core::{Config, Paths, Target};
use gtk4::{gio, glib, prelude::*};
use std::{cell::RefCell, path::Path, rc::Rc};

/// Debug logging for the headless tests (`BARE_LOG=1`). A macro, so that when logging is off the
/// message isn't even formatted: some of these sit on per-keystroke paths.
macro_rules! log {
    ($($arg:tt)*) => {
        if $crate::logging() {
            eprintln!("bare: {}", format_args!($($arg)*));
        }
    };
}
pub(crate) use log;

pub(crate) fn logging() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("BARE_LOG").is_some())
}

/// When the process started, for the start-up time in the log.
pub(crate) static STARTED: std::sync::LazyLock<std::time::Instant> =
    std::sync::LazyLock::new(std::time::Instant::now);

pub(crate) const APP_ID: &str = "app.bare.Browser";
const APP_ID_C: &std::ffi::CStr = c"app.bare.Browser";

const HELP: &str = "\
bare - a web browser with no title bar

usage: bare [address | search words | file]

  bare example.com
  bare rust lifetimes
  bare ./page.html

  bare --update-filters   download and convert the full ad/tracker lists (EasyList, EasyPrivacy)
                          now (Bare also does this by itself when they are missing or a week old)

environment:
  BARE_HOME=<dir>         keep config, data and cache under <dir> (for testing)
  BARE_APP_ID=<id>        use a separate single-instance identity (for testing)
  BARE_FILTER_UPDATES=0|1 override `filter_updates` in config.toml (for testing)
  BARE_FILTER_LISTS=<url>,<url>  fetch these lists instead of EasyList and EasyPrivacy (for testing)
";

fn main() -> glib::ExitCode {
    std::sync::LazyLock::force(&STARTED);
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "-h" || a == "--help") {
        print!("{HELP}");
        return glib::ExitCode::SUCCESS;
    }
    if args.iter().any(|a| a == "-V" || a == "--version") {
        println!("bare {}", env!("CARGO_PKG_VERSION"));
        return glib::ExitCode::SUCCESS;
    }

    let paths = Paths::from_env();
    paths.ensure();
    if args.iter().any(|a| a == "--update-filters") {
        return match update::run(&paths) {
            Ok(()) => glib::ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("bare: {e}");
                glib::ExitCode::FAILURE
            }
        };
    }
    let mut config = Config::load(&paths.config_file());
    if let Some(v) = std::env::var_os("BARE_FILTER_UPDATES") {
        config.filter_updates = v != "0";
    }

    let app_id = std::env::var("BARE_APP_ID").unwrap_or_else(|_| APP_ID.to_string());
    let app = gtk4::Application::builder()
        .application_id(app_id)
        .flags(gio::ApplicationFlags::HANDLES_COMMAND_LINE)
        .build();

    // The first invocation builds the window; later ones (`bare some.url` while it runs) reuse it.
    let browser: RefCell<Option<Rc<window::Browser>>> = RefCell::new(None);
    app.connect_command_line(move |app, cmdline| {
        let args: Vec<String> = cmdline
            .arguments()
            .iter()
            .skip(1)
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        let start = start_targets(&args, cmdline.cwd().as_deref());
        let mut slot = browser.borrow_mut();
        let browser = slot.get_or_insert_with(|| window::Browser::new(app, &paths, &config));
        browser.show(start);
        glib::ExitCode::SUCCESS
    });
    // GtkApplication's start-up asks the icon theme whether it has an icon named after the app, and
    // that question waits for the whole theme to load: 190 ms with a big one like Papirus, all before
    // the window exists. It isn't asked when a default icon is already named, so name one here (GTK
    // isn't initialized yet, which the safe binding insists on; this only stores the name). The
    // window gets its icon once it is on screen, by when the theme has loaded in the background.
    // SAFETY: a NUL-terminated string that GTK copies.
    unsafe { gtk4::ffi::gtk_window_set_default_icon_name(APP_ID_C.as_ptr()) };
    app.run()
}

/// What the command line asks to open (see `bare_core::input::targets_from_args`); an argument that
/// names an existing file, relative to where `bare` was run, opens that file.
fn start_targets(args: &[String], cwd: Option<&Path>) -> Vec<Target> {
    bare_core::input::targets_from_args(args, |arg| {
        if arg.contains("://") {
            return None;
        }
        let path = cwd.map_or_else(|| Path::new(arg).to_path_buf(), |c| c.join(arg));
        path.canonicalize().ok()?.to_str().map(str::to_string)
    })
}
