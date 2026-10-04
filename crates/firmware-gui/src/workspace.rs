use super::display::display_path;
use super::{egui, Explorer, Loaded};
use firmware_analysis_core::{
    analyze_path,
    build::{parse_map_regions, scan_folder, Artifact, ArtifactKind},
};
use std::{io::Read, path::PathBuf};

pub(super) fn analyze_selected(
    build: &firmware_analysis_core::build::BuildFolder,
    path: &std::path::Path,
    mut layout: Option<super::AnalysisOptions>,
    source: Option<String>,
) -> Result<(super::Analysis, Option<super::AnalysisOptions>, String), String> {
    if let Some(source) = &source {
        let extension = std::path::Path::new(source)
            .extension()
            .unwrap_or_default()
            .to_string_lossy()
            .to_ascii_lowercase();
        match extension.as_str() {
            "map" => {
                layout = Some(
                    parse_map_regions(&std::fs::read_to_string(source).map_err(|e| e.to_string())?)
                        .map_err(|e| e.to_string())?,
                )
            }
            "json" => {
                let options =
                    serde_json::from_slice(&std::fs::read(source).map_err(|e| e.to_string())?)
                        .map_err(|e| e.to_string())?;
                firmware_analysis_core::validate_options(&options).map_err(|e| e.to_string())?;
                layout = Some(options);
            }
            _ => {}
        }
    }
    let analysis =
        firmware_analysis_core::build::analyze_build_firmware(build, path, layout.as_ref())
            .map_err(|e| e.to_string())?;
    let source = source.unwrap_or_else(|| {
        if analysis.options.regions.is_empty() {
            "ELF inference".into()
        } else {
            build
                .matching_map(path)
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "Matching map".into())
        }
    });
    Ok((analysis, layout, source))
}

pub(super) fn read_dependency_map(analysis: &mut super::Analysis, path: &std::path::Path) {
    match std::fs::read_to_string(path) {
        Ok(text) => firmware_analysis_core::dependencies::import_map(
            analysis,
            &text,
            &path.display().to_string(),
        ),
        Err(error) => {
            analysis.dependencies = firmware_analysis_core::dependencies::units(analysis);
            analysis
                .dependencies
                .notes
                .push(format!("{}: {error}", path.display()));
        }
    }
}

/// Persist directory rules and explicit exceptions instead of a snapshot of files.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(from = "StoredStackSelection")]
pub(super) struct StackSelection {
    pub paths: Vec<PathBuf>,
    pub excluded: Vec<PathBuf>,
    pub auto_directories: Vec<PathBuf>,
}

#[derive(serde::Deserialize)]
#[serde(untagged)]
enum StoredStackSelection {
    Legacy(Vec<PathBuf>),
    Rules {
        #[serde(default)]
        paths: Vec<PathBuf>,
        #[serde(default)]
        excluded: Vec<PathBuf>,
        #[serde(default)]
        auto_directories: Vec<PathBuf>,
    },
}
impl From<StoredStackSelection> for StackSelection {
    fn from(value: StoredStackSelection) -> Self {
        match value {
            StoredStackSelection::Legacy(paths) => Self {
                paths,
                ..Default::default()
            },
            StoredStackSelection::Rules {
                paths,
                excluded,
                auto_directories,
            } => Self {
                paths,
                excluded,
                auto_directories,
            },
        }
    }
}
impl StackSelection {
    pub fn contains(&self, path: &std::path::Path) -> bool {
        // The most specific rule wins. Automatic siblings override an unchecked
        // directory, while explicit file exceptions override automatic siblings.
        let included = self
            .paths
            .iter()
            .filter(|p| path.starts_with(p))
            .map(|p| p.components().count() * 2)
            .chain(
                self.auto_directories
                    .iter()
                    .filter(|p| path.parent() == Some(p.as_path()))
                    .map(|p| p.components().count() * 2 + 1),
            )
            .max();
        let excluded = self
            .excluded
            .iter()
            .filter(|p| path.starts_with(p))
            .map(|p| p.components().count() * 2)
            .max();
        included.is_some_and(|depth| excluded.is_none_or(|excluded| depth > excluded))
    }

