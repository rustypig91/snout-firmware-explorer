use std::borrow::Cow;

/// Resolve all peer labels together, avoiding an all-pairs path scan in graphs.
pub(super) fn short_paths(paths: &[&str]) -> Vec<String> {
    use std::collections::{HashMap, HashSet};
    let normalized: Vec<_> = paths.iter().map(|p| p.replace('\\', "/")).collect();
    let distinct: HashSet<_> = normalized.iter().map(String::as_str).collect();
    let mut suffix_counts = HashMap::<&str, usize>::new();
    for path in distinct {
        *suffix_counts.entry(path).or_default() += 1;
        for (offset, _) in path.match_indices('/') {
            *suffix_counts.entry(&path[offset + 1..]).or_default() += 1;
        }
    }
    normalized
        .iter()
        .map(|path| {
            let parts: Vec<_> = path.split('/').filter(|p| !p.is_empty()).collect();
            for count in 2.min(parts.len())..=parts.len() {
                let suffix = parts[parts.len() - count..].join("/");
                let own_match =
                    usize::from(path == &suffix || path.ends_with(&format!("/{suffix}")));
                if suffix_counts.get(suffix.as_str()).copied().unwrap_or(0) <= own_match {
                    return suffix;
                }
            }
            path.clone()
        })
        .collect()
}

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

/// Display absolute paths relative to the build folder, without requiring the
/// source files to exist locally. Keep relative paths and non-path labels intact.
pub(super) fn build_relative_path(path: &str, build_root: Option<&std::path::Path>) -> String {
    use std::path::{Component, Path};

    let path = display_path(path);
    let Some(root) = build_root else {
        return path.into_owned();
    };
    let root = root.to_string_lossy();
    let root = display_path(&root);
    fn path_components(path: &Path) -> Vec<Component<'_>> {
        let mut parts = Vec::new();
        for part in path.components() {
            match part {
                Component::CurDir => {}
                Component::ParentDir if matches!(parts.last(), Some(Component::Normal(_))) => {
                    parts.pop();
                }
                _ => parts.push(part),
            }
        }
        parts
    }
    let source = Path::new(path.as_ref());
    let base = Path::new(root.as_ref());
    if !source.is_absolute() || !base.is_absolute() {
        return path.into_owned();
    }
    let source = path_components(source);
    let base = path_components(base);
    let common = source.iter().zip(&base).take_while(|(a, b)| a == b).count();
    // Different drive letters or UNC shares cannot be represented relatively.
    if common == 0
        || source[common..]
            .iter()
            .chain(&base[common..])
            .any(|part| matches!(part, Component::Prefix(_) | Component::RootDir))
    {
        return path.into_owned();
    }
    let mut relative = std::path::PathBuf::new();
    for _ in &base[common..] {
        relative.push("..");
    }
    for part in &source[common..] {
        relative.push(part.as_os_str());
    }
    if relative.as_os_str().is_empty() {
        ".".into()
    } else {
        relative.to_string_lossy().into_owned()
    }
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
    use super::{build_relative_path, display_path, short_path};

    #[test]
    fn batch_short_paths_match_individual_resolution() {
        let paths = [
            "C:/build/app/src/main.c",
            "C:/build/lib/src/main.c",
            "C:\\build\\app\\src\\main.c",
            "main.c",
            "src/main.c",
            "",
            "/",
            "/build/src/diag.c",
            "/build//src/other.c",
            "a/b/",
            "a///b",
        ];
        let expected: Vec<_> = paths.iter().map(|p| short_path(p, paths)).collect();
        assert_eq!(super::short_paths(&paths), expected);
    }

    #[test]
    fn paths_are_relative_to_build_folder_without_filesystem_access() {
        let root = std::env::temp_dir().join("snout-display/project/build");
        let inside = root.join("objects/main.c.su");
        let outside = root.parent().unwrap().join("src/main.c");
        let normalized = root.join("objects/../main.c.su");
        assert_eq!(
            build_relative_path(&inside.to_string_lossy(), Some(&root)),
            std::path::Path::new("objects")
                .join("main.c.su")
                .display()
                .to_string()
        );
        assert_eq!(
            build_relative_path(&outside.to_string_lossy(), Some(&root)),
            std::path::Path::new("..")
                .join("src")
                .join("main.c")
                .display()
                .to_string()
        );
        assert_eq!(
            build_relative_path(&normalized.to_string_lossy(), Some(&root)),
            "main.c.su"
        );
        for path in ["../src/main.c", "[unattributed]", ""] {
            assert_eq!(build_relative_path(path, Some(&root)), path);
        }
        assert_eq!(
            build_relative_path(&inside.to_string_lossy(), None),
            display_path(&inside.to_string_lossy())
        );
    }

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
