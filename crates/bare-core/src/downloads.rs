//! Downloads go straight to the downloads folder, named safely and never over an existing file.
//! The folder, the name (chosen by the *server*, so untrusted) and the "is this a download?" test
//! live here as pure functions.

use std::path::{Path, PathBuf};

/// Longest file name we create, in bytes (most filesystems allow 255; leave room for ` (n)`).
const MAX_NAME: usize = 200;

/// Turn a server-suggested name into a safe file name: no directories, no control characters,
/// no leading dots (a download must not be hidden or climb out of the folder), never empty.
pub fn sanitize_filename(suggested: &str) -> String {
    let base = suggested.rsplit(['/', '\\']).next().unwrap_or("");
    let cleaned: String = base
        .chars()
        .filter(|c| !c.is_control() && !matches!(c, '\0' | ':' | '*' | '?' | '"' | '<' | '>' | '|'))
        .collect();
    let cleaned = cleaned.trim().trim_start_matches('.').trim();
    if cleaned.is_empty() {
        return "download".to_string();
    }
    truncate_keeping_extension(cleaned, MAX_NAME)
}

fn truncate_keeping_extension(name: &str, max: usize) -> String {
    if name.len() <= max {
        return name.to_string();
    }
    let ext = name
        .rfind('.')
        .filter(|&i| name.len() - i <= 12)
        .map_or("", |i| &name[i..]);
    let stem_len = max - ext.len();
    let mut cut = stem_len;
    while !name.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{}{ext}", &name[..cut])
}

/// `dir/name`, or `dir/name (1).ext`, `dir/name (2).ext`… so an existing file is never replaced.
pub fn unique_path(dir: &Path, name: &str, exists: impl Fn(&Path) -> bool) -> PathBuf {
    let first = dir.join(name);
    if !exists(&first) {
        return first;
    }
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    (1..10_000)
        .map(|n| dir.join(format!("{stem} ({n}){ext}")))
        .find(|p| !exists(p))
        .unwrap_or_else(|| dir.join(format!("{stem} (many){ext}")))
}

/// A `Content-Disposition` value that asks for a download rather than display.
pub fn is_attachment(content_disposition: &str) -> bool {
    content_disposition
        .trim_start()
        .get(..10)
        .is_some_and(|p| p.eq_ignore_ascii_case("attachment"))
}

/// The downloads folder from `user-dirs.dirs` text (`XDG_DOWNLOAD_DIR="$HOME/Downloads"`).
pub fn download_dir_from_user_dirs(text: &str, home: &Path) -> Option<PathBuf> {
    let line = text
        .lines()
        .map(str::trim)
        .find(|l| l.starts_with("XDG_DOWNLOAD_DIR="))?;
    let value = line.split_once('=')?.1.trim().trim_matches('"');
    let path = match value.strip_prefix("$HOME") {
        Some(rest) => PathBuf::from(format!("{}{}", home.display(), rest)),
        None => PathBuf::from(value),
    };
    // A relative or empty value is garbage; don't write into the working directory.
    (path.is_absolute() && path != home).then_some(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn names_lose_directories_and_dots() {
        assert_eq!(sanitize_filename("report.pdf"), "report.pdf");
        assert_eq!(sanitize_filename("../../etc/passwd"), "passwd");
        assert_eq!(sanitize_filename("C:\\Users\\me\\evil.exe"), "evil.exe");
        assert_eq!(sanitize_filename(".bashrc"), "bashrc");
        assert_eq!(sanitize_filename("...hidden.txt"), "hidden.txt");
        assert_eq!(
            sanitize_filename("a/b/"),
            "download",
            "a trailing slash leaves no name"
        );
    }

    #[test]
    fn names_lose_control_and_awkward_characters() {
        assert_eq!(sanitize_filename("a\u{0}b\nc\td.txt"), "abcd.txt");
        assert_eq!(sanitize_filename("what?*.txt"), "what.txt");
        assert_eq!(sanitize_filename("  spaced name.zip  "), "spaced name.zip");
        assert_eq!(sanitize_filename(""), "download");
        assert_eq!(sanitize_filename("???"), "download");
    }

    #[test]
    fn long_names_are_cut_but_keep_their_extension() {
        let long = format!("{}.tar.gz", "x".repeat(400));
        let out = sanitize_filename(&long);
        assert!(out.len() <= MAX_NAME, "{}", out.len());
        assert!(out.ends_with(".gz"));
        let wide = "é".repeat(300);
        let out = sanitize_filename(&wide);
        assert!(
            out.len() <= MAX_NAME && out.chars().all(|c| c == 'é'),
            "cut on a char boundary"
        );
    }

    #[test]
    fn an_existing_file_is_never_replaced() {
        let taken: HashSet<PathBuf> = ["/d/a.txt", "/d/a (1).txt", "/d/noext", "/d/.dot"]
            .iter()
            .map(PathBuf::from)
            .collect();
        let exists = |p: &Path| taken.contains(p);
        let dir = Path::new("/d");
        assert_eq!(
            unique_path(dir, "fresh.txt", exists),
            PathBuf::from("/d/fresh.txt")
        );
        assert_eq!(
            unique_path(dir, "a.txt", exists),
            PathBuf::from("/d/a (2).txt")
        );
        assert_eq!(
            unique_path(dir, "noext", exists),
            PathBuf::from("/d/noext (1)")
        );
        assert_eq!(
            unique_path(dir, ".dot", exists),
            PathBuf::from("/d/.dot (1)")
        );
    }

    #[test]
    fn content_disposition() {
        assert!(is_attachment("attachment; filename=\"a.zip\""));
        assert!(is_attachment("  Attachment"));
        assert!(!is_attachment("inline; filename=a.png"));
        assert!(!is_attachment(""));
        assert!(!is_attachment("attach"));
    }

    #[test]
    fn user_dirs_file() {
        let home = Path::new("/home/u");
        let text = "# comment\nXDG_DESKTOP_DIR=\"$HOME/Desktop\"\nXDG_DOWNLOAD_DIR=\"$HOME/Téléchargements\"\n";
        assert_eq!(
            download_dir_from_user_dirs(text, home),
            Some(PathBuf::from("/home/u/Téléchargements"))
        );
        assert_eq!(
            download_dir_from_user_dirs("XDG_DOWNLOAD_DIR=\"/data/dl\"", home),
            Some(PathBuf::from("/data/dl"))
        );
        assert_eq!(
            download_dir_from_user_dirs("XDG_DOWNLOAD_DIR=\"$HOME/\"", home),
            None,
            "the home directory itself is not a downloads folder"
        );
        assert_eq!(
            download_dir_from_user_dirs("XDG_DOWNLOAD_DIR=\"relative\"", home),
            None
        );
        assert_eq!(download_dir_from_user_dirs("nothing here", home), None);
    }
}