    pub fn set(&mut self, path: &std::path::Path, checked: bool, paths: &[PathBuf]) {
        let file = paths.iter().any(|p| p == path);
        if checked && file {
            if let Some(parent) = path.parent() {
                if !self.auto_directories.iter().any(|p| p == parent) {
                    // Existing unchecked siblings stay unchecked; future siblings are included.
                    let unchecked: Vec<_> = paths
                        .iter()
                        .filter(|p| {
                            p.parent() == Some(parent) && p.as_path() != path && !self.contains(p)
                        })
                        .cloned()
                        .collect();
                    self.excluded.extend(unchecked);
                    self.auto_directories.push(parent.to_owned());
                }
            }
        }
        self.paths.retain(|p| !p.starts_with(path));
        self.excluded.retain(|p| !p.starts_with(path));
        self.auto_directories.retain(|p| !p.starts_with(path));
        if checked {
            self.paths.push(path.to_owned());
        } else {
            self.excluded.push(path.to_owned());
            if file {
                if let Some(parent) = path.parent() {
                    if !paths
                        .iter()
                        .any(|p| p.parent() == Some(parent) && self.contains(p))
                    {
                        self.auto_directories.retain(|p| p != parent);
                        if !paths
                            .iter()
                            .any(|p| p.starts_with(parent) && self.contains(p))
                        {
                            // An entirely unchecked folder also excludes future reports.
                            self.paths.retain(|p| !p.starts_with(parent));
                            // Keep file exceptions even when their reports have disappeared.
                            // Re-enabling automatic siblings must not reselect those files.
                            self.excluded.push(parent.to_owned());
                        }
                    }
                }
            }
        }
        self.remember_sibling_directories(paths);
        self.paths.sort();
        self.paths.dedup();
        self.excluded.sort();
        self.excluded.dedup();
        self.auto_directories.sort();
        self.auto_directories.dedup();
    }

    pub(super) fn remember_sibling_directories(&mut self, paths: &[PathBuf]) {
        let parents: std::collections::BTreeSet<_> = paths
            .iter()
            .filter(|p| self.contains(p))
            .filter_map(|p| p.parent().map(std::path::Path::to_owned))
            .collect();
        for parent in parents {
            if self.auto_directories.contains(&parent) {
                continue;
            }
            let unchecked: Vec<_> = paths
                .iter()
                .filter(|p| p.parent() == Some(parent.as_path()) && !self.contains(p))
                .cloned()
                .collect();
            self.excluded.extend(unchecked);
            self.auto_directories.push(parent);
        }
    }

    fn report_paths(
        &self,
        build: &firmware_analysis_core::build::BuildFolder,
    ) -> Result<Vec<PathBuf>, String> {
        let mut candidates: std::collections::BTreeSet<_> = build
            .artifacts
            .iter()
            .filter(|a| a.kind == ArtifactKind::StackUsage)
            .map(|a| a.path.clone())
            .collect();
        for path in self.paths.iter().chain(&self.auto_directories) {
            if path.starts_with(&build.root) {
                continue;
            }
            if path.is_dir() {
                let external = scan_folder(path).map_err(|e| e.to_string())?;
                candidates.extend(
                    external
                        .artifacts
                        .into_iter()
                        .filter(|a| a.kind == ArtifactKind::StackUsage)
                        .map(|a| a.path),
                );
            } else if path.is_file() {
                candidates.insert(path.clone());
            }
        }
        Ok(candidates
            .into_iter()
            .filter(|p| self.contains(p))
            .collect())
    }
}

pub(super) fn load_stack_reports(
    analysis: &super::Analysis,
    build: &firmware_analysis_core::build::BuildFolder,
    selected: Option<StackSelection>,
) -> Result<super::LoadedStack, String> {
    if let Some(selection) = selected {
        let paths = selection.report_paths(build)?;
        let report = firmware_analysis_core::stack::analyze_stack_files(analysis, paths)
            .map_err(|e| e.to_string())?;
        Ok((report, Some(selection)))
    } else {
        super::stack_guess::guess(analysis, build)
    }
}

