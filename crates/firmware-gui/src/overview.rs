use super::{egui, Explorer, View};
use firmware_analysis_core::{format_bytes as bytes, Analysis, Section, Usage};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Metric {
    Flash,
    Ram,
    All,
}
impl Metric {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Flash => "Flash",
            Self::Ram => "RAM",
            Self::All => "All sections",
        }
    }
    pub(super) fn value(self, usage: Usage) -> u64 {
        match self {
            Self::Flash => usage.flash,
            Self::Ram => usage.ram,
            Self::All => usage.flash.max(usage.ram),
        }
    }
    pub(super) fn section_size(self, s: &Section) -> u64 {
        if self == Self::All {
            s.size
        } else {
            self.value(s.usage)
        }
    }
}
impl Explorer {
    pub(super) fn ensure_region_cache(&mut self, a: &Analysis) {
        let key = a as *const Analysis as usize;
        if self.region_cache_key != key {
            self.region_cache = a
                .options
                .regions
                .iter()
                .map(|r| firmware_analysis_core::regions::region_usage(a, r))
                .collect();
            for (slot, metric) in [Metric::Flash, Metric::Ram].into_iter().enumerate() {
                let mut files: Vec<_> = (0..a.files.len())
                    .filter(|&i| {
                        a.files[i].path != "[unattributed]" && metric.value(a.files[i].usage) > 0
                    })
                    .collect();
                files.sort_by(|&i, &j| {
                    metric
                        .value(a.files[j].usage)
                        .cmp(&metric.value(a.files[i].usage))
                        .then_with(|| a.files[i].path.cmp(&a.files[j].path))
                });
                files.truncate(5);
                self.top_files[slot] = files;
                let mut symbols: Vec<_> = (0..a.symbols.len())
                    .filter(|&i| metric.value(a.symbols[i].usage) > 0)
                    .collect();
                symbols.sort_by(|&i, &j| {
                    metric
                        .value(a.symbols[j].usage)
                        .cmp(&metric.value(a.symbols[i].usage))
                        .then_with(|| a.symbols[i].name.cmp(&a.symbols[j].name))
                        .then_with(|| a.symbols[i].address.cmp(&a.symbols[j].address))
                });
                symbols.truncate(5);
                self.top_symbols[slot] = symbols;
            }
            self.region_cache_key = key;
        }
    }
    pub(super) fn visible_notes(&self) -> Vec<&str> {
        let mut notes: Vec<_> = self
            .analysis
            .iter()
            .flat_map(|a| a.warnings.iter().map(String::as_str))
            .collect();
        if matches!(self.view, View::Overview | View::Stack) {
            notes.extend(
                self.stack
                    .iter()
                    .flat_map(|s| s.warnings.iter().map(String::as_str)),
            );
        }
        if matches!(self.view, View::Overview | View::Compare) {
            notes.extend(
                self.comparison
                    .iter()
                    .flat_map(|c| c.warnings.iter().map(String::as_str)),
            );
        }
        notes
    }
    pub(super) fn pick_layout(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Memory layout", &["json", "map"])
            .pick_file()
        {
            if path.extension().is_some_and(|e| e == "map") {
                self.apply_map(path);
            } else {
                self.configure(Some(path));
            }
        }
    }
    pub(super) fn overview(&mut self, ui: &mut egui::Ui, a: &Analysis) {
        self.ensure_region_cache(a);
        egui::ScrollArea::vertical()
            .id_salt("overview_dashboard")
            .show(ui, |ui| {
                self.firmware_heading(ui, a);
                self.capacity_summary(ui, a);
                let notes = self.visible_notes();
                if let Some(first) = notes.first() {
                    let first = first.to_string();
                    let count = notes.len();
                    ui.group(|ui| {
                        ui.colored_label(egui::Color32::from_rgb(235, 197, 118), first);
                        if ui.link(format!("Review all {count} analysis notes")).clicked() {
                            self.show_notes = true;
                        }
                    });
                }
                ui.separator();
                ui.horizontal_wrapped(|ui| {
                    ui.strong("Memory breakdown");
                    for metric in [Metric::Flash, Metric::Ram, Metric::All] {
                        if ui.selectable_value(&mut self.overview_metric, metric, metric.label()).changed() {
                            self.overview_section = None;
                            self.overview_unit = None;
                        }
                    }
                });
                ui.small("Initialized data and RAM code can occupy both Flash and RAM. All sections counts each section once.");
                if ui.available_width() >= 760.0 {
                    ui.columns(2, |columns| {
                        self.overview_pie(&mut columns[0], a);
                        self.contributors(&mut columns[1], a);
                    });
                } else {
                    self.overview_pie(ui, a);
                    ui.separator();
                    self.contributors(ui, a);
                }
                ui.separator();
                self.analysis_status(ui, a);
            });
    }

    fn firmware_heading(&mut self, ui: &mut egui::Ui, a: &Analysis) {
        let path = std::path::Path::new(&a.path);
        ui.heading(path.file_name().unwrap_or_default().to_string_lossy());
        let relative = self
            .build
            .as_ref()
            .and_then(|b| path.strip_prefix(&b.root).ok())
            .unwrap_or(path);
        ui.label(format!(
            "{} | {} / {}-bit",
            relative.display(),
            a.metadata.architecture,
            a.metadata.bitness
        ));
        ui.collapsing("Firmware details", |ui| {
            ui.label(format!("{} endian | Entry {:#x} | ELF {}",
                a.metadata.endianness, a.metadata.entry_point, bytes(a.metadata.file_size)));
            ui.label(&a.path);
            ui.small("ELF file size includes debug information and headers; it is not programmed image size.");
        });
        ui.separator();
        ui.horizontal_wrapped(|ui| {
            super::views::metric(
                ui,
                "Flash payload",
                a.totals.flash,
                super::views::FLASH_HELP,
            );
            ui.separator();
            super::views::metric(ui, "Static RAM", a.totals.ram, super::views::RAM_HELP);
            if let Some(c) = &self.comparison {
                ui.label(format!(
                    "Change: Flash {:+} B / RAM {:+} B",
                    c.flash_delta, c.ram_delta
                ));
            }
        });
    }

