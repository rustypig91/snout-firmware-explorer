use std::borrow::Cow;

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
    use super::display_path;

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
