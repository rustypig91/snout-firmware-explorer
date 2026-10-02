use super::{Explorer, View};
use eframe::egui::{self, RichText};
use egui_extras::{Column, TableBuilder};
use firmware_analysis_core::{format_bytes as bytes, Analysis, FileTree};

pub const ACCENT: egui::Color32 = egui::Color32::from_rgb(105, 216, 191);
const FLASH_HELP: &str = "Allocated bytes stored in the load image. Initialized RAM data also needs initial values in Flash. Gaps and programmer-specific overhead are excluded.";
const RAM_HELP: &str = "Static memory required while running. Includes initialized data, zero-filled storage and explicit reservations. Additional heap and stack demand is not automatically known.";
struct Row {
    cells: Vec<String>,
    values: Vec<Option<i128>>,
    tip: String,
    action: Option<String>,
}
impl Row {
    fn new(cells: Vec<String>, numbers: &[(usize, i128)], tip: String) -> Self {
        let mut values = vec![None; cells.len()];
        for &(column, value) in numbers {
            values[column] = Some(value);
        }
        Self {
            cells,
            values,
            tip,
            action: None,
        }
    }
}
impl Explorer {
    fn table(
        &mut self,
        ui: &mut egui::Ui,
        headers: &[(&str, &str)],
        mut rows: Vec<Row>,
    ) -> Option<String> {
        let search = self.search.to_lowercase();
        rows.retain(|r| r.cells.iter().any(|c| c.to_lowercase().contains(&search)));
        let sort = self.sort_column.min(headers.len() - 1);
        rows.sort_by(|a, b| {
            let order = match (a.values[sort], b.values[sort]) {
                (Some(a), Some(b)) => a.cmp(&b),
                _ => a.cells[sort].cmp(&b.cells[sort]),
            };
            if self.descending {
                order.reverse()
            } else {
                order
            }
        });
        ui.weak(format!(
            "{} rows · Click a column heading to sort",
            rows.len()
        ));
        let mut clicked = None;
        let mut table = TableBuilder::new(ui)
            .striped(true)
            .resizable(true)
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center));
        for (index, _) in headers.iter().enumerate() {
            table = table.column(if index == 0 {
                Column::initial(280.0).at_least(120.0).clip(true)
            } else {
                Column::initial(115.0).at_least(65.0).clip(true)
            });
        }
        table
            .header(30.0, |mut header| {
                for (index, (title, help)) in headers.iter().enumerate() {
                    header.col(|ui| {
                        let indicator = if sort == index {
                            if self.descending {
                                " ▼"
                            } else {
                                " ▲"
                            }
                        } else {
                            ""
                        };
                        if ui
                            .button(format!("{title}{indicator}"))
                            .on_hover_text(*help)
                            .clicked()
                        {
                            if self.sort_column == index {
                                self.descending = !self.descending;
                            } else {
                                self.sort_column = index;
                                self.descending = index != 0;
                            }
                        }
                    });
                }
            })
            .body(|body| {
                body.rows(27.0, rows.len(), |mut row| {
                    let item = &rows[row.index()];
                    for (index, cell) in item.cells.iter().enumerate() {
                        row.col(|ui| {
                            let response = if index == 0 && item.action.is_some() {
                                ui.link(cell)
                            } else {
                                ui.label(cell)
                            };
                            if response.clicked() {
                                clicked = item.action.clone();
                            }
                            response.on_hover_text(&item.tip);
                        });
                    }
                });
            });
        clicked
    }
    pub(super) fn overview(&mut self, ui: &mut egui::Ui, a: &Analysis) {
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.horizontal(|ui| {
                metric(ui, "Flash payload", a.totals.flash, FLASH_HELP); metric(ui, "RAM at runtime · static", a.totals.ram, RAM_HELP);
                metric(ui, "ELF file on disk", a.metadata.file_size, "Includes debug information and ELF metadata. This is not the size programmed into Flash.");
            }); ui.add_space(8.0);
            ui.label("Flash/RAM types are inferred unless a memory layout is configured. Capacity is unknown without a target layout.");
            for region in &self.options.regions {
                let used: u64 = a.sections.iter().filter(|s| {
                    let addr = match region.kind { firmware_analysis_core::MemoryKind::Flash => s.load_address, _ => Some(s.address) };
                    addr.is_some_and(|addr| addr >= region.start && addr.checked_add(s.size).is_some_and(|end| end <= region.start.saturating_add(region.size)))
                }).map(|s| match region.kind { firmware_analysis_core::MemoryKind::Flash => s.usage.flash, _ => s.usage.ram }).sum();
                ui.add(egui::ProgressBar::new((used as f64 / region.size.max(1) as f64) as f32).text(format!("{}: {} / {}", region.name, bytes(used), bytes(region.size))));
            } ui.add_space(15.0);
            egui::Grid::new("metadata").num_columns(4).spacing([25.0, 8.0]).show(ui, |ui| {
                ui.weak("Architecture"); ui.label(format!("{} (machine {})", a.metadata.architecture, a.metadata.machine));
                ui.weak("ELF format"); ui.label(format!("{}-bit / {} endian", a.metadata.bitness, a.metadata.endianness)); ui.end_row();
                ui.weak("Entry point"); ui.monospace(format!("{:#010x}", a.metadata.entry_point)).on_hover_text("The ELF entry address. Cortex-M startup also depends on the vector table.");
                ui.weak("Debug information"); ui.label(if a.metadata.has_dwarf { "Present" } else { "Not present" }); ui.end_row();
            });
            ui.add_space(20.0); ui.heading("Largest contributors");
            ui.horizontal(|ui| {
                if ui.selectable_label(false, "Explore files →").clicked() { self.view = View::Files; }
                if ui.selectable_label(false, "Explore sections →").clicked() { self.view = View::Sections; }
            });
            let mut sections: Vec<_> = a.sections.iter().filter(|s| s.allocated).collect(); sections.sort_by_key(|s| std::cmp::Reverse(s.usage.flash + s.usage.ram));
            for section in sections.into_iter().take(7) { ui.horizontal(|ui| {
                ui.add_sized([155.0, 22.0], egui::Label::new(RichText::new(&section.name).monospace()));
                ui.label(format!("{} Flash · {} RAM", bytes(section.usage.flash), bytes(section.usage.ram))).on_hover_text(&section.evidence);
            }); }
            ui.add_space(15.0);
            ui.label(format!("Not attributed to a file: {} Flash · {} RAM", bytes(a.unattributed.flash), bytes(a.unattributed.ram))).on_hover_text("Includes padding, uncovered bytes and symbols whose owning source or compilation unit is unknown. File totals still reconcile with the overview.");
            if let Some(c) = &self.comparison { ui.add_space(12.0); ui.label(format!("Compared with older build: Flash {:+} B · RAM {:+} B", c.flash_delta, c.ram_delta)); }
            ui.add_space(16.0);
            ui.collapsing(format!("Analysis notes ({})", a.warnings.len()), |ui| { for warning in &a.warnings { ui.label(format!("• {warning}")); } });
            ui.collapsing("Why can the same bytes count toward Flash and RAM?", |ui| {
                ui.label("An initialized variable lives in RAM while the program runs. Its starting value is stored in the load image, usually in Flash, and startup code copies it into RAM. Zero-initialized variables need RAM but no stored payload.");
            });
        });
    }
    pub(super) fn files(&mut self, ui: &mut egui::Ui, a: &Analysis) {
        ui.weak("Click a file to inspect its symbols. Compilation-unit labels are shown when a source location is unavailable.");
        if self.tree {
            egui::ScrollArea::vertical().show(ui, |ui| {
                tree(ui, &a.tree, &self.search.to_lowercase(), "project");
            });
            return;
        }
        let rows = a
            .files
            .iter()
            .map(|f| {
                let mut row = Row::new(
                    vec![
                        f.path.clone(),
                        bytes(f.usage.flash),
                        bytes(f.usage.ram),
                        f.symbol_count.to_string(),
                        f.attribution.clone(),
                    ],
                    &[
                        (1, f.usage.flash.into()),
                        (2, f.usage.ram.into()),
                        (3, f.symbol_count as i128),
                    ],
                    format!(
                        "{}\n{} Flash bytes; {} RAM bytes\n{}",
                        f.path, f.usage.flash, f.usage.ram, f.attribution
                    ),
                );
                row.action = Some(f.path.clone());
                row
            })
            .collect();
        if let Some(file) = self.table(
            ui,
            &[
                ("File", "DWARF source path or ELF compilation-unit label"),
                ("Flash", FLASH_HELP),
                ("RAM", RAM_HELP),
                ("Symbols", "Number of attributed symbol records"),
                ("Evidence", "How the file was identified"),
            ],
            rows,
        ) {
            self.selected_file = Some(file);
            self.search.clear();
            self.view = View::Symbols;
        }
    }
    pub(super) fn symbols(&mut self, ui: &mut egui::Ui, a: &Analysis) {
        if let Some(file) = self.selected_file.clone() {
            ui.horizontal(|ui| {
                ui.label(format!("File: {file}"));
                if ui.small_button("Show all files").clicked() {
                    self.selected_file = None;
                }
            });
        }
        ui.weak("ELF size is the declared size. Flash/RAM columns assign shared bytes only once. Hover a row for metadata.");
        let rows = a.symbols.iter().filter(|s| self.selected_file.as_ref().is_none_or(|file| {
            let owner = s.source_file.as_ref().or(s.compilation_unit.as_ref()).map(|s| s.replace('\\', "/")).unwrap_or_else(|| "[unattributed]".into()); &owner == file
        })).map(|s| Row::new(vec![s.demangled_name.clone(), bytes(s.size), bytes(s.usage.flash), bytes(s.usage.ram), s.kind.clone(), s.section.clone(), format!("{:#010x}", s.address)], &[(1,s.size.into()),(2,s.usage.flash.into()),(3,s.usage.ram.into()),(6,s.address.into())],
            format!("{}\nELF size: {} B · Flash: {} B · RAM: {} B\nAddress: {:#x} · normalized: {:#x}\nSection: {} · weak: {}\nSource: {}:{}\nCompilation unit: {}\n{}", s.name, s.size, s.usage.flash, s.usage.ram, s.address, s.normalized_address, s.section, s.weak, s.source_file.as_deref().unwrap_or("Unknown"), s.source_line.map(|l| l.to_string()).unwrap_or_else(|| "?".into()), s.compilation_unit.as_deref().unwrap_or("Unknown"), s.attribution))).collect();
        self.table(
            ui,
            &[
                ("Symbol", "Demangled C++/Rust name where available"),
                (
                    "ELF size",
                    "Size declared by the ELF symbol table; zero sizes are not guessed",
                ),
                ("Flash", FLASH_HELP),
                ("RAM", RAM_HELP),
                ("Kind", "Function, data object or label"),
                ("Section", "The containing ELF section"),
                (
                    "Address",
                    "Raw symbol value; ARM Thumb functions may have bit 0 set",
                ),
            ],
            rows,
        );
    }
    pub(super) fn sections(&mut self, ui: &mut egui::Ui, a: &Analysis) {
        let rows = a.sections.iter().map(|s| Row::new(vec![s.name.clone(), bytes(s.size), bytes(s.usage.flash), bytes(s.usage.ram), format!("{:#x}",s.address), s.load_address.map(|a| format!("{a:#x}")).unwrap_or_else(|| "Unknown".into()), format!("{:?}",s.classification)], &[(1,s.size.into()),(2,s.usage.flash.into()),(3,s.usage.ram.into()),(4,s.address.into()),(5,s.load_address.unwrap_or(0).into())],
            format!("{}\nSize: {} B · load: {} B · runtime: {} B\nAlignment: {} · flags: {:#x}\nAllocated: {} · writable: {} · executable: {}\n{}", s.name,s.size,s.load_size,s.runtime_size,s.alignment,s.flags,s.allocated,s.writable,s.executable,s.evidence))).collect();
        self.table(
            ui,
            &[
                (
                    "Section",
                    "Names are descriptive; accounting uses ELF attributes",
                ),
                (
                    "Size",
                    "Declared section size, including non-allocated debug sections",
                ),
                ("Flash", FLASH_HELP),
                ("RAM", RAM_HELP),
                ("Run address", "Virtual address at runtime"),
                (
                    "Load address",
                    "Physical address derived from a matching PT_LOAD segment",
                ),
                (
                    "Classification",
                    "Inferred memory role; hover for flags and evidence",
                ),
            ],
            rows,
        );
    }
    pub(super) fn memory_map(&mut self, ui: &mut egui::Ui, a: &Analysis) {
        ui.weak("Load and runtime ranges are separate views of storage; do not add the two address spaces together.");
        let rows = a
            .memory_map
            .iter()
            .map(|r| {
                Row::new(
                    vec![
                        r.name.clone(),
                        format!("{:#010x}", r.address),
                        format!("{:#010x}", r.address + r.size),
                        bytes(r.size),
                        r.space.clone(),
                    ],
                    &[
                        (1, r.address.into()),
                        (2, (r.address + r.size).into()),
                        (3, r.size.into()),
                    ],
                    format!("{}\n{} bytes\n{}", r.name, r.size, r.evidence),
                )
            })
            .collect();
        self.table(
            ui,
            &[
                ("Range", "A load payload or runtime section"),
                ("Start", "Inclusive start address"),
                ("End", "Exclusive end address"),
                ("Size", "Address range length"),
                ("Address space", "Inferred storage role"),
            ],
            rows,
        );
    }
    pub(super) fn stack_view(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.add_enabled_ui(self.receiver.is_none(), |ui| {
                if ui.button("Open .su file…").clicked() {
                    self.pick_stack(false);
                }
                if ui.button("Scan build directory…").clicked() {
                    self.pick_stack(true);
                }
            });
        });
        ui.label("Compiler-reported local frames · Call-chain total: unknown").on_hover_text("Local stack excludes callers, callees and interrupt overhead. Recursive or indirect calls require additional analysis.");
        let Some(report) = &self.stack else {
            ui.add_space(15.0);
            ui.label("Build with -fstack-usage, then select a .su file or its build directory. Use reports from the same firmware build.");
            return;
        };
        ui.collapsing("Stack analysis notes", |ui| {
            for w in &report.warnings {
                ui.label(w);
            }
        });
        let rows = report
            .entries
            .iter()
            .map(|e| {
                Row::new(
                    vec![
                        e.function.clone(),
                        bytes(e.local_bytes),
                        e.qualifier.clone(),
                        format!("{}:{}", e.source_file, e.source_line),
                        e.symbol_candidates.len().to_string(),
                    ],
                    &[
                        (1, e.local_bytes.into()),
                        (4, e.symbol_candidates.len() as i128),
                    ],
                    format!(
                        "{}\n{}\nELF matches: {:?}",
                        e.report_file, e.evidence, e.symbol_candidates
                    ),
                )
            })
            .collect();
        self.table(ui, &[("Function","Compiler function label"),("Local frame","Compiler reported bytes, not a call-chain estimate"),("Qualifier","static: fixed frame; dynamic,bounded: compiler bound; dynamic: total may be unbounded"),("Source","Location reported by the compiler"),("ELF matches","Exact name matches only. Zero is unresolved; more than one is ambiguous.")], rows);
    }
    pub(super) fn compare_view(&mut self, ui: &mut egui::Ui) {
        if ui
            .add_enabled(
                self.receiver.is_none(),
                egui::Button::new("Select older build…"),
            )
            .clicked()
        {
            self.pick_elf(true);
        }
        let Some(c) = &self.comparison else {
            ui.add_space(20.0);
            ui.label("Open your current firmware, then select an older build to see what grew or shrank.");
            return;
        };
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.label(format!("Older: {}\nCurrent: {}", c.old_path, c.new_path));
            ui.add_space(12.0);
            ui.heading(format!(
                "Flash {:+} B    ·    RAM {:+} B",
                c.flash_delta, c.ram_delta
            ));
            ui.label(format!(
                "Flash: {} → {}    RAM: {} → {}",
                bytes(c.old.flash),
                bytes(c.new.flash),
                bytes(c.old.ram),
                bytes(c.new.ram)
            ));
            ui.weak("Positive values mean the current build uses more memory.");
            for (title, changes) in [("Changed files", &c.files), ("Changed symbols", &c.symbols)] {
                ui.add_space(15.0);
                ui.heading(title);
                if changes.is_empty() {
                    ui.label("No memory contribution changes.");
                }
                egui::Grid::new(title).striped(true).show(ui, |ui| {
                    ui.strong("Flash delta");
                    ui.strong("RAM delta");
                    ui.strong("Identity");
                    ui.end_row();
                    for change in changes {
                        ui.monospace(format!("{:+} B", change.flash_delta));
                        ui.monospace(format!("{:+} B", change.ram_delta));
                        ui.label(&change.identity).on_hover_text(&change.status);
                        ui.end_row();
                    }
                });
            }
            ui.collapsing("Comparison notes", |ui| {
                for warning in &c.warnings {
                    ui.label(warning);
                }
            });
        });
    }
}
fn metric(ui: &mut egui::Ui, title: &str, value: u64, help: &str) {
    egui::Frame::group(ui.style())
        .inner_margin(18.0)
        .show(ui, |ui| {
            ui.set_min_width(210.0);
            ui.label(title).on_hover_text(help);
            ui.label(RichText::new(bytes(value)).size(29.0).color(ACCENT))
                .on_hover_text(format!("{value} bytes\n{help}"));
        });
}
fn tree(ui: &mut egui::Ui, node: &FileTree, filter: &str, id: &str) {
    fn matches(node: &FileTree, filter: &str) -> bool {
        node.name.to_lowercase().contains(filter)
            || node.children.iter().any(|c| matches(c, filter))
    }
    if !matches(node, filter) {
        return;
    }
    let label = format!(
        "{}    {} Flash · {} RAM",
        node.name,
        bytes(node.usage.flash),
        bytes(node.usage.ram)
    );
    if node.children.is_empty() {
        ui.label(label);
    } else {
        egui::CollapsingHeader::new(label)
            .id_salt(id)
            .default_open(true)
            .show(ui, |ui| {
                let next_filter = if node.name.to_lowercase().contains(filter) {
                    ""
                } else {
                    filter
                };
                for (index, child) in node.children.iter().enumerate() {
                    tree(ui, child, next_filter, &format!("{id}/{index}"));
                }
            });
    }
}
