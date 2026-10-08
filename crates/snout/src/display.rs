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

/// Labels are resolved against the entire report, never just visible rows.
#[derive(Default)]
pub(super) struct SourcePaths {
    labels: std::collections::HashMap<String, String>,
    full: std::collections::HashMap<String, String>,
    tree: std::collections::HashMap<String, String>,
}

impl SourcePaths {
    pub(super) fn new(
        a: &snout_core::Analysis,
        stack: Option<&snout_core::stack::StackReport>,
        root: Option<&std::path::Path>,
    ) -> Self {
        let mut paths: Vec<&str> = a.files.iter().map(|f| f.path.as_str()).collect();
        for symbol in &a.symbols {
            paths.extend(symbol.source_file.as_deref());
            paths.extend(symbol.dwarf_compilation_unit.as_deref());
            paths.extend(symbol.compilation_unit.as_deref());
        }
        paths.extend(a.dependencies.nodes.iter().map(|n| n.label.as_str()));
        if let Some(stack) = stack {
            paths.extend(stack.entries.iter().map(|e| e.source_file.as_str()));
        }
        paths.sort_unstable();
        paths.dedup();
        let mut full: std::collections::HashMap<_, _> = paths
            .iter()
            .map(|p| (p.to_string(), display_path(p).into_owned()))
            .collect();
        let sources: std::collections::HashSet<_> = a
            .symbols
            .iter()
            .filter_map(|s| s.source_file.as_deref())
            .collect();
        // File rows also contain STT_FILE labels, even labels with separators.
        // Resolve only locations backed by source or DWARF unit evidence.
        let known_paths: std::collections::HashSet<_> = a
            .symbols
            .iter()
            .flat_map(|s| {
                [
                    s.source_file.as_deref(),
                    s.dwarf_compilation_unit.as_deref(),
                ]
            })
            .flatten()
            .collect();
        for path in &known_paths {
            full.insert((*path).into(), absolute_path(path, root));
        }
        let normalized_known: std::collections::HashSet<_> = known_paths
            .iter()
            .map(|path| path.replace('\\', "/"))
            .collect();
        // Dependencies and comparisons also use slash-normalized aliases of
        // DWARF paths. Resolve every recorded alias with the same evidence.
        for path in &paths {
            if normalized_known.contains(&path.replace('\\', "/")) {
                full.insert((*path).to_owned(), absolute_path(path, root));
            }
        }
        if let Some(stack) = stack {
            let mut suffixes = std::collections::HashMap::<String, Option<String>>::new();
            for source in &sources {
                let normalized = absolute_path(source, None);
                let canonical = &full[*source];
                for suffix in std::iter::once(normalized.as_str()).chain(
                    normalized
                        .match_indices('/')
                        .map(|(i, _)| &normalized[i + 1..]),
                ) {
                    suffixes
                        .entry(suffix.into())
                        .and_modify(|existing| {
                            if existing.as_ref() != Some(canonical) {
                                *existing = None;
                            }
                        })
                        .or_insert_with(|| Some(canonical.clone()));
                }
            }
            for entry in &stack.entries {
                // Prefer a unique DWARF path over guessing the compiler cwd.
                let suffix = absolute_path(&entry.source_file, None);
                let source = suffixes
                    .get(&suffix)
                    .and_then(|source| source.as_deref())
                    .unwrap_or(&entry.source_file);
                full.insert(entry.source_file.clone(), absolute_path(source, root));
            }
        }
        let canonical: Vec<_> = paths.iter().map(|p| full[*p].as_str()).collect();
        let mut labels: std::collections::HashMap<String, String> = paths
            .iter()
            .copied()
            .zip(short_paths(&canonical))
            .map(|(p, label)| (p.to_owned(), label))
            .collect();
        for (path, canonical) in &full {
            labels.insert(canonical.clone(), labels[path].clone());
        }
        // The analysis tree drops leading separators and `.` components.
        // Preserve the original lookup for UNC paths and relative source paths.
        let mut tree = std::collections::HashMap::new();
        for file in &a.files {
            let key = file
                .path
                .replace('\\', "/")
                .split('/')
                .filter(|part| !part.is_empty() && *part != ".")
                .collect::<Vec<_>>()
                .join("/");
            tree.entry(key).or_insert_with(|| full[&file.path].clone());
        }
        Self { labels, full, tree }
    }

