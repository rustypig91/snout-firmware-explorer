use super::{egui, Explorer, Loaded};
use firmware_analysis_core::{
    analyze_path,
    build::{parse_map_regions, scan_folder, Artifact, ArtifactKind},
};
use std::{io::Read, path::PathBuf};

impl Explorer {
    pub(super) fn load_build_stack(&mut self) {
        let (Some(build), Some(analysis)) = (self.build.clone(), self.analysis.clone()) else {
            return;
        };
        self.job(move || {
            let paths = build.artifacts.iter().filter(|a| a.kind == ArtifactKind::StackUsage).map(|a| a.path.clone()).collect();
            let mut report = firmware_analysis_core::stack::analyze_stack_files(&analysis, paths).map_err(|e| e.to_string())?;
            report.warnings.push(format!("All reports under {} are shown. Check paths when this folder contains multiple builds or targets.", build.root.display()));
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
    fn select_artifact(&mut self, artifact: Artifact) {
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
    fn apply_map(&mut self, path: PathBuf) {
        let current_path = self.analysis.as_ref().map(|a| a.path.clone());
        self.job(move || {
            let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
            let options = parse_map_regions(&text).map_err(|e| e.to_string())?;
            let analysis = current_path.map(|p| {
                let mut a = analyze_path(p, &options)?;
                a.warnings.push(format!("Memory regions selected from {}. Flash/RAM roles are inferred from names and attributes; verify this map belongs to the selected firmware.", path.display()));
                Ok::<_, firmware_analysis_core::Error>(a)
            }).transpose().map_err(|e| e.to_string())?;
            Ok(Loaded::Config(options, analysis))
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
                        ArtifactKind::LinkerScript,
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
                        .default_open(kind == ArtifactKind::Firmware)
                        .show(ui, |ui| {
                            for artifact in artifacts {
                                let active = self
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
                                if ui
                                    .add_enabled(
                                        self.receiver.is_none(),
                                        egui::Button::new(&label)
                                            .frame(false)
                                            .selected(active)
                                            .wrap(),
                                    )
                                    .on_hover_text(artifact.path.display().to_string())
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
        ui.label(path.display().to_string());
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
                        if ui.button("Use memory regions from this map").clicked() {
                            self.apply_map(path.clone());
                        }
                    }
                    "json" => {
                        if ui.button("Use this memory layout").clicked() {
                            self.configure(Some(path.clone()));
                        }
                    }
                    "su" if self.analysis.is_some() => {
                        if ui.button("View this stack report").clicked() {
                            self.preview = None;
                            self.scan_stack(path.clone());
                        }
                    }
                    "ld" | "lds" => {
                        ui.weak(
                            "Linker script preview; region import uses the resolved linker map.",
                        );
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
