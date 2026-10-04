//! Window size memory. GTK4 can't place windows (the WM does), so only size and the
//! maximized flag are remembered. Plain `key=value` lines so it is trivially hand-editable.

use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowState {
    pub width: i32,
    pub height: i32,
    pub maximized: bool,
}

impl Default for WindowState {
    fn default() -> Self {
        Self {
            width: 1100,
            height: 760,
            maximized: false,
        }
    }
}

const MIN: (i32, i32) = (400, 300);
const MAX: i32 = 16384;

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
                _ => {}
            }
        }
        s.width = s.width.clamp(MIN.0, MAX);
        s.height = s.height.clamp(MIN.1, MAX);
        s
    }

    pub fn to_text(&self) -> String {
        format!(
            "width={}\nheight={}\nmaximized={}\n",
            self.width, self.height, self.maximized
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
        };
        assert_eq!(WindowState::parse(&s.to_text()), s);
    }

    #[test]
    fn garbage_and_extremes_are_tamed() {
        let s = WindowState::parse("width=abc\nheight=5\nmaximized=yes\nwat\nwidth2=7\n");
        assert_eq!(
            s,
            WindowState {
                width: 1100,
                height: 300,
                maximized: false
            }
        );
        assert_eq!(WindowState::parse("width=999999").width, 16384);
    }

    #[test]
    fn missing_file_is_default() {
        assert_eq!(
            WindowState::load(Path::new("/nonexistent/window")),
            WindowState::default()
        );
    }
}
