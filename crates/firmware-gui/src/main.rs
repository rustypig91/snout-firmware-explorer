#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#[cfg(test)]
mod tests;
mod views;
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
    Firmware(Analysis),
    Baseline(Comparison),
    Stack(StackReport),
    Config(AnalysisOptions, Option<Analysis>),
}
type JobResult = Result<Loaded, String>;
struct Explorer {
    analysis: Option<Arc<Analysis>>,
    comparison: Option<Comparison>,
    stack: Option<StackReport>,
    options: AnalysisOptions,
    receiver: Option<mpsc::Receiver<JobResult>>,
    view: View,
    search: String,
    selected_file: Option<String>,
    sort_column: usize,
    descending: bool,
    error: Option<String>,
    tree: bool,
}
impl Default for Explorer {
    fn default() -> Self {
        Self {
            analysis: None,
            comparison: None,
            stack: None,
            options: AnalysisOptions::default(),
            receiver: None,
            view: View::Overview,
            search: String::new(),
            selected_file: None,
            sort_column: 1,
            descending: true,
            error: None,
            tree: false,
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
        let options = self.options.clone();
        self.job(move || {
            analyze_path(path, &options)
                .map(Loaded::Firmware)
                .map_err(|e| e.to_string())
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
    fn pick_elf(&mut self, baseline: bool) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("ELF firmware", &["elf", "axf", "out"])
            .add_filter("All files", &["*"])
            .pick_file()
        {
            if baseline {
                if let Some(current) = self.analysis.clone() {
                    let options = self.options.clone();
                    self.job(move || {
                        analyze_path(path, &options)
                            .map(|old| Loaded::Baseline(compare(&old, &current)))
                            .map_err(|e| e.to_string())
                    });
                }
            } else {
                self.open(path);
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
        if let (Some(path), Some(analysis)) = (path, self.analysis.clone()) {
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
                    Ok(Loaded::Firmware(a)) => {
                        self.analysis = Some(Arc::new(a));
                        self.comparison = None;
                        self.stack = None;
                        self.selected_file = None;
                        self.search.clear();
                    }
                    Ok(Loaded::Baseline(c)) => {
                        self.comparison = Some(c);
                        self.view = View::Compare;
                    }
                    Ok(Loaded::Stack(s)) => {
                        self.stack = Some(s);
                        self.view = View::Stack;
                    }
                    Ok(Loaded::Config(options, analysis)) => {
                        self.options = options;
                        self.analysis = analysis.map(Arc::new);
                        self.comparison = None;
                        self.stack = None;
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
        if let Some(path) = ctx.input(|i| i.raw.dropped_files.iter().find_map(|f| f.path.clone())) {
            if self.receiver.is_none() {
                self.open(path);
            }
        }
        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("RUSTY'S SNOUT")
                        .strong()
                        .color(views::ACCENT),
                );
                ui.label("/ Firmware Explorer");
                ui.separator();
                ui.add_enabled_ui(self.receiver.is_none(), |ui| {
                    if ui
                        .button("Open firmware…")
                        .on_hover_text(
                            "Open a linked ELF, AXF or OUT file, or drop it onto this window.",
                        )
                        .clicked()
                    {
                        self.pick_elf(false);
                    }
                    if ui
                        .add_enabled(self.analysis.is_some(), egui::Button::new("Compare build…"))
                        .on_hover_text("Select an older build. Deltas show current minus older.")
                        .clicked()
                    {
                        self.pick_elf(true);
                    }
                    ui.menu_button("Memory layout", |ui| {
                        ui.label(format!("{} configured regions", self.options.regions.len()));
                        if ui.button("Load region configuration…").clicked() {
                            if let Some(path) = rfd::FileDialog::new()
                                .add_filter("JSON", &["json"])
                                .pick_file()
                            {
                                self.configure(Some(path));
                            }
                            ui.close_menu();
                        }
                        if ui.button("Reset to ELF inference").clicked() {
                            self.configure(None);
                            ui.close_menu();
                        }
                    });
                });
                if self.receiver.is_some() {
                    ui.spinner();
                    ui.label("Analyzing…");
                }
            });
            ui.add_space(8.0);
        });
        egui::SidePanel::left("navigation")
            .resizable(false)
            .exact_width(165.0)
            .show(ctx, |ui| {
                ui.add_space(18.0);
                ui.weak("EXPLORE");
                ui.add_space(10.0);
                for view in View::ALL {
                    if ui
                        .add_sized(
                            [145.0, 36.0],
                            egui::Button::new(view.label()).selected(self.view == view),
                        )
                        .clicked()
                    {
                        self.view = view;
                        self.search.clear();
                        self.sort_column = 1;
                        self.descending = true;
                    }
                }
                ui.add_space(25.0);
                ui.separator();
                ui.small("ARM Cortex-M first");
                ui.small("All sizes use bytes / KiB")
                    .on_hover_text("1 KiB = 1,024 bytes. Hover values for exact byte counts.");
            });
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            if let Some(a) = &self.analysis {
                ui.horizontal(|ui| {
                    ui.small(&a.path);
                    ui.separator();
                    ui.small(format!(
                        "{} · {}-bit · {} endian",
                        a.metadata.architecture, a.metadata.bitness, a.metadata.endianness
                    ));
                });
            } else {
                ui.small("Ready · Open firmware to begin");
            }
        });
        egui::CentralPanel::default().show(ctx, |ui| {
            if let Some(error) = self.error.clone() { egui::Frame::group(ui.style()).show(ui, |ui| { ui.colored_label(egui::Color32::LIGHT_RED, error); if ui.small_button("Dismiss").clicked() { self.error = None; } }); }
            let Some(analysis) = self.analysis.clone() else {
                ui.add_space(70.0); ui.heading("Understand your firmware's footprint."); ui.add_space(12.0);
                ui.label("See what occupies Flash and RAM, find large contributors, and compare builds."); ui.add_space(20.0);
                if ui.add_enabled(self.receiver.is_none(), egui::Button::new("Open an ELF firmware file…")).clicked() { self.pick_elf(false); }
                ui.add_space(15.0); ui.weak("You can also drop an ELF file here."); ui.add_space(30.0);
                ui.collapsing("Where do I find my ELF file?", |ui| {
                    ui.label("Look in your build output for a .elf, .axf or .out file. It contains the linked code, data and memory layout.");
                    ui.label("A .bin or .hex file lacks much of this information. Debug information (-g) improves file attribution but is not needed for section sizes.");
                }); return;
            };
            ui.add_space(8.0); ui.heading(self.view.label()); ui.add_space(8.0);
            if !matches!(self.view, View::Overview | View::Compare) {
                ui.horizontal(|ui| { ui.label("Search"); ui.add(egui::TextEdit::singleline(&mut self.search).hint_text("Filter names, files or sections…").desired_width(310.0));
                    if ui.small_button("Clear").clicked() { self.search.clear(); self.selected_file = None; }
                    if self.view == View::Files { ui.checkbox(&mut self.tree, "Directory tree"); }
                }); ui.add_space(8.0);
            }
            match self.view {
                View::Overview => self.overview(ui, &analysis), View::Files => self.files(ui, &analysis),
                View::Symbols => self.symbols(ui, &analysis), View::Sections => self.sections(ui, &analysis),
                View::MemoryMap => self.memory_map(ui, &analysis), View::Stack => self.stack_view(ui), View::Compare => self.compare_view(ui),
            }
        });
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
            let mut style = (*cc.egui_ctx.style()).clone();
            style.visuals = egui::Visuals::dark();
            style.visuals.selection.bg_fill = egui::Color32::from_rgb(36, 86, 92);
            style.spacing.item_spacing = egui::vec2(10.0, 8.0);
            cc.egui_ctx.set_style(style);
            let mut app = Explorer::default();
            if let Some(path) = std::env::args_os().nth(1) {
                app.open(path.into());
            }
            Ok(Box::new(app))
        }),
    )
}