fn report_folder_ui(
    ui: &mut egui::Ui,
    folder: &std::path::Path,
    paths: &[PathBuf],
    selected: &mut StackSelection,
    search: &str,
) {
    let descendants: Vec<_> = paths.iter().filter(|p| p.starts_with(folder)).collect();
    let visible: Vec<_> = descendants
        .iter()
        .copied()
        .filter(|p| p.to_string_lossy().to_lowercase().contains(search))
        .collect();
    if visible.is_empty() {
        return;
    }
    let count = descendants.len();
    let checked_count = descendants.iter().filter(|p| selected.contains(p)).count();
    let mut checked = count > 0 && count == checked_count;
    let id = ui.make_persistent_id(folder);
    egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, true)
        .show_header(ui, |ui| {
            if ui
                .add(
                    egui::Checkbox::new(
                        &mut checked,
                        format!(
                            "{} ({checked_count}/{count})",
                            folder
                                .file_name()
                                .unwrap_or(folder.as_os_str())
                                .to_string_lossy()
                        ),
                    )
                    .indeterminate(checked_count > 0 && checked_count < count),
                )
                .on_hover_text(
                    "Select all stack reports in this directory, including new files on refresh",
                )
                .changed()
            {
                selected.set(folder, checked, paths);
            }
        })
        .body_unindented(|ui| {
            ui.scope(|ui| {
                ui.spacing_mut().indent = 8.0;
                ui.indent(id.with("reports"), |ui| {
                    let directories: std::collections::BTreeSet<_> = visible
                        .iter()
                        .filter_map(|path| {
                            let relative = path.strip_prefix(folder).ok()?;
                            (relative.components().count() > 1).then(|| {
                                folder.join(relative.components().next().unwrap().as_os_str())
                            })
                        })
                        .collect();
                    for directory in directories {
                        report_folder_ui(ui, &directory, paths, selected, search);
                    }
                    for path in visible.iter().filter(|p| p.parent() == Some(folder)) {
                        let mut checked = selected.contains(path);
                        if ui
                            .checkbox(
                                &mut checked,
                                path.file_name().unwrap_or_default().to_string_lossy(),
                            )
                            .on_hover_text(display_path(&path.to_string_lossy()))
                            .changed()
                        {
                            selected.set(path, checked, paths);
                        }
                    }
                });
            });
        });
}

impl Explorer {
    pub(super) fn saved_stack_selection(
        &self,
        firmware: &std::path::Path,
    ) -> Option<StackSelection> {
        self.build_settings
            .get(&self.build.as_ref()?.root)?
            .stack_reports
            .get(firmware)
            .cloned()
    }
    #[cfg(test)]
    pub(super) fn saved_stack_reports(&self, firmware: &std::path::Path) -> Option<Vec<PathBuf>> {
        self.saved_stack_selection(firmware)
            .map(|selection| selection.paths)
    }
    #[cfg(test)]
    pub(super) fn select_stack_reports(&mut self, paths: Vec<PathBuf>) {
        self.select_stack_selection(StackSelection {
            paths,
            ..Default::default()
        });
    }
    pub(super) fn select_stack_selection(&mut self, selection: StackSelection) {
        if self.receiver.is_some() {
            return;
        }
        let (Some(analysis), Some(build)) = (self.analysis.clone(), self.build.clone()) else {
            return;
        };
        self.build_settings
            .entry(build.root.clone())
            .or_default()
            .stack_reports
            .insert(PathBuf::from(&analysis.path), selection.clone());
        // The old report no longer represents the saved selection, including on failure.
        self.stack = None;
        // Save the user's intent before analysis, even if the app closes during the job.
        self.job(move || {
            let paths = selection.report_paths(&build)?;
            let report = firmware_analysis_core::stack::analyze_stack_files(&analysis, paths)
                .map_err(|e| e.to_string())?;
            Ok(Loaded::SelectedStack(report, selection))
        });
        self.persist_preferences();
    }
    pub(super) fn current_stack_selection(&self) -> StackSelection {
        let Some(analysis) = &self.analysis else {
            return StackSelection::default();
        };
        let mut selection = self
            .saved_stack_selection(std::path::Path::new(&analysis.path))
            .unwrap_or_else(|| StackSelection {
                paths: self
                    .stack
                    .iter()
                    .flat_map(|report| report.entries.iter())
                    .map(|entry| PathBuf::from(&entry.report_file))
                    .collect::<std::collections::BTreeSet<_>>()
                    .into_iter()
                    .collect(),
                ..Default::default()
            });
        let paths: Vec<_> = self
            .build
            .iter()
            .flat_map(|build| build.artifacts.iter())
            .filter(|artifact| artifact.kind == ArtifactKind::StackUsage)
            .map(|artifact| artifact.path.clone())
            .collect();
        selection.remember_sibling_directories(&paths);
        selection
    }