    pub(super) fn short(&self, path: &str) -> String {
        self.labels
            .get(path)
            .cloned()
            .unwrap_or_else(|| short_path(path, self.labels.keys().map(String::as_str)))
    }

    pub(super) fn tree_full(&self, path: &str) -> String {
        self.tree
            .get(path)
            .or_else(|| self.full.get(path))
            .or_else(|| self.full.get(&format!("/{path}")))
            .cloned()
            .unwrap_or_else(|| path.into())
    }

    pub(super) fn short_detail(&self, text: &str) -> String {
        text.lines()
            .map(|line| {
                let (prefix, path) = ["Source: ", "Compilation unit: "]
                    .iter()
                    .find_map(|prefix| line.strip_prefix(prefix).map(|path| (*prefix, path)))
                    .unwrap_or(("", line));
                if let Some(short) = self.labels.get(path) {
                    return format!("{prefix}{short}");
                }
                if prefix == "Source: " {
                    if let Some((path, number)) = path.rsplit_once(':') {
                        if number == "?" || number.parse::<u32>().is_ok() {
                            if let Some(short) = self.labels.get(path) {
                                return format!("{prefix}{short}:{number}");
                            }
                        }
                    }
                }
                line.to_owned()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub(super) fn full(&self, path: &str) -> String {
        self.full
            .get(path)
            .cloned()
            .unwrap_or_else(|| display_path(path).into_owned())
    }
}

/// Lexical resolution also supports paths recorded on a different OS.
pub(super) fn absolute_path(path: &str, root: Option<&std::path::Path>) -> String {
    if path.is_empty() || path.starts_with('[') || path == "Unknown" || path == "Unknown source" {
        return path.into();
    }
    // Windows canonicalize adds a verbatim prefix to build roots. Normalize
    // both inputs before joining, including reports opened on a different OS.
    fn normalized(path: &str) -> String {
        let path = path.replace('\\', "/");
        if let Some(unc) = path.strip_prefix("//?/UNC/") {
            return format!("//{unc}");
        }
        if let Some(drive) = path.strip_prefix("//?/") {
            let bytes = drive.as_bytes();
            if bytes.len() >= 3
                && bytes[0].is_ascii_alphabetic()
                && bytes[1] == b':'
                && bytes[2] == b'/'
            {
                return drive.into();
            }
        }
        path
    }
    let path = normalized(path);
    let absolute = path.starts_with('/')
        || (path.as_bytes().get(1) == Some(&b':') && path.as_bytes().get(2) == Some(&b'/'));
    let joined = if absolute {
        path
    } else if let Some(root) = root {
        format!(
            "{}/{}",
            normalized(&root.to_string_lossy()).trim_end_matches('/'),
            path
        )
    } else {
        path
    };
    let (prefix, rest, protected, rooted) = if joined.starts_with("//") {
        ("//", joined.trim_start_matches('/'), 2, true)
    } else if joined.starts_with('/') {
        ("/", joined.trim_start_matches('/'), 0, true)
    } else if joined.as_bytes().get(1) == Some(&b':') && joined.as_bytes().get(2) == Some(&b'/') {
        (&joined[..3], &joined[3..], 0, true)
    } else {
        ("", joined.as_str(), 0, false)
    };
    let mut parts = Vec::new();
    for part in rest.split('/') {
        match part {
            "" | "." => {}
            ".." if parts.len() > protected && parts.last() != Some(&"..") => {
                parts.pop();
            }
            ".." if rooted => {}
            _ => parts.push(part),
        }
    }
    format!("{prefix}{}", parts.join("/"))
}

struct CachedSourcePaths {
    revision: u64,
    source: usize,
    stack: usize,
    root: Option<std::path::PathBuf>,
    paths: std::rc::Rc<SourcePaths>,
}

#[derive(Default)]
pub(super) struct SourcePathCache(std::cell::RefCell<Option<CachedSourcePaths>>);

impl super::Explorer {
    pub(super) fn source_paths(&self, a: &snout_core::Analysis) -> std::rc::Rc<SourcePaths> {
        let source = a as *const _ as usize;
        let display = self
            .baseline_display
            .as_ref()
            .filter(|d| std::ptr::eq(a, d.analysis.as_ref()));
        let stack_report = display
            .and_then(|d| d.stack.as_deref())
            .or(self.stack.as_ref());
        let stack = stack_report.map_or(0, |s| s as *const _ as usize);
        let root = self.build.as_ref().map(|b| b.root.clone());
        let mut cache = self.source_path_cache.0.borrow_mut();
        if let Some(old) = &*cache {
            if old.revision == self.report_revision
                && old.source == source
                && old.stack == stack
                && old.root == root
            {
                return old.paths.clone();
            }
        }
        let paths = std::rc::Rc::new(SourcePaths::new(a, stack_report, root.as_deref()));
        *cache = Some(CachedSourcePaths {
            revision: self.report_revision,
            source,
            stack,
            root,
            paths: paths.clone(),
        });
        paths
    }
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
    fn absolute_tooltips_resolve_relative_and_foreign_paths_lexically() {
        use super::absolute_path;
        use std::path::Path;
        assert_eq!(
            absolute_path("../src/main.c", Some(Path::new("/project/build"))),
            "/project/src/main.c"
        );
        assert_eq!(
            absolute_path(r"C:\project\src\main.c", Some(Path::new("/local/build"))),
            "C:/project/src/main.c"
        );
        assert_eq!(
            absolute_path(r"..\src\main.c", Some(Path::new("C:/project/build"))),
            "C:/project/src/main.c"
        );
        assert_eq!(
            absolute_path("//server/share/../../src/main.c", None),
            "//server/share/src/main.c"
        );
        assert_eq!(
            absolute_path("[unattributed]", Some(Path::new("/build"))),
            "[unattributed]"
        );
        assert_eq!(absolute_path("src/main.c", None), "src/main.c");
        assert_eq!(
            absolute_path("./generated/../src//main.c", None),
            "src/main.c"
        );
        assert_eq!(absolute_path("../../src/main.c", None), "../../src/main.c");
    }

    #[test]
    fn absolute_tooltips_normalize_windows_verbatim_sources_and_build_roots() {
        use super::absolute_path;
        use std::path::Path;
        for root in [r"C:\project\build", r"\\?\C:\project\build"] {
            assert_eq!(
                absolute_path(r"..\src\main.c", Some(Path::new(root))),
                "C:/project/src/main.c"
            );
        }
        for root in [r"\\server\share\build", r"\\?\UNC\server\share\build"] {
            assert_eq!(
                absolute_path(r"..\..\src\main.c", Some(Path::new(root))),
                "//server/share/src/main.c"
            );
        }
        assert_eq!(
            absolute_path(r"\\?\C:\project\src\main.c", None),
            "C:/project/src/main.c"
        );
        assert_eq!(
            absolute_path(r"\\?\UNC\server\share\src\main.c", None),
            "//server/share/src/main.c"
        );
    }

    #[test]
    fn normalized_dependency_units_share_dwarf_labels_and_tooltips() {
        let mut a = snout_core::analyze_bytes(
            include_bytes!("../../../fixtures/build/cortex-m.elf"),
            "fixture.elf",
            &Default::default(),
        )
        .unwrap();
        let mut symbol = a.symbols[0].clone();
        symbol.source_file = Some("include/header.h".into());
        symbol.dwarf_compilation_unit = Some(r"src\unit.c".into());
        symbol.compilation_unit = None;
        a.symbols = vec![symbol];
        a.files.clear();
        a.dependencies.nodes = vec![snout_core::dependencies::DependencyNode {
            id: "dwarf:src/unit.c".into(),
            label: "src/unit.c".into(),
            evidence: "DWARF compilation unit".into(),
            usage: None,
            objects: vec![],
        }];
        let paths = super::SourcePaths::new(&a, None, Some(std::path::Path::new("/build")));
        assert_eq!(paths.full("src/unit.c"), "/build/src/unit.c");
        assert_eq!(paths.short(r"src\unit.c"), "src/unit.c");
        assert_eq!(paths.short("src/unit.c"), "src/unit.c");
    }

    #[test]
    fn equivalent_dwarf_source_spellings_do_not_make_stack_aliases_ambiguous() {
        let mut a = snout_core::analyze_bytes(
            include_bytes!("../../../fixtures/build/cortex-m.elf"),
            "fixture.elf",
            &Default::default(),
        )
        .unwrap();
        let mut symbol = a.symbols[0].clone();
        symbol.source_file = Some("/project/generated/../src/main.c".into());
        symbol.dwarf_compilation_unit = None;
        symbol.compilation_unit = None;
        let mut alias = symbol.clone();
        alias.source_file = Some(r"\project\src\main.c".into());
        let mut relative = symbol.clone();
        relative.source_file = Some("../src/main.c".into());
        a.symbols = vec![symbol, alias, relative];
        a.files.clear();
        a.dependencies.nodes.clear();
        let (entries, _) = snout_core::stack::parse_stack_usage(
            "src/main.c:12:1:probe\t8\tstatic\n./src/main.c:13:1:probe_dot\t8\tstatic\ngenerated/../src/main.c:14:1:probe_parent\t8\tstatic\nsrc//main.c:15:1:probe_separator\t8\tstatic\n",
            "probe.su",
        );
        let stack = snout_core::stack::StackReport {
            schema_version: 1,
            entries,
            warnings: vec![],
            call_graph: Default::default(),
        };
        let paths = super::SourcePaths::new(
            &a,
            Some(&stack),
            Some(std::path::Path::new("/project/build")),
        );
        assert_eq!(paths.full("src/main.c"), "/project/src/main.c");
        for entry in &stack.entries {
            assert_eq!(paths.full(&entry.source_file), "/project/src/main.c");
            assert_eq!(paths.short(&entry.source_file), "src/main.c");
        }
        assert_eq!(
            paths.short("src/main.c"),
            paths.short("/project/src/main.c")
        );

        // Distinct sources with the same suffix still require the build-folder
        // fallback rather than choosing one of the DWARF locations.
        a.symbols[2].source_file = Some("/other/src/main.c".into());
        let paths = super::SourcePaths::new(
            &a,
            Some(&stack),
            Some(std::path::Path::new("/project/build")),
        );
        assert_eq!(paths.full("src/main.c"), "/project/build/src/main.c");
        for entry in &stack.entries {
            assert_eq!(paths.full(&entry.source_file), "/project/build/src/main.c");
        }
    }

    #[test]
    fn report_labels_distinguish_files_and_share_stack_aliases() {
        let mut a = snout_core::analyze_bytes(
            include_bytes!("../../../fixtures/build/cortex-m.elf"),
            "fixture.elf",
            &Default::default(),
        )
        .unwrap();
        let mut first = a.symbols[0].clone();
        first.source_file = Some("/project/app/src/main.c".into());
        first.dwarf_compilation_unit = first.source_file.clone();
        first.compilation_unit = Some("main.c".into());
        let mut second = first.clone();
        second.source_file = Some("/project/lib/src/main.c".into());
        second.dwarf_compilation_unit = second.source_file.clone();
        a.symbols = vec![first, second];
        a.files.clear();
        a.dependencies.nodes.clear();
        let (entries, _) = snout_core::stack::parse_stack_usage(
            "app/src/main.c:12:1:probe\t8\tstatic\n",
            "probe.su",
        );
        let stack = snout_core::stack::StackReport {
            schema_version: 1,
            entries,
            warnings: vec![],
            call_graph: Default::default(),
        };
        let paths =
            super::SourcePaths::new(&a, Some(&stack), Some(std::path::Path::new("/project")));
        assert_eq!(paths.short("/project/app/src/main.c"), "app/src/main.c");
        assert_eq!(paths.short("/project/lib/src/main.c"), "lib/src/main.c");
        assert_eq!(paths.short("app/src/main.c"), "app/src/main.c");
        assert_eq!(paths.full("app/src/main.c"), "/project/app/src/main.c");
        assert_eq!(paths.full("main.c"), "main.c");
        assert_eq!(
            paths.tree_full("project/app/src/main.c"),
            "/project/app/src/main.c"
        );
        assert_eq!(
            paths.short_detail(
                "Source: /project/app/src/main.c:12\nCompilation unit: /project/lib/src/main.c"
            ),
            "Source: app/src/main.c:12\nCompilation unit: lib/src/main.c"
        );
        assert_eq!(paths.short("/project/app/src/main.c"), "app/src/main.c");
    }

    #[test]
    fn elf_labels_with_directories_are_not_resolved_as_source_locations() {
        let mut a = snout_core::analyze_bytes(
            include_bytes!("../../../fixtures/build/cortex-m.elf"),
            "fixture.elf",
            &Default::default(),
        )
        .unwrap();
        let mut symbol = a.symbols[0].clone();
        symbol.source_file = None;
        symbol.dwarf_compilation_unit = None;
        symbol.compilation_unit = Some("generated/startup.c".into());
        a.symbols = vec![symbol];
        a.files = vec![snout_core::FileUsage {
            path: "generated/startup.c".into(),
            attribution: "ELF compilation-unit label (not an object path)".into(),
            usage: Default::default(),
            symbol_count: 1,
        }];
        a.dependencies.nodes.clear();
        let root = Some(std::path::Path::new("/project/build"));
        let paths = super::SourcePaths::new(&a, None, root);
        assert_eq!(paths.full("generated/startup.c"), "generated/startup.c");

        // DWARF evidence permits resolving even a bare filename and the
        // slash-normalized file row derived from a Windows source path.
        a.symbols[0].dwarf_compilation_unit = Some("unit.c".into());
        a.symbols[0].source_file = Some(r"src\main.c".into());
        a.files[0].path = "src/main.c".into();
        let paths = super::SourcePaths::new(&a, None, root);
        assert_eq!(paths.full("unit.c"), "/project/build/unit.c");
        assert_eq!(paths.full(r"src\main.c"), "/project/build/src/main.c");
        assert_eq!(paths.full("src/main.c"), "/project/build/src/main.c");

        a.symbols[0].source_file = Some("./src/main.c".into());
        a.files[0].path = "./src/main.c".into();
        a.files.push(snout_core::FileUsage {
            path: "//server/share/src/remote.c".into(),
            attribution: "DWARF".into(),
            usage: Default::default(),
            symbol_count: 1,
        });
        let paths = super::SourcePaths::new(&a, None, root);
        assert_eq!(paths.tree_full("src/main.c"), "/project/build/src/main.c");
        assert_eq!(
            paths.tree_full("server/share/src/remote.c"),
            "//server/share/src/remote.c"
        );
    }

    #[test]
    fn baseline_only_stack_sources_share_the_display_report_labels() {
        let mut old = snout_core::analyze_bytes(
            include_bytes!("../../../fixtures/build/cortex-m.elf"),
            "fixture.elf",
            &Default::default(),
        )
        .unwrap();
        let mut symbol = old.symbols[0].clone();
        symbol.source_file = Some("/project/legacy/src/main.c".into());
        symbol.dwarf_compilation_unit = symbol.source_file.clone();
        symbol.compilation_unit = None;
        old.symbols = vec![symbol];
        old.files.clear();
        old.dependencies.nodes.clear();
        let mut current = old.clone();
        current.symbols.clear();
        let (entries, _) = snout_core::stack::parse_stack_usage(
            "legacy/src/main.c:12:1:probe\t8\tstatic\n",
            "old.su",
        );
        let old_stack = snout_core::stack::StackReport {
            schema_version: 1,
            entries,
            warnings: vec![],
            call_graph: Default::default(),
        };
        let app = super::super::Explorer {
            baseline_display: Some(super::super::baseline_display::BaselineDisplay::new(
                &current,
                &old,
                None,
                Some(&old_stack),
            )),
            ..Default::default()
        };
        let display = app.baseline_display_analysis().unwrap();
        let paths = app.source_paths(&display);
        assert_eq!(
            paths.full("legacy/src/main.c"),
            "/project/legacy/src/main.c"
        );
        assert_eq!(paths.short("legacy/src/main.c"), "src/main.c");
        assert_eq!(paths.short("/project/legacy/src/main.c"), "src/main.c");
        assert!(std::rc::Rc::ptr_eq(&paths, &app.source_paths(&display)));
    }

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
