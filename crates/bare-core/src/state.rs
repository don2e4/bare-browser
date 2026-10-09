//! Window size memory. GTK4 can't place windows (the WM does), so only size and the
//! maximized flag are remembered, plus whether the tab sidebar is shown and how wide it is. Plain
//! `key=value` lines so it is trivially hand-editable.

use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowState {
    pub width: i32,
    pub height: i32,
    pub maximized: bool,
    /// The tab tree on the left (F1 toggles it).
    pub sidebar: bool,
    pub sidebar_width: i32,
}

impl Default for WindowState {
    fn default() -> Self {
        Self {
            width: 1100,
            height: 760,
            maximized: false,
            sidebar: true,
            sidebar_width: 240,
        }
    }
}

const MIN: (i32, i32) = (400, 300);
const MAX: i32 = 16384;
const SIDEBAR_WIDTH: (i32, i32) = (120, 800);

impl WindowState {
    pub fn parse(text: &str) -> Self {
        let mut s = Self::default();
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else {
                continue;
            };
            let v = v.trim();
            match k.trim() {
                "width" => s.width = v.parse().unwrap_or(s.width),
                "height" => s.height = v.parse().unwrap_or(s.height),
                "maximized" => s.maximized = v == "true",
                "sidebar" => s.sidebar = v != "false",
                "sidebar_width" => s.sidebar_width = v.parse().unwrap_or(s.sidebar_width),
                _ => {}
            }
        }
        s.width = s.width.clamp(MIN.0, MAX);
        s.height = s.height.clamp(MIN.1, MAX);
        s.sidebar_width = s.sidebar_width.clamp(SIDEBAR_WIDTH.0, SIDEBAR_WIDTH.1);
        s
    }

    pub fn to_text(&self) -> String {
        format!(
            "width={}\nheight={}\nmaximized={}\nsidebar={}\nsidebar_width={}\n",
            self.width, self.height, self.maximized, self.sidebar, self.sidebar_width
        )
    }

    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .map(|t| Self::parse(&t))
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) {
        if let Err(e) = std::fs::write(path, self.to_text()) {
            eprintln!("bare: cannot save {}: {e}", path.display());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let s = WindowState {
            width: 900,
            height: 600,
            maximized: true,
            sidebar: false,
            sidebar_width: 300,
        };
        assert_eq!(WindowState::parse(&s.to_text()), s);
    }

    #[test]
    fn garbage_and_extremes_are_tamed() {
        let s = WindowState::parse(
            "width=abc\nheight=5\nmaximized=yes\nwat\nwidth2=7\nsidebar=no\nsidebar_width=5\n",
        );
        assert_eq!(
            s,
            WindowState {
                width: 1100,
                height: 300,
                maximized: false,
                sidebar: true,
                sidebar_width: 120,
            }
        );
        assert_eq!(WindowState::parse("width=999999").width, 16384);
        assert!(!WindowState::parse("sidebar=false").sidebar);
    }

    #[test]
    fn missing_file_is_default() {
        assert_eq!(
            WindowState::load(Path::new("/nonexistent/window")),
            WindowState::default()
        );
    }
}
