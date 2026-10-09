use super::display::display_path;
use super::{egui, Explorer, Loaded};
use snout_core::{
    analyze_path,
    build::{detect_map_format, parse_map_regions, scan_folder, Artifact, ArtifactKind},
};
use std::path::PathBuf;

pub(super) struct MapWarning {
    pub path: PathBuf,
    pub firmware: String,
    pub reasons: Vec<String>,
    focus_requested: bool,
}

fn selected_map_options(text: &str) -> Result<super::AnalysisOptions, String> {
    if detect_map_format(text) == snout_core::map::MapFormat::LlvmLld {
        snout_core::map::parse_map_sections(text).map_err(|error| error.to_string())?;
        Ok(Default::default())
    } else {
        parse_map_regions(text).map_err(|error| error.to_string())
    }
}

pub(super) fn analyze_selected(
    build: &snout_core::build::BuildFolder,
    path: &std::path::Path,
    layout: Option<super::AnalysisOptions>,
    source: Option<String>,
) -> Result<(super::Analysis, Option<super::AnalysisOptions>, String), String> {
    if let Some(source) = source {
        if std::path::Path::new(&source)
            .extension()
            .is_some_and(|extension| extension.to_string_lossy().eq_ignore_ascii_case("map"))
        {
            let text = std::fs::read_to_string(&source).map_err(|error| error.to_string())?;
            let options = selected_map_options(&text)?;
            let mut analysis = analyze_path(path, &options).map_err(|error| error.to_string())?;
            snout_core::dependencies::import_map(&mut analysis, &text, &source);
            return Ok((analysis, Some(options), source));
        }
        let options = layout.unwrap_or_default();
        let analysis = analyze_path(path, &options).map_err(|error| error.to_string())?;
        return Ok((analysis, Some(options), source));
    }
    let analysis = snout_core::build::analyze_build_firmware(build, path, layout.as_ref())
        .map_err(|error| error.to_string())?;
    let source = build
        .matching_map(path)
        .filter(|map| {
            !analysis.options.regions.is_empty()
                || std::fs::read_to_string(map).ok().is_some_and(|text| {
                    detect_map_format(&text) == snout_core::map::MapFormat::LlvmLld
                        && selected_map_options(&text).is_ok()
                })
        })
        .map(|map| map.display().to_string())
        .unwrap_or_else(|| "ELF inference".into());
    Ok((analysis, layout, source))
}

/// Persist directory rules and explicit exceptions instead of a snapshot of files.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(super) struct StackSelection {
    pub paths: Vec<PathBuf>,
    pub excluded: Vec<PathBuf>,
    pub auto_directories: Vec<PathBuf>,
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
                            // The directory exclusion overrides ancestor/equal rules.
                            // Keep more specific choices for reports currently absent
                            // from the scan so they are restored when those files return.
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

    fn report_paths(&self, build: &snout_core::build::BuildFolder) -> Result<Vec<PathBuf>, String> {
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
    build: &snout_core::build::BuildFolder,
    selected: Option<StackSelection>,
) -> Result<super::LoadedStack, String> {
    if let Some(selection) = selected {
        let paths = selection.report_paths(build)?;
        let report =
            snout_core::stack::analyze_stack_files(analysis, paths).map_err(|e| e.to_string())?;
        Ok((report, Some(selection)))
    } else {
        super::stack_guess::guess(analysis, build)
    }
}

