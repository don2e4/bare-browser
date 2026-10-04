//! Where Bare keeps its files. `BARE_HOME=<dir>` puts everything under one directory
//! (`config/`, `data/`, `cache/`), which is how tests avoid touching a real profile.

use std::ffi::OsString;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    pub config: PathBuf,
    pub data: PathBuf,
    pub cache: PathBuf,
    /// Where downloads are saved.
    pub downloads: PathBuf,
}

impl Paths {
    pub fn from_env() -> Self {
        let mut paths = Self::resolve(|k| std::env::var_os(k));
        // The user's chosen downloads folder lives in ~/.config/user-dirs.dirs (not in the environment).
        if std::env::var_os("BARE_HOME").is_none()
            && std::env::var_os("XDG_DOWNLOAD_DIR").is_none()
            && let Some(home) = std::env::var_os("HOME").map(PathBuf::from)
            && let Ok(text) = std::fs::read_to_string(home.join(".config/user-dirs.dirs"))
            && let Some(dir) = crate::downloads::download_dir_from_user_dirs(&text, &home)
        {
            paths.downloads = dir;
        }
        paths
    }

    pub fn resolve(env: impl Fn(&str) -> Option<OsString>) -> Self {
        let non_empty = |k: &str| env(k).filter(|v| !v.is_empty()).map(PathBuf::from);
        if let Some(root) = non_empty("BARE_HOME") {
            return Self {
                config: root.join("config"),
                data: root.join("data"),
                cache: root.join("cache"),
                downloads: root.join("downloads"),
            };
        }
        let home = non_empty("HOME").unwrap_or_else(|| PathBuf::from("/tmp"));
        let xdg = |var: &str, fallback: &str| {
            non_empty(var)
                .unwrap_or_else(|| home.join(fallback))
                .join("bare")
        };
        Self {
            config: xdg("XDG_CONFIG_HOME", ".config"),
            data: xdg("XDG_DATA_HOME", ".local/share"),
            cache: xdg("XDG_CACHE_HOME", ".cache"),
            downloads: non_empty("XDG_DOWNLOAD_DIR").unwrap_or_else(|| home.join("Downloads")),
        }
    }

    pub fn config_file(&self) -> PathBuf {
        self.config.join("config.toml")
    }

    pub fn bookmarks_file(&self) -> PathBuf {
        self.config.join("bookmarks.txt")
    }

    pub fn history_file(&self) -> PathBuf {
        self.data.join("history.sqlite")
    }

    /// The converted filter lists downloaded by `bare --update-filters` (absent = built-in baseline).
    pub fn filters_file(&self) -> PathBuf {
        self.data.join("filters.json")
    }

    /// Written whenever Bare tries to fetch the lists, so a failed attempt isn't repeated at once.
    pub fn filters_attempt_file(&self) -> PathBuf {
        self.data.join("filters.checked")
    }

    /// Fingerprint of `filters_file`, so startup needn't read and hash a 13 MB file.
    pub fn filters_id_file(&self) -> PathBuf {
        self.data.join("filters.id")
    }

    pub fn window_file(&self) -> PathBuf {
        self.config.join("window")
    }

    /// Create the directories (best effort; callers cope with failure by running without persistence).
    pub fn ensure(&self) {
        for d in [&self.config, &self.data, &self.cache] {
            if let Err(e) = std::fs::create_dir_all(d) {
                eprintln!("bare: cannot create {}: {e}", d.display());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> {
        let m: HashMap<String, OsString> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), OsString::from(v)))
            .collect();
        move |k| m.get(k).cloned()
    }

    #[test]
    fn xdg_defaults_from_home() {
        let p = Paths::resolve(env(&[("HOME", "/home/u")]));
        assert_eq!(p.config, PathBuf::from("/home/u/.config/bare"));
        assert_eq!(p.data, PathBuf::from("/home/u/.local/share/bare"));
        assert_eq!(p.cache, PathBuf::from("/home/u/.cache/bare"));
        assert_eq!(p.downloads, PathBuf::from("/home/u/Downloads"));
    }

    #[test]
    fn xdg_vars_win() {
        let p = Paths::resolve(env(&[
            ("HOME", "/home/u"),
            ("XDG_CONFIG_HOME", "/c"),
            ("XDG_CACHE_HOME", ""),
        ]));
        assert_eq!(p.config, PathBuf::from("/c/bare"));
        assert_eq!(p.cache, PathBuf::from("/home/u/.cache/bare"));
    }

    #[test]
    fn bare_home_overrides_everything() {
        let p = Paths::resolve(env(&[("HOME", "/home/u"), ("BARE_HOME", "/t")]));
        assert_eq!(p.config_file(), PathBuf::from("/t/config/config.toml"));
        assert_eq!(p.window_file(), PathBuf::from("/t/config/window"));
        assert_eq!(p.data, PathBuf::from("/t/data"));
        assert_eq!(
            p.downloads,
            PathBuf::from("/t/downloads"),
            "tests must not write into the real Downloads"
        );
    }
}
