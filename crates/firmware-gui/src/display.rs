use std::borrow::Cow;

/// Keep at least a parent directory and extend the suffix to distinguish peers.
/// Treat both separators as paths, including debug paths from another OS.
pub(super) fn short_path<'a>(path: &str, peers: impl IntoIterator<Item = &'a str>) -> String {
    let normalized = path.replace('\\', "/");
    let parts: Vec<_> = normalized.split('/').filter(|p| !p.is_empty()).collect();
    let peers: Vec<_> = peers.into_iter().map(|p| p.replace('\\', "/")).collect();
    for count in 2.min(parts.len())..=parts.len() {
        let suffix = parts[parts.len() - count..].join("/");
        if peers
            .iter()
            .all(|p| p == &normalized || !(p == &suffix || p.ends_with(&format!("/{suffix}"))))
        {
            return suffix;
        }
    }
    normalized
}

/// Format a path for display without changing filesystem paths or selection keys.
#[cfg(windows)]
pub(super) fn display_path(path: &str) -> Cow<'_, str> {
    if let Some(unc) = path.strip_prefix(r"\\?\UNC\") {
        return Cow::Owned(format!(r"\\{unc}"));
    }
    if let Some(drive) = path.strip_prefix(r"\\?\") {
        let bytes = drive.as_bytes();
        if bytes.len() >= 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && bytes[2] == b'\\'
        {
            return Cow::Borrowed(drive);
        }
    }
    Cow::Borrowed(path)
}

#[cfg(not(windows))]
pub(super) fn display_path(path: &str) -> Cow<'_, str> {
    Cow::Borrowed(path)
}

#[cfg(test)]
mod tests {
    use super::{display_path, short_path};

    #[test]
    fn short_paths_preserve_distinct_sources_and_cross_platform_separators() {
        let paths = [
            "C:/build/app/src/main.c",
            "C:/build/lib/src/main.c",
            "/build/src/diag.c",
        ];
        assert_eq!(short_path(paths[0], paths), "app/src/main.c");
        assert_eq!(short_path(paths[1], paths), "lib/src/main.c");
        assert_eq!(short_path(paths[2], paths), "src/diag.c");
        assert_eq!(short_path(r"C:\build\src\main.c", []), "src/main.c");
        assert_eq!(short_path("main.c", []), "main.c");
        assert_eq!(short_path("", []), "");
    }

    #[test]
    #[cfg(windows)]
    fn formats_windows_paths() {
        assert_eq!(display_path(r"\\?\C:\build\app.map"), r"C:\build\app.map");
        assert_eq!(
            display_path(r"\\?\UNC\server\share\app.map"),
            r"\\server\share\app.map"
        );
        assert_eq!(
            display_path(r"Source: \\?\C:\app.map"),
            r"Source: \\?\C:\app.map"
        );
    }

    #[test]
    #[cfg(not(windows))]
    fn leaves_windows_paths_unchanged() {
        for path in [r"\\?\C:\build\app.map", r"\\?\UNC\server\share\app.map"] {
            assert_eq!(display_path(path), path);
        }
    }

    #[test]
    fn leaves_other_paths_unchanged() {
        for path in [
            r"C:\build\app.map",
            "/tmp/app.map",
            "relative/app.map",
            r"\\?\Volume{abc}\file",
        ] {
            assert_eq!(display_path(path), path);
        }
    }
}
