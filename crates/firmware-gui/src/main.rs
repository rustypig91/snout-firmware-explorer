#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod stack_guess;
mod startup;
mod update;
mod update_ui;
mod wake;
use wake::Wake;
static RESTART_PATH: std::sync::Mutex<Option<PathBuf>> = std::sync::Mutex::new(None);
mod artifact_browser;
mod dependencies;
mod display;
mod insights;
mod overview;
#[cfg(test)]
mod performance;
mod pie;
mod preferences;
mod shell;
#[cfg(test)]
mod tests;
mod views;
#[cfg(target_os = "linux")]
mod window_theme;
mod workspace;
use eframe::egui;
use firmware_analysis_core::{
    analyze_path,
    compare::{compare, Comparison},
    stack::StackReport,
    Analysis, AnalysisOptions,
};
use std::{
    path::PathBuf,
    sync::{mpsc, Arc},
    time::Duration,
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum View {
    Overview,
    Files,
    Symbols,
    Sections,
    MemoryMap,
    Dependencies,
    Stack,
    Compare,
}
impl View {
    const ALL: [Self; 8] = [
        Self::Overview,
        Self::Files,
        Self::Symbols,
        Self::Sections,
        Self::MemoryMap,
        Self::Dependencies,
        Self::Stack,
        Self::Compare,
    ];
    fn label(self) -> &'static str {
        match self {
            Self::Overview => "Overview",
            Self::Files => "Files",
            Self::Symbols => "Symbols",
            Self::Sections => "Sections",
            Self::MemoryMap => "Memory map",
            Self::Dependencies => "Dependencies",
            Self::Stack => "Stack",
            Self::Compare => "Compare",
        }
    }
}
// Session-only settings for the controls shared by the data tabs.
#[derive(Clone)]
struct TabOptions {
    search: String,
    sort_column: usize,
    descending: bool,
    tree: bool,
    selected_file: Option<String>,
    selected_region: Option<usize>,
    kind_filter: String,
}
impl TabOptions {
    fn new(view: View) -> Self {
        Self {
            search: String::new(),
            sort_column: 1,
            descending: !matches!(view, View::MemoryMap),
            tree: false,
            selected_file: None,
            selected_region: None,
            kind_filter: "All".into(),
        }
    }
}
type LoadedStack = (StackReport, Option<workspace::StackSelection>);
type Refreshed = (
    firmware_analysis_core::build::BuildFolder,
    Analysis,
    Option<LoadedStack>,
    Option<AnalysisOptions>,
    String,
);
enum Loaded {
    ResetBuildSettings(Box<Loaded>),
    Refresh(Box<Refreshed>),
    Firmware(
        Analysis,
        Option<LoadedStack>,
        Option<AnalysisOptions>,
        String,
    ),
    Build(firmware_analysis_core::build::BuildFolder),
    Text(PathBuf, String),
    Baseline(Analysis),
    SelectedStack(StackReport, workspace::StackSelection),
    Config(
        AnalysisOptions,
        Option<Analysis>,
        String,
        Option<LoadedStack>,
        Option<Arc<firmware_analysis_core::build::BuildFolder>>,
    ),
    Dependencies(Analysis, PathBuf),
}
type JobResult = Result<Loaded, String>;
#[derive(Clone)]
struct RememberedFirmware {
    folder: PathBuf,
    path: PathBuf,
    layout: Option<AnalysisOptions>,
    source: String,
}
#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
struct BuildSettings {
    firmware: Option<PathBuf>,
    layouts: std::collections::BTreeMap<PathBuf, SavedLayout>,
    #[serde(default)]
    dependency_maps: std::collections::BTreeMap<PathBuf, PathBuf>,
    #[serde(default)]
    stack_reports: std::collections::BTreeMap<PathBuf, workspace::StackSelection>,
}
#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct SavedLayout {
    options: AnalysisOptions,
    source: String,
}
struct Explorer {
    browser_cache: Option<artifact_browser::BrowserCache>,
    report_revision: u64,
    table_cache: views::TableCache,
    preferences_file: Option<PathBuf>,
    analysis: Option<Arc<Analysis>>,
    graph_view: dependencies::GraphView,
    build: Option<Arc<firmware_analysis_core::build::BuildFolder>>,
    artifact_search: String,
    preview: Option<(PathBuf, String)>,
    layout_override: Option<AnalysisOptions>,
    comparison: Option<Comparison>,
    baseline: Option<Arc<Analysis>>,
    stack: Option<StackReport>,
    stack_show_unresolved: bool,
    options: AnalysisOptions,
    receiver: Option<mpsc::Receiver<JobResult>>,
    view: View,
    tab_options: [TabOptions; 8],
    search: String,
    selected_file: Option<String>,
    selected_region: Option<usize>,
    overview_section: Option<usize>,
    overview_unit: Option<pie::UnitKey>,
    overview_scroll_top: bool,
    sort_column: usize,
    descending: bool,
    error: Option<String>,
    tree: bool,
    details: Option<(String, String)>,
    show_notes: bool,
    show_about: bool,
    visible_rows: usize,
    kind_filter: String,
    comparison_group: usize,
    overview_metric: overview::Metric,
    contributor_ram: bool,
    region_cache: Vec<firmware_analysis_core::regions::RegionUsage>,
    region_cache_key: usize,
    top_files: [Vec<usize>; 2],
    top_symbols: [Vec<usize>; 2],
    layout_source: String,
    updates: update_ui::Updates,
    pending_restore: Option<(PathBuf, Option<AnalysisOptions>, String)>,
    remembered_firmware: Option<RememberedFirmware>,
    build_settings: std::collections::BTreeMap<PathBuf, BuildSettings>,
}
impl Default for Explorer {
    fn default() -> Self {
        Self {
            browser_cache: None,
            report_revision: 0,
            table_cache: Default::default(),
            preferences_file: None,
            analysis: None,
            graph_view: Default::default(),
            build: None,
            artifact_search: String::new(),
            preview: None,
            layout_override: None,
            comparison: None,
            baseline: None,
            stack: None,
            stack_show_unresolved: false,
            options: AnalysisOptions::default(),
            receiver: None,
            view: View::Overview,
            tab_options: View::ALL.map(TabOptions::new),
            search: String::new(),
            selected_file: None,
            selected_region: None,
            overview_section: None,
            overview_unit: None,
            overview_scroll_top: false,
            sort_column: 1,
            descending: true,
            error: None,
            tree: false,
            details: None,
            show_notes: false,
            show_about: false,
            visible_rows: 0,
            kind_filter: "All".into(),
            comparison_group: 0,
            overview_metric: overview::Metric::Flash,
            contributor_ram: false,
            region_cache: Vec::new(),
            region_cache_key: 0,
            top_files: Default::default(),
            top_symbols: Default::default(),
            layout_source: String::new(),
            pending_restore: None,
            remembered_firmware: None,
            build_settings: Default::default(),
            updates: update_ui::Updates::default(),
        }
    }
}
impl Explorer {
    fn job(&mut self, task: impl FnOnce() -> JobResult + Send + 'static) {
        if self.receiver.is_some() {
            return;
        }
        let (sender, receiver) = mpsc::channel();
        self.receiver = Some(receiver);
        self.error = None;
        std::thread::spawn(move || {
            let _ = sender.send(task());
        });
    }
    fn open(&mut self, path: PathBuf) {
        let layout = self.saved_layout(&path).map(|saved| saved.options.clone());
        self.open_with_layout(path, layout);
    }
    fn discover_layout(&mut self) {
        if let Some(a) = &self.analysis {
            self.open_with_layout(PathBuf::from(&a.path), None);
        } else {
            self.layout_override = None;
            self.options = AnalysisOptions::default();
        }
    }
    fn open_with_layout(&mut self, path: PathBuf, layout: Option<AnalysisOptions>) {
        self.load_firmware(path, layout, false);
    }
    fn load_firmware(
        &mut self,
        path: PathBuf,
        layout: Option<AnalysisOptions>,
        reset_settings: bool,
    ) {
        let Some(build) = self.build.clone() else {
            return;
        };
        let source = layout.as_ref().map(|_| {
            self.saved_layout(&path)
                .map(|saved| saved.source.clone())
                .unwrap_or_else(|| self.layout_source.clone())
        });
        let dependency_map = if reset_settings {
            None
        } else {
            self.saved_dependency_map(&path)
        };
        let reports = if reset_settings {
            None
        } else {
            self.saved_stack_selection(&path)
        };
        self.job(move || {
            let (mut analysis, layout, source) =
                workspace::analyze_selected(&build, &path, layout, source)?;
            if let Some(map) = dependency_map {
                workspace::read_dependency_map(&mut analysis, &map);
            }
            let stack = match workspace::load_stack_reports(&analysis, &build, reports) {
                Ok(report) => Some(report),
                Err(e) => {
                    analysis
                        .warnings
                        .push(format!("Stack reports could not be loaded: {e}"));
                    None
                }
            };
            let loaded = Loaded::Firmware(analysis, stack, layout, source);
            Ok(if reset_settings {
                Loaded::ResetBuildSettings(Box::new(loaded))
            } else {
                loaded
            })
        });
    }
    fn configure(&mut self, path: Option<PathBuf>) {
        let dependency_map = self.dependency_map_for_reload();
        let current_path = self.analysis.as_ref().map(|a| a.path.clone());
        let build = self.build.clone();
        let reports = current_path
            .as_ref()
            .and_then(|p| self.saved_stack_selection(std::path::Path::new(p)));
        self.job(move || {
            let source = path
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "ELF inference".into());
            let options = match path {
                Some(path) => firmware_analysis_core::map::parse_map_regions(
                    &std::fs::read_to_string(path).map_err(|e| e.to_string())?,
                )
                .map_err(|e| e.to_string())?,
                None => AnalysisOptions::default(),
            };
            firmware_analysis_core::validate_options(&options).map_err(|e| e.to_string())?;
            let mut analysis = current_path
                .map(|p| analyze_path(p, &options))
                .transpose()
                .map_err(|e| e.to_string())?;
            if let (Some(analysis), Some(map)) = (&mut analysis, dependency_map) {
                workspace::read_dependency_map(analysis, &map);
            }
            workspace::configured_report(options, analysis, source, build.as_deref(), reports)
        });
    }
    fn pick_baseline(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("ELF firmware", &["elf", "axf", "out"])
            .add_filter("All files", &["*"])
            .pick_file()
        {
            if self.analysis.is_some() {
                let options = self.options.clone();
                self.job(move || {
                    analyze_path(path, &options)
                        .map(Loaded::Baseline)
                        .map_err(|e| e.to_string())
                });
            }
        }
    }
    fn poll(&mut self) {
        match self.receiver.as_ref().map(|r| r.try_recv()) {
            Some(Ok(result)) => {
                self.report_revision = self.report_revision.wrapping_add(1);
                self.table_cache = Default::default();
                self.receiver = None;
                self.region_cache_key = 0;
                let refreshed = matches!(&result, Ok(Loaded::Refresh(_)));
                let result = result.map(|loaded| match loaded {
                    Loaded::ResetBuildSettings(loaded) => {
                        if let Some(build) = &self.build {
                            self.build_settings.remove(&build.root);
                        }
                        self.remembered_firmware = None;
                        self.pending_restore = None;
                        *loaded
                    }
                    Loaded::Refresh(result) => {
                        let (build, a, stack, layout, source) = *result;
                        self.build = Some(Arc::new(build));
                        Loaded::Firmware(a, stack, layout, source)
                    }
                    other => other,
                });
                match result {
                    Ok(Loaded::ResetBuildSettings(_)) => unreachable!(),
                    Ok(Loaded::Refresh(_)) => unreachable!(),
                    Ok(Loaded::Build(build)) => {
                        self.build = Some(Arc::new(build));
                        self.analysis = None;
                        self.graph_view = Default::default();
                        self.preview = None;
                        self.options = AnalysisOptions::default();
                        self.layout_override = None;
                        self.comparison = None;
                        self.baseline = None;
                        self.stack = None;
                        self.details = None;
                        self.clear_firmware_filters();
                        self.overview_section = None;
                        self.overview_unit = None;
                        self.artifact_search.clear();
                        self.visible_rows = 0;
                        let restore = self
                            .pending_restore
                            .take()
                            .or_else(|| {
                                let build = self.build.as_ref()?;
                                let settings = self.build_settings.get(&build.root)?;
                                let path = settings.firmware.as_ref()?;
                                build
                                    .artifacts
                                    .iter()
                                    .any(|a| {
                                        a.kind
                                            == firmware_analysis_core::build::ArtifactKind::Firmware
                                            && &a.path == path
                                    })
                                    .then(|| {
                                        let saved = settings.layouts.get(path);
                                        (
                                            path.clone(),
                                            saved.map(|s| s.options.clone()),
                                            saved.map(|s| s.source.clone()).unwrap_or_default(),
                                        )
                                    })
                            })
                            .or_else(|| {
                                let remembered = self.remembered_firmware.as_ref()?;
                                let build = self.build.as_ref()?;
                                (remembered.folder == build.root
                                    && remembered.path.is_file()
                                    && build.artifacts.iter().any(|artifact| {
                                        artifact.kind
                                            == firmware_analysis_core::build::ArtifactKind::Firmware
                                            && artifact.path == remembered.path
                                    }))
                                .then(|| {
                                    (
                                        remembered.path.clone(),
                                        remembered.layout.clone(),
                                        remembered.source.clone(),
                                    )
                                })
                            });
                        if let Some((path, layout, source)) = restore {
                            self.layout_source = source;
                            self.open_with_layout(path, layout);
                        }
                    }
                    Ok(Loaded::Text(path, text)) => {
                        self.change_view(View::Overview);
                        self.preview = Some((path, text));
                    }
                    Ok(Loaded::Firmware(a, stack, layout, source)) => {
                        self.layout_source = source;
                        self.layout_override = layout;
                        self.options = a.options.clone();
                        self.preview = None;
                        self.analysis = Some(Arc::new(a));
                        self.graph_view = Default::default();
                        if refreshed {
                            self.comparison = self
                                .baseline
                                .as_ref()
                                .map(|old| compare(old, self.analysis.as_ref().unwrap()));
                        } else {
                            self.comparison = None;
                            self.baseline = None;
                        }
                        self.replace_stack(stack);
                        self.clear_firmware_filters();
                        self.overview_section = None;
                        self.overview_unit = None;
                        self.details = None;
                        self.visible_rows = 0;
                    }
                    Ok(Loaded::Dependencies(analysis, map)) => {
                        if let Some(build) = &self.build {
                            self.build_settings
                                .entry(build.root.clone())
                                .or_default()
                                .dependency_maps
                                .insert(PathBuf::from(&analysis.path), map);
                        }
                        self.analysis = Some(Arc::new(analysis));
                        self.graph_view = Default::default();
                        self.preview = None;
                        self.change_view(View::Dependencies);
                    }
                    Ok(Loaded::Baseline(old)) => {
                        self.comparison =
                            self.analysis.as_ref().map(|current| compare(&old, current));
                        self.baseline = Some(Arc::new(old));
                        self.change_view(View::Compare);
                    }
                    Ok(Loaded::SelectedStack(s, paths)) => {
                        if let (Some(build), Some(analysis)) = (&self.build, &self.analysis) {
                            self.build_settings
                                .entry(build.root.clone())
                                .or_default()
                                .stack_reports
                                .insert(PathBuf::from(&analysis.path), paths);
                        }
                        self.stack = Some(s);
                        self.change_view(View::Stack);
                    }
                    Ok(Loaded::Config(options, analysis, source, stack, build)) => {
                        self.layout_source = source;
                        self.details = None;
                        self.clear_region_filters();
                        self.overview_section = None;
                        self.overview_unit = None;
                        self.layout_override = Some(options.clone());
                        self.options = options;
                        self.analysis = analysis.map(Arc::new);
                        if let Some(build) = build {
                            self.build = Some(build);
                        }
                        self.replace_stack(stack);
                        self.graph_view = Default::default();
                        self.preview = None;
                        self.comparison = None;
                        self.baseline = None;
                    }
                    Err(error) => self.error = Some(error),
                }
                if let (Some(build), Some(analysis)) = (&self.build, &self.analysis) {
                    let settings = self.build_settings.entry(build.root.clone()).or_default();
                    settings.firmware = Some(PathBuf::from(&analysis.path));
                    if let Some(options) = &self.layout_override {
                        settings.layouts.insert(
                            PathBuf::from(&analysis.path),
                            SavedLayout {
                                options: options.clone(),
                                source: self.layout_source.clone(),
                            },
                        );
                    } else {
                        settings.layouts.remove(&PathBuf::from(&analysis.path));
                    }
                    self.remembered_firmware = Some(RememberedFirmware {
                        folder: build.root.clone(),
                        path: PathBuf::from(&analysis.path),
                        layout: self.layout_override.clone(),
                        source: self.layout_source.clone(),
                    });
                }
                self.persist_preferences();
            }
            Some(Err(mpsc::TryRecvError::Disconnected)) => {
                self.receiver = None;
                self.error = Some("The analysis worker stopped unexpectedly.".into());
            }
            _ => {}
        }
    }
    fn replace_stack(&mut self, stack: Option<LoadedStack>) {
        self.stack = stack.map(|(report, selection)| {
            if let (Some(build), Some(analysis), Some(selection)) =
                (&self.build, &self.analysis, selection)
            {
                self.build_settings
                    .entry(build.root.clone())
                    .or_default()
                    .stack_reports
                    .insert(PathBuf::from(&analysis.path), selection);
            }
            report
        });
    }
}
impl eframe::App for Explorer {
    fn on_exit(&mut self, _: Option<&eframe::glow::Context>) {
        if let Err(error) = self.save_preferences() {
            eprintln!("Could not save workspace preferences: {error}");
        }
    }
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        self.poll();
        if self.receiver.is_some() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        self.poll_updates(ctx);
        self.show(ctx);
        self.show_updates(ctx);
    }
}
fn main() -> eframe::Result {
    let startup = match startup::parse(std::env::args_os().skip(1)) {
        Ok(Some(startup)) => startup,
        Ok(None) => return Ok(()),
        Err(error) => {
            eprintln!("{error}");
            rfd::MessageDialog::new()
                .set_title("Invalid arguments")
                .set_description(error)
                .set_level(rfd::MessageLevel::Error)
                .show();
            std::process::exit(2);
        }
    };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_icon(
                eframe::icon_data::from_png_bytes(include_bytes!("../packaging/icons/snout.png"))
                    .expect("bundled Snout icon"),
            )
            .with_inner_size([1280.0, 820.0])
            .with_min_inner_size([900.0, 600.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Rusty's Snout - Firmware Explorer",
        options,
        Box::new(move |cc| {
            shell::configure_style(&cc.egui_ctx);
            #[cfg(target_os = "linux")]
            window_theme::apply_startup_theme(cc.egui_ctx.clone());
            let mut app = Explorer {
                preferences_file: preferences::preferences_path(),
                ..Default::default()
            };
            app.restore_preferences(startup.folder.is_none());
            app.open_startup(&startup);
            if app.updates.check_on_startup && !startup.no_update_check {
                app.start_update_check(&cc.egui_ctx, false);
            }
            Ok(Box::new(app))
        }),
    )?;
    if let Some(path) = RESTART_PATH.lock().expect("restart path lock").take() {
        let mut command = std::process::Command::new(&path);
        if let Some(directory) = path.parent() {
            command.current_dir(directory);
        }
        for key in ["APPIMAGE", "APPDIR", "OWD", "ARGV0"] {
            command.env_remove(key);
        }
        if let Err(error) = command.spawn() {
            rfd::MessageDialog::new()
                .set_title("Update installed")
                .set_description(format!(
                    "Snout was updated, but could not restart: {error}. Please open it again."
                ))
                .set_level(rfd::MessageLevel::Error)
                .show();
        }
    }
    Ok(())
}
