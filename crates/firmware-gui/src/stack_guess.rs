use super::{workspace::StackSelection, Analysis, LoadedStack};
use firmware_analysis_core::{
    build::{ArtifactKind, BuildFolder},
    stack::analyze_stack_files,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

/// Prefer build provenance over function names, which can be shared by multiple targets.
pub(super) fn guess(analysis: &Analysis, build: &BuildFolder) -> Result<LoadedStack, String> {
    let paths: Vec<_> = build
        .artifacts
        .iter()
        .filter(|a| a.kind == ArtifactKind::StackUsage)
        .map(|a| a.path.clone())
        .collect();
    let mut objects: BTreeSet<_> = analysis
        .dependencies
        .nodes
        .iter()
        .flat_map(|n| &n.objects)
        .filter(|object| !object.contains('('))
        .map(|object| PathBuf::from(object.replace('\\', "/")).with_extension("su"))
        .collect();
    let firmware = Path::new(&analysis.path);
    let map_path = analysis
        .dependencies
        .map_path
        .as_deref()
        .map(Path::new)
        .or_else(|| build.matching_map(firmware));
    if let Some(text) = map_path.and_then(|path| std::fs::read_to_string(path).ok()) {
        objects.extend(map_objects(&text));
    }
    let map_parent = map_path.and_then(Path::parent).unwrap_or(&build.root);
    // A same-name ELF in a closer build subtree owns that configuration's
    // reports. Neither target names nor shared function names can override it.
    let common_depth = |left: &Path, right: &Path| {
        left.components()
            .zip(right.components())
            .take_while(|(left, right)| left == right)
            .count()
    };
    let fallback_paths: Vec<_> = paths
        .iter()
        .filter(|report| {
            let own_depth = common_depth(firmware.parent().unwrap_or(firmware), report);
            !build.artifacts.iter().any(|artifact| {
                artifact.kind == ArtifactKind::Firmware
                    && artifact.path != firmware
                    && artifact.path.file_name() == firmware.file_name()
                    && common_depth(artifact.path.parent().unwrap_or(&artifact.path), report)
                        > own_depth
            })
        })
        .cloned()
        .collect();
    let mut map_matches = BTreeSet::new();
    for object in objects {
        let expected = map_parent.join(&object);
        let expected = expected.canonicalize().unwrap_or(expected);
        if paths.contains(&expected) {
            map_matches.insert(expected);
            continue;
        }
        let expected = build.root.join(&object);
        let expected = expected.canonicalize().unwrap_or(expected);
        if paths.contains(&expected) {
            map_matches.insert(expected);
            continue;
        }
        // Keep directory components and require a unique suffix match when the
        // map uses paths from a different build location. A suffix alone cannot
        // override evidence that a report belongs to another configuration.
        let suffix_matches: Vec<_> = fallback_paths
            .iter()
            .filter(|path| object.components().count() > 1 && path.ends_with(&object))
            .collect();
        if suffix_matches.len() == 1 {
            map_matches.insert(suffix_matches[0].clone());
        }
    }
    let stem = firmware
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .to_lowercase();
    let mut target_matches: Vec<_> = fallback_paths
        .iter()
        .filter(|path| {
            path.strip_prefix(&build.root)
                .unwrap_or(path)
                .parent()
                .is_some_and(|parent| {
                    parent
                        .components()
                        .any(|part| target_name(&part.as_os_str().to_string_lossy()) == stem)
                })
        })
        .cloned()
        .collect();
    // A scan root can contain the same target in several build configurations,
    // even after another configuration's ELF has been cleaned. Object folders
    // may be siblings of bin/, so compare shared ancestors rather than requiring
    // reports below the ELF's immediate parent.
    if let Some(parent) = firmware.parent() {
        if let Some(depth) = target_matches
            .iter()
            .map(|path| common_depth(parent, path))
            .max()
        {
            target_matches.retain(|path| common_depth(parent, path) == depth);
        }
    }
    let provenance: Option<(Vec<PathBuf>, &str)> = if !map_matches.is_empty() {
        Some((
            map_matches.into_iter().collect(),
            "object paths in the ELF's linker map",
        ))
    } else if !target_matches.is_empty() {
        Some((
            target_matches,
            "build directories matching the ELF target name",
        ))
    } else {
        None
    };
    let analyzed_paths = provenance
        .as_ref()
        .map(|(paths, _)| paths.clone())
        .unwrap_or_else(|| fallback_paths.clone());
    let mut report = if provenance.is_some() {
        analyze_stack_files(analysis, analyzed_paths).map_err(|e| e.to_string())?
    } else {
        // These are candidates across the build folder, not a selected set.
        // An unrelated unreadable report must not hide usable function matches.
        let mut combined = analyze_stack_files(analysis, Vec::new()).map_err(|e| e.to_string())?;
        for path in analyzed_paths {
            match analyze_stack_files(analysis, vec![path]) {
                Ok(candidate) => {
                    combined.entries.extend(candidate.entries);
                    for warning in candidate.warnings {
                        if !combined.warnings.contains(&warning) {
                            combined.warnings.push(warning);
                        }
                    }
                }
                Err(error) => combined
                    .warnings
                    .push(format!("Skipped stack report while guessing: {error}")),
            }
        }
        if !combined.entries.is_empty() {
            combined
                .warnings
                .retain(|warning| !warning.starts_with("No stack usage information found."));
        }
        combined
            .entries
            .sort_by_key(|entry| std::cmp::Reverse(entry.local_bytes));
        combined
    };
    let (chosen, reason) = if let Some(provenance) = provenance {
        provenance
    } else {
        // A cleaned configuration may leave reports without a competing ELF.
        // Prefer matching reports in the closest shared build subtree, just as
        // target-directory guesses do, before comparing symbol ownership.
        if let Some(parent) = firmware.parent() {
            if let Some(depth) = report
                .entries
                .iter()
                .filter(|entry| !entry.symbol_candidates.is_empty())
                .map(|entry| common_depth(parent, Path::new(&entry.report_file)))
                .max()
            {
                report
                    .entries
                    .retain(|entry| common_depth(parent, Path::new(&entry.report_file)) == depth);
            }
        }
        let mut symbol_files: BTreeMap<&str, BTreeSet<PathBuf>> = BTreeMap::new();
        let mut matched_files = BTreeSet::new();
        for entry in &report.entries {
            let path = PathBuf::from(&entry.report_file);
            if entry.symbol_candidates.len() == 1 {
                matched_files.insert(path.clone());
            }
            // An unresolved choice between ELF symbols still competes with
            // other reports for each candidate; ignoring it invents ownership.
            for symbol in &entry.symbol_candidates {
                symbol_files.entry(symbol).or_default().insert(path.clone());
            }
        }
        let ambiguous: BTreeSet<_> = symbol_files
            .values()
            .filter(|files| files.len() > 1)
            .flat_map(|files| files.iter().cloned())
            .collect();
        if !ambiguous.is_empty() {
            report.warnings.push("Ambiguous reports: multiple files match the same ELF functions. Choose reports in the left menu to resolve build ownership.".into());
        }
        // If a whole build directory is ambiguous, its other reports cannot establish provenance.
        let ambiguous_parents: BTreeSet<_> = ambiguous.iter().filter_map(|p| p.parent()).collect();
        let chosen = matched_files
            .into_iter()
            .filter(|p| {
                !p.parent()
                    .is_some_and(|parent| ambiguous_parents.contains(parent))
            })
            .collect();
        (chosen, "unambiguous function matches in the ELF")
    };
    let chosen: BTreeSet<_> = chosen.into_iter().collect();
    report
        .entries
        .retain(|entry| chosen.contains(Path::new(&entry.report_file)));
    if chosen.is_empty() {
        if !paths.is_empty() {
            report.warnings.push("Could not confidently select stack reports for this ELF. Check the report files or directories in the left menu.".into());
        }
        return Ok((report, None));
    }
    let mut selection = StackSelection {
        paths: chosen.iter().cloned().collect(),
        ..Default::default()
    };
    // Remember complete directories so new reports and subdirectories inherit the choice.
    let parents: BTreeSet<_> = chosen.iter().filter_map(|p| p.parent()).collect();
    for parent in parents {
        if parent != build.root
            && paths
                .iter()
                .filter(|p| p.starts_with(parent))
                .all(|p| chosen.contains(p))
        {
            selection.paths.retain(|p| !p.starts_with(parent));
            selection.paths.push(parent.to_owned());
        }
    }
    selection.remember_sibling_directories(&paths);
    selection.paths.sort();
    report.warnings.push(format!("Stack reports selected automatically using {reason}. Review the checked files in the left menu."));
    Ok((report, Some(selection)))
}

/// Read GNU ld input-object rows even when the map was generated without --cref.
fn map_objects(text: &str) -> BTreeSet<PathBuf> {
    let mut in_map = false;
    let mut objects = BTreeSet::new();
    for line in text.lines().map(str::trim) {
        if line == "Linker script and memory map" {
            in_map = true;
            continue;
        }
        if line == "Cross Reference Table" {
            break;
        }
        if !in_map {
            continue;
        }
        let object = if let Some(object) = line.strip_prefix("LOAD ") {
            object.trim().to_owned()
        } else {
            let fields: Vec<_> = line.split_whitespace().collect();
            let Some(address) = fields.iter().position(|field| field.starts_with("0x")) else {
                continue;
            };
            if fields
                .get(address + 1)
                .is_none_or(|field| !field.starts_with("0x"))
            {
                continue;
            }
            fields[address + 2..].join(" ")
        };
        let object = object.replace('\\', "/");
        if !object.contains('(') && (object.ends_with(".o") || object.ends_with(".obj")) {
            objects.insert(PathBuf::from(object).with_extension("su"));
        }
    }
    objects
}

fn target_name(name: &str) -> String {
    let name = name.to_lowercase();
    let name = name.strip_suffix(".dir").unwrap_or(&name);
    name.strip_suffix("-objects")
        .or_else(|| name.strip_suffix("_objects"))
        .or_else(|| name.strip_suffix("_elf"))
        .or_else(|| name.strip_suffix(".elf"))
        .unwrap_or(name)
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use firmware_analysis_core::{
        analyze_path,
        build::scan_folder,
        dependencies::{DependencyGraph, DependencyNode},
    };

    fn setup() -> (tempfile::TempDir, PathBuf, Analysis) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let elf = root.join("app.elf");
        std::fs::write(&elf, include_bytes!("../../../fixtures/build/cortex-m.elf")).unwrap();
        let analysis = analyze_path(elf, &Default::default()).unwrap();
        (dir, root, analysis)
    }

    fn report(path: &Path, bytes: u64) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, format!("diag.c:22:36:diagnose\t{bytes}\tstatic\n")).unwrap();
    }

    #[test]
    fn target_directory_names_disambiguate_identical_function_names() {
        let (_dir, root, analysis) = setup();
        let chosen = root.join("CMakeFiles/app-objects.dir/src/diag.su");
        let other = root.join("CMakeFiles/app-extra-objects.dir/src/diag.su");
        report(&chosen, 56);
        report(&other, 96);
        let (stack, selection) = guess(&analysis, &scan_folder(&root).unwrap()).unwrap();
        let selection = selection.unwrap();
        assert!(selection.contains(&chosen));
        assert!(!selection.contains(&other));
        assert!(selection.contains(&chosen.parent().unwrap().join("new.su")));
        assert_eq!(stack.entries.len(), 1);
        assert_eq!(stack.entries[0].local_bytes, 56);
    }

    #[test]
    fn target_directory_guess_stays_with_the_elf_build_configuration() {
        let (_dir, root, mut analysis) = setup();
        let elf = root.join("release/app.elf");
        std::fs::create_dir_all(elf.parent().unwrap()).unwrap();
        std::fs::rename(&analysis.path, &elf).unwrap();
        analysis.path = elf.display().to_string();
        let chosen = root.join("release/CMakeFiles/app.dir/diag.su");
        let other = root.join("debug/CMakeFiles/app.dir/diag.su");
        report(&chosen, 56);
        report(&other, 96);
        let (stack, selection) = guess(&analysis, &scan_folder(&root).unwrap()).unwrap();
        let selection = selection.unwrap();
        assert!(selection.contains(&chosen));
        assert!(!selection.contains(&other));
        assert_eq!(stack.entries.len(), 1);
        assert_eq!(stack.entries[0].local_bytes, 56);
    }

    #[test]
    fn target_directory_guess_prefers_sibling_objects_in_the_same_configuration() {
        let (_dir, root, mut analysis) = setup();
        let elf = root.join("release/bin/app.elf");
        std::fs::create_dir_all(elf.parent().unwrap()).unwrap();
        std::fs::rename(&analysis.path, &elf).unwrap();
        analysis.path = elf.display().to_string();
        let chosen = root.join("release/CMakeFiles/app.dir/diag.su");
        let other = root.join("debug/CMakeFiles/app.dir/diag.su");
        report(&chosen, 56);
        report(&other, 96);
        // The debug ELF may have been cleaned while its reports remain.
        let (stack, selection) = guess(&analysis, &scan_folder(&root).unwrap()).unwrap();
        let selection = selection.unwrap();
        assert!(selection.contains(&chosen));
        assert!(!selection.contains(&other));
        assert_eq!(stack.entries.len(), 1);
        assert_eq!(stack.entries[0].local_bytes, 56);
    }

    #[test]
    fn function_guess_prefers_the_same_configuration_after_other_elf_is_cleaned() {
        let (_dir, root, mut analysis) = setup();
        let elf = root.join("release/bin/app.elf");
        std::fs::create_dir_all(elf.parent().unwrap()).unwrap();
        std::fs::rename(&analysis.path, &elf).unwrap();
        analysis.path = elf.display().to_string();
        let chosen = root.join("release/objects/diag.su");
        let other = root.join("debug/objects/diag.su");
        report(&chosen, 56);
        report(&other, 96);
        for contents in [
            "main.c:69:5:main\t96\tstatic\n",
            "diag.c:22:36:diagnose\t96\tstatic\n",
        ] {
            std::fs::write(&other, contents).unwrap();
            let (stack, selection) = guess(&analysis, &scan_folder(&root).unwrap()).unwrap();
            let selection = selection.unwrap();
            assert!(selection.contains(&chosen));
            assert!(!selection.contains(&other));
            assert_eq!(stack.entries.len(), 1);
            assert_eq!(stack.entries[0].local_bytes, 56);
        }
    }

    #[test]
    fn guesses_do_not_borrow_reports_from_another_firmware_configuration() {
        for directory in ["CMakeFiles/app.dir", "objects"] {
            let (_dir, root, mut analysis) = setup();
            let elf = root.join("release/bin/app.elf");
            std::fs::create_dir_all(elf.parent().unwrap()).unwrap();
            std::fs::rename(&analysis.path, &elf).unwrap();
            analysis.path = elf.display().to_string();
            let other_elf = root.join("debug/bin/app.elf");
            std::fs::create_dir_all(other_elf.parent().unwrap()).unwrap();
            std::fs::copy(&elf, &other_elf).unwrap();
            let other = root.join("debug").join(directory).join("diag.su");
            report(&other, 96);
            let (stack, selection) = guess(&analysis, &scan_folder(&root).unwrap()).unwrap();
            assert!(selection.is_none(), "{directory}");
            assert!(stack.entries.is_empty());

            // Reports need not be below the ELF's immediate parent (bin/).
            let chosen = root.join("release").join(directory).join("diag.su");
            report(&chosen, 56);
            let (stack, selection) = guess(&analysis, &scan_folder(&root).unwrap()).unwrap();
            let selection = selection.unwrap();
            assert!(selection.contains(&chosen));
            assert!(!selection.contains(&other));
            assert_eq!(stack.entries.len(), 1);
            assert_eq!(stack.entries[0].local_bytes, 56);
        }
    }

    #[test]
    fn unrelated_unreadable_reports_do_not_block_a_target_directory_guess() {
        let (_dir, root, analysis) = setup();
        let chosen = root.join("CMakeFiles/app.dir/diag.su");
        report(&chosen, 56);
        std::fs::write(root.join("unrelated.su"), [0xff]).unwrap();
        let (stack, selection) = guess(&analysis, &scan_folder(&root).unwrap()).unwrap();
        assert!(selection.unwrap().contains(&chosen));
        assert_eq!(stack.entries.len(), 1);
    }

    #[test]
    fn unreadable_unrelated_reports_do_not_block_function_match_guesses() {
        let (_dir, root, analysis) = setup();
        let chosen = root.join("reports/diag.su");
        report(&chosen, 56);
        let unreadable = root.join("unrelated/broken.su");
        std::fs::create_dir_all(unreadable.parent().unwrap()).unwrap();
        std::fs::write(&unreadable, [0xff]).unwrap();
        let (stack, selection) = guess(&analysis, &scan_folder(&root).unwrap()).unwrap();
        assert!(selection.unwrap().contains(&chosen));
        assert_eq!(stack.entries.len(), 1);
        assert_eq!(stack.entries[0].local_bytes, 56);
        assert!(stack
            .warnings
            .iter()
            .any(|warning| warning.contains("broken.su")));
    }

    #[test]
    fn map_suffix_guess_does_not_borrow_another_configurations_reports() {
        let (_dir, root, mut analysis) = setup();
        let elf = root.join("release/bin/app.elf");
        std::fs::create_dir_all(elf.parent().unwrap()).unwrap();
        std::fs::rename(&analysis.path, &elf).unwrap();
        analysis.path = elf.display().to_string();
        let other_elf = root.join("debug/bin/app.elf");
        std::fs::create_dir_all(other_elf.parent().unwrap()).unwrap();
        std::fs::copy(&elf, &other_elf).unwrap();
        let map = root.join("release/app.map");
        std::fs::write(&map, "Linker script and memory map\nLOAD objects/diag.o\n").unwrap();
        analysis.dependencies.map_path = Some(map.display().to_string());
        let other = root.join("debug/objects/diag.su");
        report(&other, 96);

        let (stack, selection) = guess(&analysis, &scan_folder(&root).unwrap()).unwrap();
        assert!(selection.is_none());
        assert!(stack.entries.is_empty());

        // A relocated report is still eligible within the selected configuration.
        let chosen = root.join("release/relocated/objects/diag.su");
        report(&chosen, 56);
        let (stack, selection) = guess(&analysis, &scan_folder(&root).unwrap()).unwrap();
        let selection = selection.unwrap();
        assert!(selection.contains(&chosen));
        assert!(!selection.contains(&other));
        assert_eq!(stack.entries.len(), 1);
        assert_eq!(stack.entries[0].local_bytes, 56);
    }

    #[test]
    fn exact_map_object_paths_override_target_names_and_same_suffixes() {
        let (_dir, root, mut analysis) = setup();
        let chosen = root.join("release/objects/diag.su");
        let same_suffix = root.join("debug/objects/diag.su");
        let named_target = root.join("CMakeFiles/app.dir/diag.su");
        report(&chosen, 56);
        report(&same_suffix, 80);
        report(&named_target, 96);
        analysis.dependencies = DependencyGraph {
            map_path: Some(root.join("release/app.map").display().to_string()),
            nodes: vec![DependencyNode {
                id: "test".into(),
                label: "test".into(),
                evidence: "test".into(),
                usage: None,
                objects: vec!["objects/diag.o".into()],
            }],
            ..Default::default()
        };
        let (stack, selection) = guess(&analysis, &scan_folder(&root).unwrap()).unwrap();
        let selection = selection.unwrap();
        assert!(selection.contains(&chosen));
        assert!(!selection.contains(&same_suffix));
        assert!(!selection.contains(&named_target));
        assert_eq!(stack.entries.len(), 1);
        assert_eq!(stack.entries[0].local_bytes, 56);
    }

    #[test]
    fn map_without_cross_references_selects_linked_objects() {
        let (_dir, root, analysis) = setup();
        let chosen = root.join("objects/diag.su");
        let other = root.join("CMakeFiles/app.dir/diag.su");
        report(&chosen, 56);
        report(&other, 96);
        std::fs::write(root.join("app.map"), "Discarded input sections\n .text 0x0 0x20 other/diag.o\nLinker script and memory map\n .text 0x08000000 0x20 objects/diag.o\n").unwrap();
        let (stack, selection) = guess(&analysis, &scan_folder(&root).unwrap()).unwrap();
        let selection = selection.unwrap();
        assert!(selection.contains(&chosen));
        assert!(!selection.contains(&other));
        assert_eq!(stack.entries[0].local_bytes, 56);
    }

    #[test]
    fn ambiguous_symbol_candidates_still_block_overlapping_report_guesses() {
        let (_dir, root, mut analysis) = setup();
        let mut duplicate = analysis
            .symbols
            .iter()
            .find(|s| s.name == "diagnose")
            .unwrap()
            .clone();
        duplicate.address += 0x1000;
        duplicate.source_line = None;
        analysis.symbols.push(duplicate);
        let unique = root.join("first/diag.su");
        report(&unique, 56);
        let ambiguous = root.join("second/diag.su");
        std::fs::create_dir_all(ambiguous.parent().unwrap()).unwrap();
        std::fs::write(&ambiguous, "diag.c:999:1:diagnose\t96\tstatic\n").unwrap();
        let (stack, selection) = guess(&analysis, &scan_folder(&root).unwrap()).unwrap();
        assert!(selection.is_none());
        assert!(stack.entries.is_empty());
        assert!(stack
            .warnings
            .iter()
            .any(|warning| warning.starts_with("Ambiguous reports:")));
    }

    #[test]
    fn symbol_fallback_selects_unique_reports_and_leaves_unknown_reports_unchecked() {
        let (_dir, root, analysis) = setup();
        let chosen = root.join("reports/diag.su");
        let unknown = root.join("reports/unknown.su");
        report(&chosen, 56);
        std::fs::write(&unknown, "foreign.c:1:1:foreign_function\t24\tstatic\n").unwrap();
        let (stack, selection) = guess(&analysis, &scan_folder(&root).unwrap()).unwrap();
        let selection = selection.unwrap();
        assert!(selection.contains(&chosen));
        assert!(!selection.contains(&unknown));
        assert!(selection.contains(&root.join("reports/new.su")));
        assert_eq!(stack.entries.len(), 1);
        report(&root.join("reports/duplicate.su"), 96);
        let (stack, selection) = guess(&analysis, &scan_folder(&root).unwrap()).unwrap();
        assert!(selection.is_none());
        assert!(stack.entries.is_empty());
        assert!(stack
            .warnings
            .iter()
            .any(|warning| warning.starts_with("Ambiguous reports:")));
        let mut app = crate::Explorer {
            analysis: Some(std::sync::Arc::new(analysis)),
            stack: Some(stack),
            ..Default::default()
        };
        let ctx = crate::egui::Context::default();
        let output = ctx.run(Default::default(), |ctx| {
            crate::egui::CentralPanel::default().show(ctx, |ui| app.stack_view(ui));
        });
        assert!(output.shapes.iter().any(|shape| matches!(
            &shape.shape,
            crate::egui::Shape::Text(text)
                if text.galley.text().starts_with("Ambiguous reports:")
        )));
    }
}