    fn capacity_summary(&mut self, ui: &mut egui::Ui, a: &Analysis) {
        if a.options.regions.is_empty() {
            ui.label("Inferred memory types | Capacity unknown");
        }
        for (i, region) in a.options.regions.iter().enumerate() {
            let usage = &self.region_cache[i];
            let fraction = usage.used as f64 / region.size.max(1) as f64;
            // Keep the text separate from the bar so narrow windows can wrap it.
            ui.label(format!(
                "{}: {} used / {} total ({:.1}%) | {} remaining",
                region.name,
                bytes(usage.used),
                bytes(region.size),
                fraction * 100.0,
                bytes(usage.free)
            ));
            ui.add(egui::ProgressBar::new(fraction as f32).desired_height(8.0));
        }
        if !a.options.regions.is_empty() {
            ui.small("Remaining space excludes static ELF occupancy only; runtime heap and stack may use it.");
        }
        ui.horizontal_wrapped(|ui| {
            let source = if self.layout_source.is_empty() {
                "ELF inference"
            } else {
                &self.layout_source
            };
            ui.label(format!("Layout: {source}"));
            if ui
                .add_enabled(
                    self.receiver.is_none(),
                    egui::Button::new("Configure memory regions..."),
                )
                .clicked()
            {
                self.pick_layout();
            }
        });
    }

    fn analysis_status(&mut self, ui: &mut egui::Ui, a: &Analysis) {
        ui.strong("Analysis status");
        let dwarf = if a.metadata.has_dwarf {
            "present"
        } else {
            "absent"
        };
        let files = a
            .files
            .iter()
            .filter(|f| f.path != "[unattributed]")
            .count();
        ui.label(format!(
            "DWARF: {dwarf} | {} symbols | {files} attributed files/units",
            a.symbols.len()
        ));
        for (label, total, unknown) in [
            ("Flash", a.totals.flash, a.unattributed.flash),
            ("RAM", a.totals.ram, a.unattributed.ram),
        ] {
            let coverage = if total == 0 {
                "n/a".into()
            } else {
                format!(
                    "{:.1}%",
                    total.saturating_sub(unknown) as f64 * 100.0 / total as f64
                )
            };
            ui.label(format!(
                "{label}: {coverage} file attribution | {} unattributed",
                bytes(unknown)
            ));
        }
        ui.small("Unattributed bytes include unknown owners, padding and reservations; they do not necessarily indicate a parsing failure.");
        if ui.link("Inspect unattributed symbols").clicked() {
            self.change_view(View::Symbols);
            self.selected_file = Some("[unattributed]".into());
            self.kind_filter = "All".into();
        }
        if ui.link("Inspect sections and reservations").clicked() {
            self.change_view(View::Sections);
        }
        let entries = self.stack.as_ref().map_or(0, |s| s.entries.len());
        ui.label(format!(
            "Stack reports: {entries} local frame entries | Call-chain total unknown"
        ));
        if entries > 0 && ui.link("Inspect stack reports").clicked() {
            self.change_view(View::Stack);
        }
    }
    fn contributors(&mut self, ui: &mut egui::Ui, a: &Analysis) {
        ui.horizontal_wrapped(|ui| {
            ui.strong("Largest contributors");
            ui.selectable_value(&mut self.contributor_ram, false, "Flash");
            ui.selectable_value(&mut self.contributor_ram, true, "RAM");
        });
        let metric = if self.contributor_ram {
            Metric::Ram
        } else {
            Metric::Flash
        };
        ui.label("Source files");
        let files = self.top_files[usize::from(self.contributor_ram)].clone();
        if files.is_empty() {
            ui.label("No attributed files in this memory space.");
        }
        for index in files {
            let file = &a.files[index];
            if ui
                .link(format!(
                    "{} | {}",
                    bytes(metric.value(file.usage)),
                    file.path
                ))
                .clicked()
            {
                self.change_view(View::Symbols);
                self.selected_file = Some(file.path.clone());
                self.kind_filter = "All".into();
            }
        }
        ui.add_space(10.0);
        ui.label("Functions and data symbols");
        let symbols = self.top_symbols[usize::from(self.contributor_ram)].clone();
        if symbols.is_empty() {
            ui.label("No sized symbols in this memory space.");
        }
        for index in symbols {
            let symbol = &a.symbols[index];
            ui.collapsing(
                format!(
                    "{} | {} ({:#x})",
                    bytes(metric.value(symbol.usage)),
                    symbol.demangled_name,
                    symbol.normalized_address
                ),
                |ui| {
                    ui.label(format!(
                        "{} | {} | ELF size {}",
                        symbol.kind,
                        symbol.section,
                        bytes(symbol.size)
                    ));
                    ui.label(format!(
                        "Source: {} | {}",
                        symbol.source_file.as_deref().unwrap_or("Unknown"),
                        symbol.attribution
                    ));
                    if ui.link("Open in Symbols").clicked() {
                        self.change_view(View::Symbols);
                        self.selected_file = None;
                        self.kind_filter = "All".into();
                        self.search = symbol.demangled_name.clone();
                    }
                },
            );
        }
        ui.small("Ranked by unique attributed bytes; aliases are not counted twice.");
    }
}
