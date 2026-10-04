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

impl Explorer {
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

    pub(super) fn load_build_stack(&mut self) {
        let (Some(build), Some(analysis)) = (self.build.clone(), self.analysis.clone()) else {
            return;
        };
        self.job(move || {
            let paths = build.artifacts.iter().filter(|a| a.kind == ArtifactKind::StackUsage).map(|a| a.path.clone()).collect();
            let mut report = firmware_analysis_core::stack::analyze_stack_files(&analysis, paths).map_err(|e| e.to_string())?;
            report.warnings.push(format!("Reports loaded from {}. The Stack tab shows candidates in the selected ELF; unresolved entries remain available. Check report paths when this folder contains multiple builds or targets.", build.root.display()));
            Ok(Loaded::Stack(report))
        });
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
                                let label = if in_use {
                                    egui::RichText::new(format!(
                                        "{} (in use)",
                                        display_path(&label)
                                    ))
                                    .strong()
                                } else {
                                    egui::RichText::new(display_path(&label))
                                };
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
        if let Some(artifact) = selected {
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
                        if ui.button("Use memory regions from this map").clicked() {
                            self.apply_map(path.clone());
                        }
                    }
                    "json" => {
                        if ui.button("Use this memory layout").clicked() {
                            self.configure(Some(path.clone()));
                        }
                    }
                    "su" if self.analysis.is_some()
                        && ui.button("View this stack report").clicked() =>
                    {
                        self.preview = None;
                        self.scan_stack(path.clone());
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