/// Stack candidates must describe the same ELF snapshot as the configured report.
pub(super) fn configured_report(
    options: super::AnalysisOptions,
    mut analysis: Option<super::Analysis>,
    source: String,
    build: Option<&snout_core::build::BuildFolder>,
    selection: Option<StackSelection>,
) -> Result<Loaded, String> {
    // Layout jobs reread the ELF after a rebuild, so report discovery must also
    // reflect files added or removed since the last build-folder scan.
    let build = build
        .filter(|_| analysis.is_some())
        .map(|build| scan_folder(&build.root).map(std::sync::Arc::new))
        .transpose()
        .map_err(|e| e.to_string())?;
    let stack = match (&mut analysis, build.as_deref()) {
        (Some(analysis), Some(build)) => match load_stack_reports(analysis, build, selection) {
            Ok(stack) => Some(stack),
            Err(error) => {
                analysis
                    .warnings
                    .push(format!("Stack reports could not be loaded: {error}"));
                None
            }
        },
        _ => None,
    };
    Ok(Loaded::Config(options, analysis, source, stack, build))
}

fn ui_map_selection(explorer: &Explorer, ui: &mut egui::Ui, clear_map: &mut bool) {
    ui.add_enabled_ui(explorer.receiver.is_none(), |ui| {
        if ui
            .radio(
                explorer.layout_source.is_empty() || explorer.layout_source == "ELF inference",
                "No map selected",
            )
            .on_hover_text(
                "Infer address ranges from the ELF; capacity and free space remain unknown.",
            )
            .clicked()
            && explorer.layout_source != "ELF inference"
        {
            *clear_map = true;
        }
    });
}