    pub(super) fn saved_dependency_map(&self, firmware: &std::path::Path) -> Option<PathBuf> {
        self.build_settings
            .get(&self.build.as_ref()?.root)?
            .dependency_maps
            .get(firmware)
            .cloned()
    }
    pub(super) fn dependency_map_for_reload(&self) -> Option<PathBuf> {
        let analysis = self.analysis.as_ref()?;
        let firmware = std::path::Path::new(&analysis.path);
        // A failed read clears map_path, but the selected map must still be
        // retried when changing memory layouts, just as it is on refresh.
        self.saved_dependency_map(firmware)
            .or_else(|| analysis.dependencies.map_path.as_ref().map(PathBuf::from))
            .or_else(|| {
                self.build
                    .as_ref()?
                    .matching_map(firmware)
                    .map(PathBuf::from)
            })
    }
    pub(super) fn apply_dependency_map(&mut self, path: PathBuf) {
        let Some(analysis) = self.analysis.clone() else {
            return;
        };
        self.job(move || {
            let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
            let graph = firmware_analysis_core::dependencies::from_map(
                &analysis,
                &text,
                &path.display().to_string(),
            )?;
            let mut analysis = (*analysis).clone();
            analysis.dependencies = graph;
            Ok(Loaded::Dependencies(analysis, path))
        });
    }

    pub(super) fn saved_layout(&self, path: &std::path::Path) -> Option<&super::SavedLayout> {
        self.build_settings
            .get(&self.build.as_ref()?.root)?
            .layouts
            .get(path)
    }

    pub(super) fn reset_build_settings(&mut self) {
        if self.receiver.is_some() {
            return;
        }
        if let Some(build) = &self.build {
            if let Some(analysis) = &self.analysis {
                self.load_firmware(PathBuf::from(&analysis.path), None, true);
            } else {
                self.build_settings.remove(&build.root);
                self.remembered_firmware = None;
                self.pending_restore = None;
                self.preview = None;
                self.discover_layout();
            }
        }
    }

    pub(super) fn map_in_use(&self, path: &std::path::Path) -> bool {
        self.analysis.is_some() && std::path::Path::new(&self.layout_source) == path
    }

