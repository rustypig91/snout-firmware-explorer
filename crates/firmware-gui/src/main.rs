#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod pie;
mod shell;
#[cfg(test)]
mod tests;
mod views;
mod workspace;
use eframe::egui;
use firmware_analysis_core::{
    analyze_path,
    compare::{compare, Comparison},
    stack::{analyze_stack, StackReport},
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
    Stack,
    Compare,
}
impl View {
    const ALL: [Self; 7] = [
        Self::Overview,
        Self::Files,
        Self::Symbols,
        Self::Sections,
        Self::MemoryMap,
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
            Self::Stack => "Stack",
            Self::Compare => "Compare",
        }
    }
}
enum Loaded {
    Firmware(Analysis, Option<StackReport>, Option<AnalysisOptions>),
    Build(firmware_analysis_core::build::BuildFolder),
    Text(PathBuf, String),
    Baseline(Comparison),
    Stack(StackReport),
    Config(AnalysisOptions, Option<Analysis>),
}
type JobResult = Result<Loaded, String>;
struct Explorer {
    analysis: Option<Arc<Analysis>>,
    build: Option<Arc<firmware_analysis_core::build::BuildFolder>>,
    artifact_search: String,
    preview: Option<(PathBuf, String)>,
    layout_override: Option<AnalysisOptions>,
    comparison: Option<Comparison>,
    stack: Option<StackReport>,
    options: AnalysisOptions,
    receiver: Option<mpsc::Receiver<JobResult>>,
    view: View,
    search: String,
    selected_file: Option<String>,
    selected_region: Option<usize>,
    overview_section: Option<usize>,
    overview_unit: Option<pie::UnitKey>,
    sort_column: usize,
    descending: bool,
    error: Option<String>,
    tree: bool,
    details: Option<(String, String)>,
    show_notes: bool,
    visible_rows: usize,
    kind_filter: String,
    comparison_symbols: bool,
}
impl Default for Explorer {
    fn default() -> Self {
        Self {
            analysis: None,
            build: None,
            artifact_search: String::new(),
            preview: None,
            layout_override: None,
            comparison: None,
            stack: None,
            options: AnalysisOptions::default(),
            receiver: None,
            view: View::Overview,
            search: String::new(),
            selected_file: None,
            selected_region: None,
            overview_section: None,
            overview_unit: None,
            sort_column: 1,
            descending: true,
            error: None,
            tree: false,
            details: None,
            show_notes: false,
            visible_rows: 0,
            kind_filter: "All".into(),
            comparison_symbols: false,
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
        let layout = if self
            .analysis
            .as_ref()
            .is_some_and(|a| std::path::Path::new(&a.path) != path)
        {
            None
        } else {
            self.layout_override.clone()
        };
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
        let Some(build) = self.build.clone() else {
            return;
        };
        self.job(move || {
            let mut analysis = firmware_analysis_core::build::analyze_build_firmware(&build, &path, layout.as_ref()).map_err(|e| e.to_string())?;
            let reports = build.artifacts.iter().filter(|a| a.kind == firmware_analysis_core::build::ArtifactKind::StackUsage).map(|a| a.path.clone()).collect();
            let stack = match firmware_analysis_core::stack::analyze_stack_files(&analysis, reports) {
                Ok(mut report) => {
                    report.warnings.push(format!("Reports discovered under {}. This folder may contain multiple targets or configurations; exact symbol-name matches do not prove build ownership.", build.root.display()));
                    Some(report)
                }
                Err(e) => { analysis.warnings.push(format!("Stack reports could not be loaded: {e}")); None }
            };
            Ok(Loaded::Firmware(analysis, stack, layout))
        });
    }
    fn configure(&mut self, path: Option<PathBuf>) {
        let current_path = self.analysis.as_ref().map(|a| a.path.clone());
        self.job(move || {
            let options = match path {
                Some(path) => {
                    serde_json::from_slice(&std::fs::read(path).map_err(|e| e.to_string())?)
                        .map_err(|e| e.to_string())?
                }
                None => AnalysisOptions::default(),
            };
            firmware_analysis_core::validate_options(&options).map_err(|e| e.to_string())?;
            let analysis = current_path
                .map(|p| analyze_path(p, &options))
                .transpose()
                .map_err(|e| e.to_string())?;
            Ok(Loaded::Config(options, analysis))
        });
    }
    fn pick_baseline(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("ELF firmware", &["elf", "axf", "out"])
            .add_filter("All files", &["*"])
            .pick_file()
        {
            if let Some(current) = self.analysis.clone() {
                let options = self.options.clone();
                self.job(move || {
                    analyze_path(path, &options)
                        .map(|old| Loaded::Baseline(compare(&old, &current)))
                        .map_err(|e| e.to_string())
                });
            }
        }
    }
    fn pick_stack(&mut self, directory: bool) {
        let path = if directory {
            rfd::FileDialog::new().pick_folder()
        } else {
            rfd::FileDialog::new()
                .add_filter("Stack usage", &["su"])
                .pick_file()
        };
        if let Some(path) = path {
            self.scan_stack(path);
        }
    }
    fn scan_stack(&mut self, path: PathBuf) {
        if let Some(analysis) = self.analysis.clone() {
            self.job(move || {
                analyze_stack(&analysis, path)
                    .map(Loaded::Stack)
                    .map_err(|e| e.to_string())
            });
        }
    }
    fn poll(&mut self) {
        match self.receiver.as_ref().map(|r| r.try_recv()) {
            Some(Ok(result)) => {
                self.receiver = None;
                match result {
                    Ok(Loaded::Build(build)) => {
                        self.build = Some(Arc::new(build));
                        self.analysis = None;
                        self.preview = None;
                        self.options = AnalysisOptions::default();
                        self.layout_override = None;
                        self.comparison = None;
                        self.stack = None;
                        self.details = None;
                        self.selected_region = None;
                        self.overview_section = None;
                        self.overview_unit = None;
                        self.selected_file = None;
                        self.artifact_search.clear();
                        self.search.clear();
                        self.visible_rows = 0;
                    }
                    Ok(Loaded::Text(path, text)) => {
                        self.preview = Some((path, text));
                    }
                    Ok(Loaded::Firmware(a, stack, layout)) => {
                        self.layout_override = layout;
                        self.options = a.options.clone();
                        self.preview = None;
                        self.analysis = Some(Arc::new(a));
                        self.comparison = None;
                        self.stack = stack;
                        self.selected_file = None;
                        self.selected_region = None;
                        self.overview_section = None;
                        self.overview_unit = None;
                        self.search.clear();
                        self.details = None;
                        self.visible_rows = 0;
                    }
                    Ok(Loaded::Baseline(c)) => {
                        self.comparison = Some(c);
                        self.change_view(View::Compare);
                    }
                    Ok(Loaded::Stack(s)) => {
                        self.stack = Some(s);
                        self.change_view(View::Stack);
                    }
                    Ok(Loaded::Config(options, analysis)) => {
                        self.details = None;
                        self.selected_region = None;
                        self.overview_section = None;
                        self.overview_unit = None;
                        self.layout_override = Some(options.clone());
                        self.options = options;
                        self.analysis = analysis.map(Arc::new);
                        self.preview = None;
                        self.comparison = None;
                    }
                    Err(error) => self.error = Some(error),
                }
            }
            Some(Err(mpsc::TryRecvError::Disconnected)) => {
                self.receiver = None;
                self.error = Some("The analysis worker stopped unexpectedly.".into());
            }
            _ => {}
        }
    }
}
impl eframe::App for Explorer {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        self.poll();
        if self.receiver.is_some() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        self.show(ctx);
    }
}
fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 820.0])
            .with_min_inner_size([900.0, 600.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Rusty's Snout - Firmware Explorer",
        options,
        Box::new(|cc| {
            shell::configure_style(&cc.egui_ctx);
            let mut app = Explorer::default();
            if let Some(path) = std::env::args_os().nth(1) {
                app.scan_build(path.into());
            }
            Ok(Box::new(app))
        }),
    )
}