fn ui_map_actions(
    explorer: &Explorer,
    ui: &mut egui::Ui,
    selected_map: &mut Option<PathBuf>,
    autodetect_map: &mut bool,
) {
    ui.add_enabled_ui(explorer.receiver.is_none(), |ui| {
        ui.horizontal_wrapped(|ui| {
            if ui.button("Load map file...").clicked() {
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("Linker map", &["map"])
                    .pick_file()
                {
                    *selected_map = Some(path);
                }
            }
            if ui
                .button("Autodetect map file")
                .on_hover_text("Rerun automatic map matching for the selected ELF.")
                .clicked()
            {
                *autodetect_map = true;
            }
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
        if let Some(cache) = &mut self.browser_cache {
            cache.revision = self.report_revision.wrapping_sub(1);
        }
        // The old report no longer represents the saved selection, including on failure.
        self.stack = None;
        // Save the user's intent before analysis, even if the app closes during the job.
        self.job(move || {
            let paths = selection.report_paths(&build)?;
            let report = snout_core::stack::analyze_stack_files(&analysis, paths)
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
                if let Some(settings) = self.build_settings.get_mut(&build.root) {
                    settings.reset_choices();
                }
                self.remembered_firmware = None;
                self.pending_restore = None;
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
        if self.analysis.is_none() || self.receiver.is_some() {
            return;
        }
        match artifact.kind {
            ArtifactKind::Map => {
                if !self.map_in_use(&artifact.path) {
                    self.apply_map(artifact.path);
                }
            }
            ArtifactKind::StackUsage => {
                let mut selection = self.current_stack_selection();
                let paths = self
                    .build
                    .as_ref()
                    .map(|build| {
                        build
                            .artifacts
                            .iter()
                            .filter(|artifact| artifact.kind == ArtifactKind::StackUsage)
                            .map(|artifact| artifact.path.clone())
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                let checked = !selection.contains(&artifact.path);
                selection.set(&artifact.path, checked, &paths);
                self.select_stack_selection(selection);
            }
            ArtifactKind::Firmware => unreachable!(),
        }
    }

    pub(super) fn apply_map(&mut self, path: PathBuf) {
        self.apply_map_checked(path, false);
    }

    fn apply_map_checked(&mut self, path: PathBuf, ignore: bool) {
        let current_path = self.analysis.as_ref().map(|a| a.path.clone());
        let build = self.build.clone();
        let reports = current_path
            .as_ref()
            .and_then(|p| self.saved_stack_selection(std::path::Path::new(p)));
        self.job_observing(current_path.as_ref().map(PathBuf::from), move || {
            let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
            if !ignore {
                if let Some(firmware) = &current_path {
                    let bytes = std::fs::read(firmware).map_err(|error| error.to_string())?;
                    let reasons = snout_core::build::map_match_issues(&bytes, &text);
                    if !reasons.is_empty() {
                        return Ok(Loaded::MapWarning(MapWarning {
                            path,
                            firmware: firmware.clone(),
                            reasons,
                            focus_requested: false,
                        }));
                    }
                }
            }
            let options = selected_map_options(&text)?;
            let mut analysis = current_path.map(|p| {
                let mut a = analyze_path(p, &options)?;
                a.warnings.push(format!("Linker map selected from {} ({}). Flash/RAM roles are inferred from names and attributes; verify this map belongs to the selected firmware.", path.display(), detect_map_format(&text).label()));
                Ok::<_, snout_core::Error>(a)
            }).transpose().map_err(|e| e.to_string())?;
            if let Some(analysis) = &mut analysis {
                snout_core::dependencies::import_map(analysis, &text, &path.display().to_string());
            }
            configured_report(options, analysis, path.display().to_string(), build.as_deref(), reports)
        });
    }
    pub(super) fn show_map_warning(&mut self, ctx: &egui::Context) {
        let Some(warning) = &mut self.map_warning else {
            return;
        };
        let request_focus = !warning.focus_requested;
        warning.focus_requested = true;
        let mut ignore = false;
        let mut revert = false;
        let id = egui::Id::new("map_mismatch");
        let mut area = egui::Modal::default_area(id);
        if let Some(rect) = ctx.memory(|memory| memory.area_rect(id)) {
            // Stabilize the centered origin before Area rounds to physical pixels.
            // Fractional-scale layout noise can otherwise cause a one-pixel oscillation.
            let scale = ctx.pixels_per_point();
            let size = (rect.size() * scale * 64.0).round() / (scale * 64.0);
            area = area.anchor(
                egui::Align2::LEFT_TOP,
                (ctx.screen_rect().size() - size) * 0.5,
            );
        }
        let response = egui::Modal::new(id)
            .area(area)
            .backdrop_color(egui::Color32::from_black_alpha(160))
            .frame(egui::Frame::window(&ctx.style()).inner_margin(egui::Margin::same(20.0)))
            .show(ctx, |ui| {
                ui.set_width((ctx.screen_rect().width() - 64.0).clamp(240.0, 560.0));
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                ui.spacing_mut().item_spacing = egui::vec2(10.0, 12.0);
                ui.spacing_mut().button_padding = egui::vec2(14.0, 8.0);
                ui.heading("Map may not match firmware");
                ui.label("This map could not be confirmed against the selected ELF. Using it may show incorrect memory capacities or dependencies.");
                ui.separator();
                for (label, path) in [("Map", warning.path.display().to_string()), ("ELF", warning.firmware.clone())] {
                    let filename = std::path::Path::new(&path).file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_else(|| display_path(&path).into_owned());
                    ui.add(egui::Label::new(egui::RichText::new(format!("{label}: {filename}")).strong()).truncate()).on_hover_text(path);
                }
                let color = ui.visuals().warn_fg_color;
                egui::Frame::none()
                    .fill(color.linear_multiply(0.08))
                    .stroke(egui::Stroke::new(1.0_f32, color.linear_multiply(0.35)))
                    .rounding(6.0)
                    .inner_margin(12.0)
                    .show(ui, |ui| {
                        ui.colored_label(color, egui::RichText::new("Why it does not match").strong());
                        egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
                            for reason in &warning.reasons { ui.label(format!("• {reason}")); }
                        });
                    });
                ui.weak("Revert keeps your previous map selection and report.");
                ui.horizontal(|ui| {
                    let revert_button = ui.button("Revert");
                    if request_focus { revert_button.request_focus(); }
                    revert = revert_button.clicked();
                    ignore = ui.button("Ignore").on_hover_text("Use this map despite the warning").clicked();
                });
            });
        // Escape cancels. Backdrop clicks never accept or dismiss the warning.
        if revert || response.is_top_modal && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.resolve_map_warning(false);
        } else if ignore {
            self.resolve_map_warning(true);
        }
    }

    pub(super) fn resolve_map_warning(&mut self, ignore: bool) {
        if let Some(warning) = self.map_warning.take() {
            if ignore {
                self.apply_map_checked(warning.path, true);
            }
        }
    }

    #[cfg(test)]
    pub(super) fn build_browser(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("build_artifacts")
            .default_width(260.0)
            .width_range(180.0..=500.0)
            .show(ctx, |ui| self.build_files_contents(ui, false));
    }

    pub(super) fn build_files(&mut self, ui: &mut egui::Ui) {
        self.build_files_contents(ui, true);
    }

    fn build_files_contents(&mut self, ui: &mut egui::Ui, current_only: bool) {
        let Some(build) = self.build.clone() else {
            ui.label("Open a build folder to select map and stack usage files.");
            if ui
                .add_enabled(
                    self.receiver.is_none(),
                    egui::Button::new("Open build folder..."),
                )
                .clicked()
            {
                self.pick_build();
            }
            return;
        };
        if !build.warnings.is_empty() {
            ui.collapsing(format!("{} scan notes", build.warnings.len()), |ui| {
                for note in &build.warnings {
                    ui.label(note);
                }
            });
        }
        let mut selected = None;
        let mut selected_map = None;
        let mut clear_map = false;
        let mut autodetect_map = false;
        let previous = self.browser_cache.take();
        let firmware_changed = previous
            .as_ref()
            .and_then(|cache| cache.analysis.as_ref())
            .map(|analysis| &analysis.path)
            != self.analysis.as_ref().map(|analysis| &analysis.path);
        let closed = previous
            .as_ref()
            .map(|c| c.closed.clone())
            .unwrap_or_default();
        let mut cache = previous
            .filter(|cache| {
                std::sync::Arc::ptr_eq(&cache.build, &build)
                    && cache.revision == self.report_revision
                    && match (&cache.analysis, &self.analysis) {
                        (Some(a), Some(b)) => std::sync::Arc::ptr_eq(a, b),
                        (None, None) => true,
                        _ => false,
                    }
            })
            .unwrap_or_else(|| {
                let mut cache = super::artifact_browser::BrowserCache::new(
                    build.clone(),
                    self.analysis.clone(),
                    self.report_revision,
                    self.current_stack_selection(),
                );
                cache.closed = closed;
                cache
            });
        let mut report_change = None;
        if current_only && self.analysis.is_some() {
            ui.label("Choose supporting files for the ELF selected in the header. Selections are saved automatically.");
            ui.add_space(8.0);
            ui.add(
                egui::TextEdit::singleline(&mut self.artifact_search)
                    .hint_text("Find a map or stack report...")
                    .desired_width(f32::INFINITY),
            );
            cache.filter(&self.artifact_search);
            ui.add_space(12.0);
            ui.columns(2, |columns| {
                egui::Frame::group(columns[0].style()).inner_margin(12.0).show(&mut columns[0], |ui| {
                    ui.heading("Map file");
                    ui.weak("One map supplies memory regions and symbol dependencies.");
                    ui.add_space(8.0);
                    ui_map_selection(self, ui, &mut clear_map);
                    ui.separator();
                    egui::ScrollArea::vertical().id_salt("map_choices")
                        .max_height((ui.available_height() - 90.0).max(60.0))
                        .show(ui, |ui| {
                            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
                            if cache.artifacts[1].is_empty() {
                                ui.weak(if self.artifact_search.is_empty() { "No map files found." } else { "No maps match this search." });
                            }
                            let height = ui.spacing().interact_size.y;
                            super::artifact_browser::show_rows(ui, height, cache.artifacts[1].len(), |ui, range| {
                                for row in range {
                                    let index = cache.artifacts[1][row];
                                    let map = &build.artifacts[index];
                                    let in_use = self.map_in_use(&map.path);
                                    if ui.add_enabled(self.receiver.is_none(), egui::RadioButton::new(in_use, display_path(&cache.labels[index])))
                                        .on_hover_text(display_path(&map.path.to_string_lossy())).clicked() && !in_use {
                                        selected_map = Some(map.path.clone());
                                    }
                                }
                            });
                        });
                    ui.add_space(8.0);
                    ui_map_actions(self, ui, &mut selected_map, &mut autodetect_map);
                    ui.add_space(8.0);
                    if self.options.regions.is_empty() {
                        ui.weak("Physical capacities are unknown.");
                    } else {
                        ui.small(format!("{} memory regions loaded", self.options.regions.len()));
                    }
                    if self.layout_source != "ELF inference" {
                        let source = std::path::Path::new(&self.layout_source);
                        let label = source.strip_prefix(&build.root).unwrap_or(source).display().to_string();
                        ui.add(egui::Label::new(format!("Selected: {}", display_path(&label))).truncate())
                            .on_hover_text(display_path(&self.layout_source));
                    }

                });
                egui::Frame::group(columns[1].style()).inner_margin(12.0).show(&mut columns[1], |ui| {
                    ui.heading("Stack reports");
                    ui.weak("Select files or entire folders. Frames are local, not call-chain totals.");
                    let count = cache.paths.iter().filter(|path| cache.reports.contains(path)).count();
                    ui.add_space(8.0);
                    ui.small(format!("{count} of {} reports selected", cache.paths.len()));
                    ui.separator();
                    egui::ScrollArea::vertical().id_salt("su_choices")
                        .max_height((ui.available_height() - 30.0).max(140.0))
                        .show(ui, |ui| {
                            if cache.paths.is_empty() { ui.weak("No stack reports found."); }
                            else { ui.add_enabled_ui(self.receiver.is_none(), |ui| { report_change = cache.report_ui(ui); }); }
                        });
                });
            });
        } else {
            ui.strong("BUILD FILES");
            ui.add(
                egui::TextEdit::singleline(&mut self.artifact_search)
                    .hint_text("Find file...")
                    .desired_width(f32::INFINITY),
            );
            cache.filter(&self.artifact_search);
            ui.small(format!("{} compatible files", build.artifacts.len()));
            ui.separator();
            egui::ScrollArea::vertical().show(ui, |ui| {
                    if build.artifacts.is_empty() {
                        ui.label("No compatible files found in this folder or its subfolders.");
                    }
                    // Record newly loaded firmware's expansion even when the search
                    // hides its row. The cache consumes the change on this frame.
                    if firmware_changed {
                        if let Some(analysis) = &self.analysis {
                            // Match the row's persistent ID without creating a
                            // one-frame UI scope that shifts subsequent widget IDs.
                            let id = ui.id()
                                .with(egui::Id::new(std::path::Path::new(&analysis.path)))
                                .with("firmware_files");
                            let mut state = egui::collapsing_header::CollapsingState::load_with_default_open(
                                ui.ctx(), id, true);
                            state.set_open(true);
                            state.store(ui.ctx());
                        }
                    }
                    let supporting_match = !self.artifact_search.is_empty()
                        && (!cache.artifacts[1].is_empty() || !cache.artifacts[2].is_empty());
                    let has_firmware = build.artifacts.iter().any(|a| a.kind == ArtifactKind::Firmware);
                    let firmware_indices: Vec<_> = if supporting_match {
                        build.artifacts.iter().enumerate()
                            .filter(|(_, artifact)| artifact.kind == ArtifactKind::Firmware)
                            .map(|(index, _)| index).collect()
                    } else {
                        cache.artifacts[0].clone()
                    };
                    if firmware_indices.is_empty() && !build.artifacts.is_empty() {
                        ui.label(if has_firmware {
                            "No firmware matches this search."
                        } else {
                            "No firmware binaries found. Open a build folder containing an ELF to select its map and stack usage files."
                        });
                    }
                    // Supporting files are listed without exposing selection before an ELF is loaded.
                    if !has_firmware {
                        for (group, kind) in [(1, ArtifactKind::Map), (2, ArtifactKind::StackUsage)] {
                            if cache.artifacts[group].is_empty() {
                                continue;
                            }
                            egui::CollapsingHeader::new(format!("{} ({})", kind.label(), cache.artifacts[group].len()))
                                .default_open(true).show(ui, |ui| {
                                    let height = ui.spacing().interact_size.y.max(ui.text_style_height(&egui::TextStyle::Body));
                                    super::artifact_browser::show_rows(ui, height, cache.artifacts[group].len(), |ui, range| {
                                            for row in range {
                                                let index = cache.artifacts[group][row];
                                                let artifact = &build.artifacts[index];

                                                if ui.add_enabled(false,
                                                    egui::Button::new(display_path(&cache.labels[index]))
                                                        .frame(false).truncate())
                                                    .on_hover_text(display_path(&artifact.path.to_string_lossy())).clicked() {
                                                    selected = Some(artifact.clone());
                                                }
                                            }
                                        });
                                });
                        }
                    }
                    for index in firmware_indices {
                        let artifact = &build.artifacts[index];
                        if current_only && !self.analysis.as_ref().is_some_and(|a| std::path::Path::new(&a.path) == artifact.path) { continue; }
                        let active = self.analysis.as_ref().is_some_and(|a|
                            std::path::Path::new(&a.path) == artifact.path);
                        // Reserve collapsed offscreen rows even during background jobs.
                        // Previously opened rows still need their body/animation height.
                        let height = ui.spacing().interact_size.y
                            .max(ui.text_style_height(&egui::TextStyle::Button))
                            .max(ui.spacing().icon_width);
                        let row_rect = egui::Rect::from_min_size(ui.next_widget_position(),
                            egui::vec2(ui.available_width(), height));
                        if !active && !ui.is_rect_visible(row_rect) {
                            // push_id hashes its salt into an Id before deriving the
                            // child UI's ID. Match that scope when looking up state.
                            let id = ui.id().with(egui::Id::new(&artifact.path)).with("firmware_files");
                            let expanded = egui::collapsing_header::CollapsingState::load(ui.ctx(), id)
                                .is_some_and(|mut state| {
                                    if self.receiver.is_none() && state.is_open() {
                                        state.set_open(false);
                                        state.store(ui.ctx());
                                    }
                                    state.openness(ui.ctx()) > 0.0
                                });
                            if !expanded {
                                ui.allocate_space(row_rect.size());
                                continue;
                            }
                        }
                        ui.push_id(&artifact.path, |ui| {
                            let mut state = egui::collapsing_header::CollapsingState::load_with_default_open(
                                ui.ctx(), ui.make_persistent_id("firmware_files"), active);
                            // Only the loaded firmware exposes supporting-file selection.
                            if !active && self.receiver.is_none() {
                                state.set_open(false);
                            }
                            let was_open = state.is_open();
                            let mut clicked = false;
                            let mut header = state.show_header(ui, |ui| {
                                let response = ui.add_enabled(self.receiver.is_none(),
                                    egui::Button::new(display_path(&cache.labels[index]))
                                        .frame(false).selected(active).truncate())
                                    .on_hover_text(display_path(&artifact.path.to_string_lossy()));
                                if response.clicked() {
                                    clicked = true;
                                    if active {
                                                            } else {
                                        selected = Some(artifact.clone());
                                    }
                                }
                            });
                            if clicked {
                                header.set_open(true);
                            }
                            if header.is_open() && !was_open && !active && self.receiver.is_none() {
                                selected = Some(artifact.clone());
                            }
                            header.body(|ui| {
                                if !active {
                                    ui.small("Loading firmware…");
                                    return;
                                }
                                egui::CollapsingHeader::new(format!("Map file ({})", cache.artifacts[1].len()))
                                    .id_salt("maps").default_open(true).show(ui, |ui| {
                                        ui_map_selection(self, ui, &mut clear_map);
                                        if cache.artifacts[1].is_empty() {
                                            ui.small("No map files found.");
                                        }
                                        let height = ui.spacing().interact_size.y;
                                        super::artifact_browser::show_rows(ui, height, cache.artifacts[1].len(), |ui, range| {
                                                for row in range {
                                                    let map_index = cache.artifacts[1][row];
                                                    let map = &build.artifacts[map_index];
                                                    let in_use = self.map_in_use(&map.path);
                                                    ui.horizontal(|ui| {
                                                        if ui.add_enabled(self.receiver.is_none(), egui::RadioButton::new(in_use, ""))
                                                            .on_hover_text("Use memory regions from this map for this firmware")
                                                            .clicked() && !in_use {
                                                            selected_map = Some(map.path.clone());
                                                        }

                                                        if ui.add_enabled(self.receiver.is_none(),
                                                            egui::Button::new(display_path(&cache.labels[map_index]))
                                                                .frame(false).truncate())
                                                            .on_hover_text(format!("Select map\n{}", display_path(&map.path.to_string_lossy())))
                                                            .clicked() {
                                                            selected = Some(map.clone());
                                                        }
                                                    });
                                                }
                                            });
                                        ui_map_actions(self, ui, &mut selected_map, &mut autodetect_map);
                                    });
                                egui::CollapsingHeader::new(format!("Stack usage files ({})", cache.artifacts[2].len()))
                                    .id_salt("reports").default_open(false).show(ui, |ui| {
                                        if cache.artifacts[2].is_empty() {
                                            ui.small("No stack usage files found.");
                                        } else {
                                            ui.add_enabled_ui(self.receiver.is_none(), |ui| {
                                                report_change = cache.report_ui(ui);
                                            });
                                        }
                                    });
                            });
                        });
                    }
                });
        }
        if let Some((path, checked)) = report_change {
            cache.reports.set(&path, checked, &cache.paths);
            let reports = cache.reports.clone();
            self.browser_cache = Some(cache);
            self.select_stack_selection(reports);
        } else {
            self.browser_cache = Some(cache);
            if clear_map {
                self.configure(None);
            } else if autodetect_map {
                self.discover_layout();
            } else if let Some(path) = selected_map {
                self.apply_map(path);
            } else if let Some(artifact) = selected {
                self.select_artifact(artifact);
            }
        }
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
    fn missing_checked_sibling_stays_selected_when_last_visible_file_is_unchecked() {
        let root = PathBuf::from("build");
        let first = root.join("a.su");
        let missing = root.join("b.su");
        let mut selected = StackSelection::default();
        let paths = [first.clone(), missing.clone()];
        selected.set(&first, true, &paths);
        selected.set(&missing, true, &paths);
        selected.set(&first, false, std::slice::from_ref(&first));
        assert!(!selected.contains(&first));
        assert!(selected.contains(&missing));
        assert!(!selected.contains(&root.join("new.su")));
        let restored: StackSelection =
            serde_json::from_value(serde_json::to_value(&selected).unwrap()).unwrap();
        assert!(restored.contains(&missing));
    }

    #[test]
    fn stack_selection_uses_only_the_current_rules_format() {
        assert!(
            serde_json::from_value::<StackSelection>(serde_json::json!(["build/a.su"])).is_err()
        );
        let selection = StackSelection::default();
        let restored: StackSelection =
            serde_json::from_value(serde_json::to_value(&selection).unwrap()).unwrap();
        assert_eq!(restored, selection);
    }
}