    pub(super) fn pick_build(&mut self) {
        if let Some(path) = rfd::FileDialog::new().pick_folder() {
            self.scan_build(path);
        }
    }
    pub(super) fn scan_build(&mut self, path: PathBuf) {
        self.job(move || {
            scan_folder(path)
                .map(Loaded::Build)
                .map_err(|e| e.to_string())
        });
    }
    pub(super) fn select_artifact(&mut self, artifact: Artifact) {
        if artifact.kind == ArtifactKind::Firmware {
            self.open(artifact.path);
            return;
        }
        self.job(move || {
            let file = std::fs::File::open(&artifact.path).map_err(|e| e.to_string())?;
            let mut bytes = Vec::new();
            file.take(1024 * 1024 + 1)
                .read_to_end(&mut bytes)
                .map_err(|e| e.to_string())?;
            let truncated = bytes.len() > 1024 * 1024;
            bytes.truncate(1024 * 1024);
            let mut text = String::from_utf8_lossy(&bytes).into_owned();
            if truncated {
                text.push_str(
                    "\n[Preview truncated at 1 MiB; applying a map reads the complete file.]",
                );
            }
            Ok(Loaded::Text(artifact.path, text))
        });
    }
    pub(super) fn apply_map(&mut self, path: PathBuf) {
        let dependency_map = self.dependency_map_for_reload();
        let current_path = self.analysis.as_ref().map(|a| a.path.clone());
        self.job(move || {
            let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
            let options = parse_map_regions(&text).map_err(|e| e.to_string())?;
            let mut analysis = current_path.map(|p| {
                let mut a = analyze_path(p, &options)?;
                a.warnings.push(format!("Memory regions selected from {}. Flash/RAM roles are inferred from names and attributes; verify this map belongs to the selected firmware.", path.display()));
                Ok::<_, firmware_analysis_core::Error>(a)
            }).transpose().map_err(|e| e.to_string())?;
            if let (Some(analysis), Some(map)) = (&mut analysis, dependency_map) { read_dependency_map(analysis, &map); }
            Ok(Loaded::Config(options, analysis, path.display().to_string()))
        });
    }
    pub(super) fn build_browser(&mut self, ctx: &egui::Context) {
        let Some(build) = self.build.clone() else {
            return;
        };
        let mut selected = None;
        let mut selected_map = None;
        let mut reports = self.current_stack_selection();
        let previous_reports = reports.clone();
        egui::SidePanel::left("build_artifacts")
            .default_width(260.0)
            .width_range(180.0..=500.0)
            .show(ctx, |ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.artifact_search)
                        .hint_text("Find file...")
                        .desired_width(f32::INFINITY),
                );
                ui.small(format!("{} compatible files", build.artifacts.len()));
                if !build.warnings.is_empty() {
                    ui.collapsing(format!("{} scan notes", build.warnings.len()), |ui| {
                        for note in &build.warnings {
                            ui.label(note);
                        }
                    });
                }
                ui.separator();
                egui::ScrollArea::vertical().show(ui, |ui| {
                    if build.artifacts.is_empty() {
                        ui.label("No compatible files found in this folder or its subfolders.");
                    }
                    for kind in [
                        ArtifactKind::Firmware,
                        ArtifactKind::Map,
                        ArtifactKind::StackUsage,
                        ArtifactKind::MemoryLayout,
                    ] {
                        let artifacts: Vec<_> = build
                            .artifacts
                            .iter()
                            .filter(|a| a.kind == kind)
                            .filter(|a| {
                                a.path
                                    .strip_prefix(&build.root)
                                    .unwrap_or(&a.path)
                                    .to_string_lossy()
                                    .to_lowercase()
                                    .contains(&self.artifact_search.to_lowercase())
                            })
                            .collect();
                        if artifacts.is_empty() {
                            continue;
                        }
                        if kind == ArtifactKind::StackUsage {
                            egui::CollapsingHeader::new(format!("{} ({})", kind.label(), artifacts.len()))
                                .default_open(true)
                                .show(ui, |ui| {
                                    let paths: Vec<_> = build.artifacts.iter()
                                        .filter(|a| a.kind == ArtifactKind::StackUsage)
                                        .map(|a| a.path.clone()).collect();
                                    ui.add_enabled_ui(self.analysis.is_some() && self.receiver.is_none(), |ui| {
                                        report_folder_ui(ui, &build.root, &paths, &mut reports, &self.artifact_search.to_lowercase());
                                    });
                                });
                            continue;
                        }
                        egui::CollapsingHeader::new(format!(
                            "{} ({})",
                            kind.label(),
                            artifacts.len()
                        ))
                        .default_open(
                            kind == ArtifactKind::Firmware
                                || (kind == ArtifactKind::Map
                                    && artifacts.iter().any(|a| self.map_in_use(&a.path))),
                        )
                        .show(ui, |ui| {
                            for artifact in artifacts {
                                let active = artifact.kind == ArtifactKind::Firmware
                                    && self.analysis.as_ref().is_some_and(|a| {
                                        std::path::Path::new(&a.path) == artifact.path
                                    })
                                    || self
                                        .preview
                                        .as_ref()
                                        .map(|(p, _)| p == &artifact.path)
                                        .unwrap_or_else(|| {
                                            self.analysis.as_ref().is_some_and(|a| {
                                                std::path::Path::new(&a.path) == artifact.path
                                            })
                                        });
                                let label = artifact
                                    .path
                                    .strip_prefix(&build.root)
                                    .unwrap_or(&artifact.path)
                                    .display()
                                    .to_string();
                                let in_use = artifact.kind == ArtifactKind::Map
                                    && self.map_in_use(&artifact.path);
                                if artifact.kind == ArtifactKind::Map {
                                    ui.horizontal(|ui| {
                                        if ui.add_enabled(self.analysis.is_some() && self.receiver.is_none(),
                                            egui::RadioButton::new(in_use, ""))
                                            .on_hover_text("Use memory regions from this map for the current ELF")
                                            .clicked() && !in_use
                                        {
                                            selected_map = Some(artifact.path.clone());
                                        }
                                        if ui.add_enabled(self.receiver.is_none(),
                                            egui::Button::new(display_path(&label)).frame(false).selected(active).wrap())
                                            .on_hover_text("Preview map").clicked()
                                        {
                                            selected = Some(artifact.clone());
                                        }
                                    });
                                    continue;
                                }
                                let label = egui::RichText::new(display_path(&label));
                                if ui
                                    .add_enabled(
                                        self.receiver.is_none(),
                                        egui::Button::new(label)
                                            .frame(false)
                                            .selected(active)
                                            .wrap(),
                                    )
                                    .on_hover_text(display_path(&artifact.path.to_string_lossy()))
                                    .clicked()
                                {
                                    selected = Some(artifact.clone());
                                }
                            }
                        });
                    }
                });
            });
        if reports != previous_reports {
            self.select_stack_selection(reports);
        } else if let Some(path) = selected_map {
            self.apply_map(path);
        } else if let Some(artifact) = selected {
            self.select_artifact(artifact);
        }
    }
    pub(super) fn artifact_preview(&mut self, ui: &mut egui::Ui) -> bool {
        let Some((path, text)) = self.preview.clone() else {
            return false;
        };
        ui.heading(path.file_name().unwrap_or_default().to_string_lossy());
        ui.label(display_path(&path.to_string_lossy()));
        ui.horizontal(|ui| {
            if self.analysis.is_some() && ui.button("Back to firmware").clicked() {
                self.preview = None;
            }
            ui.add_enabled_ui(self.receiver.is_none(), |ui| {
                match path
                    .extension()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_ascii_lowercase()
                    .as_str()
                {
                    "map" => {
                        if self.analysis.is_some()
                            && ui.button("Use cross references from this map").clicked()
                        {
                            self.apply_dependency_map(path.clone());
                        }
                        ui.small("Select this map's radio button in the left menu to use its memory regions.");
                    }
                    "json" if ui.button("Use this memory layout").clicked() => {
                        self.configure(Some(path.clone()));
                    }
                    _ => {}
                }
            });
        });
        ui.separator();
        egui::ScrollArea::both()
            .id_salt("artifact_text")
            .show(ui, |ui| {
                ui.add(egui::Label::new(egui::RichText::new(text).monospace()).selectable(true));
            });
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folder_rules_keep_new_files_selected_and_preserve_explicit_exceptions() {
        let root = PathBuf::from("build");
        let paths = vec![
            root.join("a.su"),
            root.join("objects/b.su"),
            root.join("objects/nested/c.su"),
        ];
        let mut selected = StackSelection::default();
        selected.set(&root, true, &paths);
        selected.set(&paths[1], false, &paths);
        assert_eq!(selected.paths, vec![root.clone()]);
        assert!(selected.contains(&paths[0]));
        assert!(!selected.contains(&paths[1]));
        assert!(selected.contains(&paths[2]));
        assert!(selected.contains(&root.join("objects/new.su")));
        selected.set(&root.join("objects"), false, &paths);
        assert!(selected.contains(&paths[0]));
        assert!(!selected.contains(&root.join("objects/new.su")));
        assert!(!selected.contains(&paths[2]));
        selected.set(&root.join("objects"), true, &paths);
        assert!(selected.contains(&paths[1]));
        assert!(selected.contains(&root.join("objects/new.su")));
        selected.set(&root, false, &paths);
        assert!(!selected.contains(&root.join("new.su")));
    }

    #[test]
    fn selecting_one_file_selects_future_siblings_but_preserves_unchecked_existing_files() {
        let root = PathBuf::from("build");
        let paths = vec![root.join("a.su"), root.join("b.su")];
        let mut selected = StackSelection::default();
        selected.set(&paths[0], true, &paths);
        assert!(selected.contains(&paths[0]));
        assert!(!selected.contains(&paths[1]));
        assert!(selected.contains(&root.join("new.su")));
        assert!(!selected.contains(&root.join("other/new.su")));
        selected.set(&paths[0], false, &paths);
        assert!(!selected.contains(&root.join("new.su")));
        // A later selection inside an excluded directory re-enables future siblings.
        selected.set(&paths[1], true, &paths);
        assert!(!selected.contains(&paths[0]));
        assert!(selected.contains(&paths[1]));
        assert!(selected.contains(&root.join("new.su")));
    }

    #[test]
    fn deselecting_every_file_in_a_recursive_folder_excludes_future_files() {
        let root = PathBuf::from("build");
        let paths = vec![
            root.join("objects/a.su"),
            root.join("objects/b.su"),
            root.join("keep.su"),
        ];
        let mut selected = StackSelection::default();
        selected.set(&root, true, &paths);
        selected.set(&paths[0], false, &paths);
        selected.set(&paths[1], false, &paths);
        assert!(!selected.contains(&root.join("objects/new.su")));
        assert!(selected.contains(&paths[2]));
        assert!(selected.contains(&root.join("new.su")));
    }

    #[test]
    fn partial_default_selections_include_future_siblings_and_can_select_subdirectories() {
        let root = PathBuf::from("build");
        let paths = vec![
            root.join("a.su"),
            root.join("b.su"),
            root.join("objects/c.su"),
        ];
        let mut selected = StackSelection {
            paths: paths[..2].to_vec(),
            ..Default::default()
        };
        selected.set(&paths[0], false, &paths);
        assert!(!selected.contains(&paths[0]));
        assert!(selected.contains(&paths[1]));
        assert!(selected.contains(&root.join("new.su")));
        assert!(!selected.contains(&paths[2]));
        selected.set(&root.join("objects"), true, &paths);
        assert!(selected.paths.contains(&root.join("objects")));
        assert!(selected.contains(&paths[2]));
        assert!(selected.contains(&root.join("objects/nested/new.su")));
    }

    #[test]
    fn missing_unchecked_sibling_stays_excluded_after_reselecting_the_last_file() {
        let root = PathBuf::from("build");
        let first = root.join("a.su");
        let missing = root.join("b.su");
        let mut selected = StackSelection::default();
        selected.set(&first, true, &[first.clone(), missing.clone()]);
        selected.set(&first, false, std::slice::from_ref(&first));
        selected.set(&first, true, std::slice::from_ref(&first));
        assert!(selected.contains(&first));
        assert!(!selected.contains(&missing));
        assert!(selected.contains(&root.join("new.su")));
    }

    #[test]
    fn old_saved_file_and_folder_lists_still_deserialize() {
        let paths = vec![PathBuf::from("build/a.su"), PathBuf::from("build/objects")];
        let selection: StackSelection = serde_json::from_value(serde_json::json!(paths)).unwrap();
        assert_eq!(selection.paths, paths);
        assert!(selection.contains(std::path::Path::new("build/objects/new.su")));
    }
}
