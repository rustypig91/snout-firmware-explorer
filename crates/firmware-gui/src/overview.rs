use super::display::{display_path, short_path};
use super::snapshots::{stack_key, symbol_key};
use super::{egui, Explorer, View};
use firmware_analysis_core::{Analysis, Section, Usage};
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
        let key = a as *const Analysis as usize | usize::from(self.diffs_active());
        if self.region_cache_key != key {
            self.region_cache = a
                .options
                .regions
                .iter()
                .map(|r| self.display_region_usage(a, r))
                .collect();
            for (slot, metric) in [Metric::Flash, Metric::Ram].into_iter().enumerate() {
                let mut files: Vec<_> = (0..a.files.len())
                    .filter(|&i| {
                        self.diff_visible_in_view("file", &a.files[i].path, View::Overview)
                    })
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
                    .filter(|&i| {
                        self.diff_visible_in_view(
                            "symbol",
                            &symbol_key(&a.symbols[i]),
                            View::Overview,
                        )
                    })
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
        if self.view == View::Overview {
            notes.extend(
                self.comparison
                    .iter()
                    .flat_map(|c| c.warnings.iter().map(String::as_str)),
            );
        }
        notes
    }
    pub(super) fn overview(&mut self, ui: &mut egui::Ui, a: &Analysis) {
        self.ensure_region_cache(a);
        let mut scroll = egui::ScrollArea::vertical().id_salt("overview_dashboard");
        if std::mem::take(&mut self.overview_scroll_top) {
            scroll = scroll.vertical_scroll_offset(0.0);
        }
        scroll.show(ui, |ui| {
                self.firmware_heading(ui, a);
                self.capacity_summary(ui, a);
                self.ram_composition(ui, a);
                let display = self.baseline_display_analysis();
                if let Some(tls) = &display.as_deref().unwrap_or(a).tls {
                    ui.collapsing("Thread-local storage detected — click to view details", |ui| {
                        ui.label(format!("Template per thread: {} — {} initialized, {} zero-initialized; alignment {}", self.snapshot_bytes("tls", "", "template_size", tls.template_size), self.snapshot_bytes("tls", "", "initialized_size", tls.initialized_size), self.snapshot_bytes("tls", "", "zero_initialized_size", tls.zero_initialized_size), self.snapshot_bytes("tls", "", "alignment", tls.alignment)));
                        ui.label("Total TLS RAM is unknown. Static RAM excludes TLS templates; allocation may be inside existing stack reservations.");
                        ui.collapsing(format!("{} TLS variables", self.snapshot_count("counts", "", "tls", a.tls.as_ref().map_or(0, |t| t.symbols.len()) as u64)), |ui| {
                            for symbol in &tls.symbols {
                                if !self.diff_visible("tls_symbol", &symbol.name) { continue; }
                                ui.monospace(format!("{}  {}  {} [{}]", self.snapshot_address("tls_symbol", &symbol.name, "offset", symbol.offset), self.snapshot_bytes("tls_symbol", &symbol.name, "size", symbol.size), symbol.name, symbol.section));
                            }
                        });
                    });
                }
                self.growth_summary(ui);
                ui.separator();
                ui.horizontal_wrapped(|ui| {
                    ui.strong("Memory breakdown").on_hover_text("Initialized data and RAM code can occupy both Flash and RAM. All sections counts each section once.");
                    for metric in [Metric::Flash, Metric::Ram, Metric::All] {
                        if ui.selectable_value(&mut self.overview_metric, metric, metric.label()).changed() {
                            self.overview_section = None;
                            self.overview_unit = None;
                        }
                    }
                });
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
        if let Some(name) = self.snapshot_label() {
            ui.label(format!(
                "Comparing to snapshot: {name} · current minus baseline"
            ));
        }
        ui.collapsing("Firmware details", |ui| {
            ui.label(format!("{} / {}-bit", a.metadata.architecture, a.metadata.bitness));
            ui.label(format!("{} endian | Entry {} | ELF {}",
                a.metadata.endianness, self.snapshot_address("metadata", "", "entry_point", a.metadata.entry_point), self.snapshot_bytes("metadata", "", "file_size", a.metadata.file_size)));
            ui.label(display_path(&a.path));
            ui.small("ELF file size includes debug information and headers; it is not programmed image size.");
        });
        ui.separator();
        if let Some(c) = &self.comparison {
            ui.label(format!(
                "Change: Flash {:+} B / RAM {:+} B",
                c.flash_delta, c.ram_delta
            ));
        }
    }

    fn capacity_summary(&mut self, ui: &mut egui::Ui, a: &Analysis) {
        for (kind, label, used, help) in [
            (
                firmware_analysis_core::MemoryKind::Flash,
                "Flash",
                a.totals.flash,
                super::views::FLASH_HELP,
            ),
            (
                firmware_analysis_core::MemoryKind::Ram,
                "RAM",
                a.totals.ram,
                super::views::RAM_HELP,
            ),
        ] {
            if !a.options.regions.iter().any(|r| r.kind == kind) {
                ui.label(format!(
                    "{label}: {} used | Capacity unknown",
                    self.snapshot_bytes(
                        "totals",
                        "",
                        if label == "Flash" { "flash" } else { "ram" },
                        used
                    )
                ))
                .on_hover_text(help);
            }
        }
        for (i, region) in a.options.regions.iter().enumerate() {
            let usage = &self.region_cache[i];

            // Keep the text separate from the bar so narrow windows can wrap it.
            ui.label(format!(
                "{}: {} used / {} total ({}) | {} remaining",
                region.name,
                self.snapshot_bytes("region", &region.name, "used", usage.used),
                self.snapshot_bytes("region", &region.name, "size", region.size),
                self.snapshot_region_percentage(&region.name,usage.used,region.size),
                self.snapshot_bytes("region", &region.name, "free", usage.free)
            )).on_hover_text("Remaining space excludes static ELF occupancy only; runtime heap and stack may use it.");
            ui.add(
                egui::ProgressBar::new((usage.used as f64 / region.size.max(1) as f64) as f32)
                    .desired_height(8.0),
            );
        }
        if let Some(old) = self.snapshot_analysis() {
            for region in old
                .options
                .regions
                .iter()
                .filter(|r| self.snapshot_removed("region", &r.name))
            {
                ui.label(format!(
                    "{}: {} used / {} total (removed)",
                    region.name,
                    self.snapshot_bytes("region", &region.name, "used", 0),
                    self.snapshot_bytes("region", &region.name, "size", 0)
                ));
            }
        }
    }

    fn ram_composition(&mut self, ui: &mut egui::Ui, a: &Analysis) {
        ui.horizontal_wrapped(|ui| {
            ui.strong("RAM composition").on_hover_text("BSS and reservation labels follow section naming conventions. No-load storage alone does not prove startup initialization. Runtime heap and stack demand may exceed reservations.");
            let mut roles = std::collections::BTreeMap::<&str, u64>::new();
            for section in a.sections.iter().filter(|s| s.usage.ram > 0) {
                *roles.entry(super::insights::ram_role(section)).or_default() += section.usage.ram;
            }
            if let Some(old) = self.snapshot_analysis() {
                for section in old.sections.iter().filter(|s| s.usage.ram > 0) {
                    roles.entry(super::insights::ram_role(section)).or_default();
                }
            }
            for (role, size) in roles {
                ui.label(format!("{role}: {}", self.snapshot_bytes("ram_role", role, "size", size)));
            }
        });
    }

    fn growth_summary(&mut self, ui: &mut egui::Ui) {
        let display = self.baseline_display_analysis();
        let paths = display
            .as_deref()
            .or(self.analysis.as_deref())
            .map(|a| self.source_paths(a));
        let Some(c) = &self.comparison else {
            return;
        };
        let mut open = None;
        egui::CollapsingHeader::new("Largest build increases")
            .default_open(true)
            .show(ui, |ui| {
                let groups = [
                    (2, "Sections", &c.sections),
                    (0, "Files", &c.files),
                    (1, "Symbols", &c.symbols),
                ];
                let mut render =
                    |ui: &mut egui::Ui,
                     group,
                     title,
                     changes: &Vec<firmware_analysis_core::compare::Change>| {
                        ui.push_id(title, |ui| {
                            ui.strong(title);
                            let mut any = false;
                            for ram in [false, true] {
                                let delta = |change: &firmware_analysis_core::compare::Change| {
                                    if ram {
                                        change.ram_delta
                                    } else {
                                        change.flash_delta
                                    }
                                };
                                let mut increases: Vec<_> =
                                    changes.iter().filter(|change| delta(change) > 0).collect();
                                increases.sort_by_key(|change| std::cmp::Reverse(delta(change)));
                                for change in increases.into_iter().take(3) {
                                    any = true;
                                    let label = if group == 0 {
                                        paths.as_ref().map_or_else(
                                            || {
                                                short_path(
                                                    &change.identity,
                                                    changes.iter().map(|c| c.identity.as_str()),
                                                )
                                            },
                                            |p| p.short(&change.identity),
                                        )
                                    } else if group == 1 {
                                        change
                                            .identity
                                            .rsplit(" | ")
                                            .next()
                                            .unwrap_or(&change.identity)
                                            .to_owned()
                                    } else {
                                        change.identity.clone()
                                    };
                                    if ui
                                        .link(format!(
                                            "{} +{} B | {label}",
                                            if ram { "RAM" } else { "Flash" },
                                            delta(change)
                                        ))
                                        .on_hover_text(if group == 0 {
                                            paths.as_ref().map_or_else(
                                                || change.identity.clone(),
                                                |p| p.full(&change.identity),
                                            )
                                        } else {
                                            change.identity.clone()
                                        })
                                        .clicked()
                                    {
                                        open = Some((group, change.identity.clone()));
                                    }
                                }
                            }
                            if !any {
                                ui.weak("No increases.");
                            }
                        });
                    };
                if ui.available_width() >= 760.0 {
                    ui.columns(3, |columns| {
                        for (column, (group, title, changes)) in columns.iter_mut().zip(groups) {
                            render(column, group, title, changes);
                        }
                    });
                } else {
                    for (group, title, changes) in groups {
                        ui.collapsing(title, |ui| render(ui, group, title, changes));
                    }
                }
            });
        if let Some((group, identity)) = open {
            self.change_view(match group {
                1 => View::Symbols,
                2 => View::Sections,
                _ => View::Files,
            });
            self.selected_file = None;
            self.kind_filter = "All".into();
            self.search = if group == 1 {
                identity
                    .rsplit(" | ")
                    .next()
                    .unwrap_or(&identity)
                    .to_owned()
            } else {
                identity
            };
        }
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
            "DWARF: {dwarf} | {} symbols | {} attributed files/units",
            self.snapshot_count("counts", "", "symbols", a.symbols.len() as u64),
            self.snapshot_count("counts", "", "files", files as u64)
        ));
        for (label, total, unknown) in [
            ("Flash", a.totals.flash, a.unattributed.flash),
            ("RAM", a.totals.ram, a.unattributed.ram),
        ] {
            let coverage = if total == 0 {
                "n/a".into()
            } else {
                let field = if label == "RAM" { "ram" } else { "flash" };
                let old = self
                    .snapshot_old("totals", "", field)
                    .zip(self.snapshot_old("unattributed", "", field))
                    .filter(|(total, _)| *total != 0)
                    .map(|(total, unknown)| {
                        total.saturating_sub(unknown) as f64 * 100.0 / total as f64
                    });
                self.snapshot_percentage(
                    total.saturating_sub(unknown) as f64 * 100.0 / total as f64,
                    old,
                )
            };
            ui.label(format!(
                "{label}: {coverage} file attribution | {} unattributed",
                self.snapshot_bytes("unattributed", "", if label == "Flash" { "flash" } else { "ram" }, unknown)
            )).on_hover_text("Unattributed bytes include unknown owners, padding and reservations; they do not necessarily indicate a parsing failure.");
            if unknown > 0 {
                ui.collapsing(format!("Explain {} unattributed {label}", self.snapshot_bytes("unattributed", "", if label == "Flash" { "flash" } else { "ram" }, unknown)), |ui| {
                    let ram = label == "RAM";
                    let mut gaps: Vec<_> = a.sections.iter().map(|s| (s, super::insights::section_unattributed(a, s, ram)))
                        .filter(|(_, (gap, _))| *gap > 0).collect();
                    gaps.sort_by_key(|(_, (gap, _))| std::cmp::Reverse(*gap));
                    for (section, (gap, uncovered)) in gaps {
                        let reason = if uncovered == gap && super::insights::ram_role(section) == "Reservations (by section name)" {
                            "reserved without a source owner"
                        } else if uncovered == gap {
                            "not covered by sized symbols"
                        } else if uncovered == 0 {
                            "symbols without a source owner"
                        } else {
                            "unknown source owners and uncovered bytes"
                        };
                        if ui.link(format!("{} in {} · {reason}", self.snapshot_bytes("section", &section.name, if ram { "unattributed.ram" } else { "unattributed.flash" }, gap), section.name))
                            .on_hover_text(format!("{} uncovered by sized symbols; {} in symbols without a source owner", self.snapshot_bytes("section", &section.name, if ram { "uncovered.ram" } else { "uncovered.flash" }, uncovered), self.snapshot_bytes("section", &section.name, if ram { "unowned.ram" } else { "unowned.flash" }, gap - uncovered)))
                            .clicked() {
                            self.change_view(View::Overview);
                            self.overview_metric = if ram { Metric::Ram } else { Metric::Flash };
                            self.overview_section = Some(section.index);
                            self.overview_unit = None;
                            self.overview_scroll_top = true;
                        }
                    }
                });
            }
        }
        if ui.link("Inspect unattributed symbols").clicked() {
            self.change_view(View::Symbols);
            self.search.clear();
            self.selected_file = Some("[unattributed]".into());
            self.kind_filter = "All".into();
        }
        if ui.link("Inspect sections and reservations").clicked() {
            self.change_view(View::Sections);
        }
        let entries = self.stack.as_ref().map_or(0, |s| s.entries.len());
        ui.label(format!("Stack reports: {} local frames | Call-chain total unknown",self.snapshot_count("counts", "", "stack", entries as u64)))
            .on_hover_text("Compiler .su reports describe individual frames. The app does not construct a call graph, so caller/callee nesting, recursion, indirect calls and interrupt overhead cannot be totaled.");
        if let Some(report) = &self.stack {
            let largest = report
                .entries
                .iter()
                .filter(|e| e.symbol_candidates.len() == 1)
                .max_by_key(|e| e.local_bytes);
            if let Some(entry) = largest {
                if ui
                    .link(format!(
                        "Largest matched frame: {} · {} ({})",
                        self.snapshot_bytes(
                            "stack",
                            &stack_key(entry),
                            "local_bytes",
                            entry.local_bytes
                        ),
                        entry.function,
                        entry.qualifier
                    ))
                    .on_hover_text(format!(
                        "{}\n{}",
                        display_path(&entry.report_file),
                        entry.evidence
                    ))
                    .clicked()
                {
                    let function = entry.function.clone();
                    self.change_view(View::Stack);
                    self.search = function;
                }
            } else if entries > 0 {
                ui.weak("Available frames have no unique match in this ELF. Inspect report paths and matching evidence.");
            }
        }
        if ui
            .link(if entries > 0 {
                "Inspect stack reports and uncertainty"
            } else {
                "Load stack reports (-fstack-usage)"
            })
            .clicked()
        {
            self.change_view(View::Stack);
            self.search.clear();
        }
    }
    fn contributors(&mut self, ui: &mut egui::Ui, a: &Analysis) {
        let display = self.baseline_display_analysis();
        let paths = self.source_paths(display.as_deref().unwrap_or(a));
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
                    self.snapshot_bytes(
                        "file",
                        &file.path,
                        if metric == Metric::Ram {
                            "usage.ram"
                        } else {
                            "usage.flash"
                        },
                        metric.value(file.usage)
                    ),
                    paths.short(&file.path)
                ))
                .on_hover_text(paths.full(&file.path))
                .clicked()
            {
                self.change_view(View::Symbols);
                self.search.clear();
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
                    "{} | {} ({})",
                    self.snapshot_bytes(
                        "symbol",
                        &symbol_key(symbol),
                        if metric == Metric::Ram {
                            "usage.ram"
                        } else {
                            "usage.flash"
                        },
                        metric.value(symbol.usage)
                    ),
                    symbol.demangled_name,
                    self.snapshot_address(
                        "symbol",
                        &symbol_key(symbol),
                        "normalized_address",
                        symbol.normalized_address
                    )
                ),
                |ui| {
                    ui.label(format!(
                        "{} | {} | ELF size {}",
                        symbol.kind,
                        symbol.section,
                        self.snapshot_bytes("symbol", &symbol_key(symbol), "size", symbol.size)
                    ));
                    ui.label(format!(
                        "Source: {} | {}",
                        paths.short(symbol.source_file.as_deref().unwrap_or("Unknown")),
                        symbol.attribution
                    ))
                    .on_hover_text(paths.full(symbol.source_file.as_deref().unwrap_or("Unknown")));
                    if ui.link("Open in Symbols").clicked() {
                        self.change_view(View::Symbols);
                        self.selected_file = None;
                        self.kind_filter = "All".into();
                        self.search = symbol.demangled_name.clone();
                    }
                },
            );
        }
        ui.weak("Unique attributed bytes").on_hover_text("Aliases share storage and are not counted twice. ELF symbol sizes can be larger than their unique contribution.");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contributors_share_baseline_path_labels_and_reuse_the_display_cache() {
        let mut current = firmware_analysis_core::analyze_bytes(
            include_bytes!("../../../fixtures/build/cortex-m.elf"),
            "fixture.elf",
            &Default::default(),
        )
        .unwrap();
        current.symbols.truncate(1);
        current.symbols[0].source_file = Some("/project/app/src/main.c".into());
        current.symbols[0].dwarf_compilation_unit = current.symbols[0].source_file.clone();
        current.symbols[0].compilation_unit = None;
        current.files = vec![firmware_analysis_core::FileUsage {
            path: "/project/app/src/main.c".into(),
            attribution: "DWARF".into(),
            usage: Usage { flash: 4, ram: 0 },
            symbol_count: 1,
        }];
        current.dependencies.nodes.clear();
        let mut old = current.clone();
        old.files[0].path = "/project/lib/src/main.c".into();
        old.symbols[0].source_file = Some(old.files[0].path.clone());
        old.symbols[0].dwarf_compilation_unit = old.symbols[0].source_file.clone();
        let mut app = Explorer {
            baseline_display: Some(super::super::baseline_display::BaselineDisplay::new(
                &current, &old, None, None,
            )),
            ..Default::default()
        };
        app.ensure_region_cache(&current);
        let display = app.baseline_display_analysis().unwrap();
        let paths = app.source_paths(&display);
        assert_eq!(paths.short(&current.files[0].path), "app/src/main.c");
        let ctx = egui::Context::default();
        for _ in 0..2 {
            let output = ctx.run(Default::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| app.contributors(ui, &current));
            });
            assert!(output.shapes.iter().any(|shape| {
                matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text().contains("app/src/main.c"))
            }));
            assert!(std::rc::Rc::ptr_eq(&paths, &app.source_paths(&display)));
        }
    }
}
