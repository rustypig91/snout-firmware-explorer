use super::{Explorer, View};
use eframe::egui::{self, RichText};
use egui_extras::{Column, TableBuilder};
use firmware_analysis_core::{format_bytes as bytes, Analysis, FileTree};

pub const ACCENT: egui::Color32 = egui::Color32::from_rgb(113, 185, 219);
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
        self.visible_rows = rows.len();
        let mut clicked = None;
        let height = ui.available_height();
        egui::ScrollArea::horizontal()
            .id_salt("table_horizontal")
            .show(ui, |ui| {
                ui.set_min_width(
                    (250.0 + (headers.len() - 2) as f32 * 96.0 + 100.0).max(ui.available_width()),
                );
                ui.set_min_height(height);
                let mut table = TableBuilder::new(ui)
                    .striped(false)
                    .sense(egui::Sense::click())
                    .resizable(true)
                    .cell_layout(egui::Layout::left_to_right(egui::Align::Center));
                for (index, _) in headers.iter().enumerate() {
                    table = table.column(if index == headers.len() - 1 {
                        Column::remainder().at_least(100.0).clip(true)
                    } else if index == 0 {
                        Column::initial(250.0).at_least(120.0).clip(true)
                    } else {
                        Column::initial(96.0).at_least(65.0).clip(true)
                    });
                }
                table
                    .header(26.0, |mut header| {
                        for (index, (title, help)) in headers.iter().enumerate() {
                            header.col(|ui| {
                                if sort_header(
                                    ui,
                                    title,
                                    (sort == index).then_some(self.descending),
                                )
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
                        body.rows(23.0, rows.len(), |mut row| {
                            let item = &rows[row.index()];
                            row.set_selected(
                                self.details
                                    .as_ref()
                                    .is_some_and(|(_, detail)| detail == &item.tip),
                            );
                            for (index, cell) in item.cells.iter().enumerate() {
                                row.col(|ui| {
                                    let numeric = item.values[index].is_some();
                                    ui.with_layout(
                                        if numeric {
                                            egui::Layout::right_to_left(egui::Align::Center)
                                        } else {
                                            egui::Layout::left_to_right(egui::Align::Center)
                                        },
                                        |ui| {
                                            let response = if index == 0 && item.action.is_some() {
                                                ui.link(cell)
                                            } else if numeric || index == 0 {
                                                ui.add(
                                                    egui::Label::new(
                                                        RichText::new(cell).monospace(),
                                                    )
                                                    .truncate(),
                                                )
                                            } else {
                                                ui.add(egui::Label::new(cell).truncate())
                                            };
                                            if response.clicked() {
                                                clicked = item.action.clone();
                                                self.details =
                                                    Some((item.cells[0].clone(), item.tip.clone()));
                                                self.show_details = true;
                                                self.show_notes = false;
                                            }
                                            response.on_hover_text(cell);
                                        },
                                    );
                                });
                            }
                            if row.response().clicked() {
                                self.details = Some((item.cells[0].clone(), item.tip.clone()));
                                self.show_details = true;
                                self.show_notes = false;
                            }
                        });
                    });
            });
        clicked
    }
    pub(super) fn overview(&mut self, ui: &mut egui::Ui, a: &Analysis) {
        ui.horizontal(|ui| {
            metric(ui, "Flash payload", a.totals.flash, FLASH_HELP);
            ui.separator();
            metric(ui, "Static RAM", a.totals.ram, RAM_HELP);
            ui.separator();
            ui.weak(format!("ELF {}", bytes(a.metadata.file_size)))
                .on_hover_text(
                    "File size includes debug information; it is not the programmed image size.",
                );
            if let Some(c) = &self.comparison {
                ui.separator();
                ui.label(format!(
                    "Change: Flash {:+} B / RAM {:+} B",
                    c.flash_delta, c.ram_delta
                ));
            }
        });
        for region in &self.options.regions {
            let used: u64 = a
                .sections
                .iter()
                .filter(|s| {
                    let addr = match region.kind {
                        firmware_analysis_core::MemoryKind::Flash => s.load_address,
                        _ => Some(s.address),
                    };
                    addr.is_some_and(|addr| {
                        addr >= region.start
                            && addr
                                .checked_add(s.size)
                                .is_some_and(|end| end <= region.start.saturating_add(region.size))
                    })
                })
                .map(|s| match region.kind {
                    firmware_analysis_core::MemoryKind::Flash => s.usage.flash,
                    _ => s.usage.ram,
                })
                .sum();
            ui.add(
                egui::ProgressBar::new((used as f64 / region.size.max(1) as f64) as f32)
                    .desired_height(14.0)
                    .text(format!(
                        "{}: {} / {}",
                        region.name,
                        bytes(used),
                        bytes(region.size)
                    )),
            );
        }
        ui.horizontal(|ui| {
            ui.weak(if self.options.regions.is_empty() { "Inferred memory types / capacity unknown" } else { "Configured memory regions / unmatched ranges inferred" }).on_hover_text("ELF attributes describe loading and permissions, not physical memory technology. Configure regions using Layout.");
            ui.separator();
            ui.weak(format!("Unattributed: {} Flash / {} RAM", bytes(a.unattributed.flash), bytes(a.unattributed.ram))).on_hover_text("Unknown file owners, padding and reservations. File totals still reconcile with the overview.");
        });
        ui.separator();
        ui.horizontal(|ui| {
            ui.strong("ALLOCATED SECTIONS");
            if ui.small_button("Files").clicked() { self.change_view(View::Files); }
            if ui.small_button("Symbols").clicked() { self.change_view(View::Symbols); }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label("?").on_hover_text("Initialized variables need RAM while running and initial values in Flash. No-payload storage such as BSS needs RAM only. Select a row to inspect its evidence.");
            });
        });
        let rows = a
            .sections
            .iter()
            .filter(|s| s.allocated)
            .map(|s| {
                Row::new(
                    vec![
                        s.name.clone(),
                        bytes(s.usage.flash),
                        bytes(s.usage.ram),
                        bytes(s.load_size),
                        bytes(s.runtime_size),
                        classification(s.classification).into(),
                    ],
                    &[
                        (1, s.usage.flash.into()),
                        (2, s.usage.ram.into()),
                        (3, s.load_size.into()),
                        (4, s.runtime_size.into()),
                    ],
                    format!(
                        "{}\nFlash: {} B / RAM: {} B\nRun address: {:#x}\nLoad address: {:?}\n{}",
                        s.name, s.usage.flash, s.usage.ram, s.address, s.load_address, s.evidence
                    ),
                )
            })
            .collect();
        self.table(
            ui,
            &[
                ("Section", "Allocated sections; click a row for details"),
                ("Flash", FLASH_HELP),
                ("RAM", RAM_HELP),
                ("Load", "Bytes stored in the image"),
                ("Runtime", "Bytes present while executing"),
                (
                    "Role",
                    "Memory role inferred from ELF attributes or configured regions",
                ),
            ],
            rows,
        );
    }
    pub(super) fn directory_tree(&mut self, ui: &mut egui::Ui, a: &Analysis) {
        let mut selected = None;
        egui::ScrollArea::both().show(ui, |ui| {
            tree(ui, &a.tree, "", "project", "", &mut selected);
        });
        if let Some(path) = selected {
            if let Some(file) = a
                .files
                .iter()
                .find(|f| f.path.trim_start_matches('/') == path)
            {
                self.change_view(View::Symbols);
                self.selected_file = Some(file.path.clone());
            }
        }
    }
    pub(super) fn files(&mut self, ui: &mut egui::Ui, a: &Analysis) {
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
            self.change_view(View::Symbols);
            self.selected_file = Some(file);
        }
    }
    pub(super) fn symbols(&mut self, ui: &mut egui::Ui, a: &Analysis) {
        if let Some(file) = &self.selected_file {
            ui.weak(file);
        }
        let rows = a.symbols.iter().filter(|s| self.kind_filter == "All" || s.kind == self.kind_filter).filter(|s| self.selected_file.as_ref().is_none_or(|file| {
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
        let rows = a.sections.iter().map(|s| Row::new(vec![s.name.clone(), bytes(s.size), bytes(s.usage.flash), bytes(s.usage.ram), format!("{:#x}",s.address), s.load_address.map(|a| format!("{a:#x}")).unwrap_or_else(|| "Unknown".into()), classification(s.classification).into()], &[(1,s.size.into()),(2,s.usage.flash.into()),(3,s.usage.ram.into()),(4,s.address.into()),(5,s.load_address.unwrap_or(0).into())],
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
        ui.small("Load and runtime are separate address spaces").on_hover_text("Do not add load and runtime ranges together; the same storage can appear in both views.");
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
        ui.label("Compiler-reported local frames · Call-chain total: unknown").on_hover_text("Local stack excludes callers, callees and interrupt overhead. Recursive or indirect calls require additional analysis.");
        let Some(report) = &self.stack else {
            self.visible_rows = 0;
            ui.add_space(15.0);
            ui.label("Build with -fstack-usage, then select a .su file or its build directory. Use reports from the same firmware build.");
            return;
        };
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
        let Some(c) = &self.comparison else {
            self.visible_rows = 0;
            ui.weak(
                "Use Compare in the header to select an older build. Current minus older is shown.",
            );
            return;
        };
        ui.horizontal(|ui| {
            ui.label(format!("Flash {:+} B", c.flash_delta))
                .on_hover_text(format!("{} to {}", bytes(c.old.flash), bytes(c.new.flash)));
            ui.separator();
            ui.label(format!("RAM {:+} B", c.ram_delta))
                .on_hover_text(format!("{} to {}", bytes(c.old.ram), bytes(c.new.ram)));
            ui.separator();
            ui.weak("Current minus older")
                .on_hover_text(format!("Older: {}\nCurrent: {}", c.old_path, c.new_path));
        });
        let changes = if self.comparison_symbols {
            &c.symbols
        } else {
            &c.files
        };
        let rows = changes
            .iter()
            .map(|c| {
                Row::new(
                    vec![
                        c.identity.clone(),
                        format!("{:+} B", c.flash_delta),
                        format!("{:+} B", c.ram_delta),
                        c.status.clone(),
                    ],
                    &[(1, c.flash_delta), (2, c.ram_delta)],
                    format!(
                        "{}\n{}\nFlash: {} to {}\nRAM: {} to {}",
                        c.identity,
                        c.status,
                        bytes(c.old.flash),
                        bytes(c.new.flash),
                        bytes(c.old.ram),
                        bytes(c.new.ram)
                    ),
                )
            })
            .collect();
        self.table(
            ui,
            &[
                (
                    "Identity",
                    "File or symbol identity; select to inspect the change",
                ),
                ("Flash delta", "Positive values mean growth"),
                ("RAM delta", "Positive values mean growth"),
                ("Status", "Added, removed or changed"),
            ],
            rows,
        );
    }
}
fn classification(value: firmware_analysis_core::Classification) -> &'static str {
    use firmware_analysis_core::Classification::*;
    match value {
        ReadOnly => "Read-only",
        InitializedRam => "Initialized RAM",
        NoLoadRam => "RAM / no payload",
        NonAllocated => "Not allocated",
    }
}
/// Draw the direction indicator geometrically, without depending on font glyphs.
fn sort_header(ui: &mut egui::Ui, title: &str, descending: Option<bool>) -> egui::Response {
    let font = egui::TextStyle::Button.resolve(ui.style());
    let text = ui
        .painter()
        .layout_no_wrap(title.into(), font, ui.visuals().text_color());
    let padding = ui.spacing().button_padding;
    let icon_width = if descending.is_some() { 16.0 } else { 0.0 };
    let size = egui::vec2(
        (text.size().x + padding.x * 2.0 + icon_width).max(ui.available_width()),
        (text.size().y + padding.y * 2.0).max(ui.spacing().interact_size.y),
    );
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    let label = match descending {
        Some(true) => format!("{title}, sorted descending"),
        Some(false) => format!("{title}, sorted ascending"),
        None => title.into(),
    };
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), &label)
    });
    if ui.is_rect_visible(rect) {
        let visuals = ui.style().interact(&response);
        if response.hovered() || response.has_focus() {
            ui.painter().rect_filled(rect, 0.0, visuals.weak_bg_fill);
        }
        ui.painter().line_segment(
            [rect.left_bottom(), rect.right_bottom()],
            ui.visuals().widgets.noninteractive.bg_stroke,
        );
        let text_pos = egui::pos2(
            rect.left() + padding.x,
            rect.center().y - text.size().y * 0.5,
        );
        ui.painter().galley(text_pos, text, visuals.text_color());
        if let Some(descending) = descending {
            let center = egui::pos2(rect.right() - padding.x - 5.0, rect.center().y);
            let direction = if descending { 1.0 } else { -1.0 };
            let points = vec![
                center + egui::vec2(-4.0, -2.5 * direction),
                center + egui::vec2(4.0, -2.5 * direction),
                center + egui::vec2(0.0, 2.5 * direction),
            ];
            ui.painter().add(egui::Shape::convex_polygon(
                points,
                visuals.text_color(),
                egui::Stroke::NONE,
            ));
        }
    }
    response
}
fn metric(ui: &mut egui::Ui, title: &str, value: u64, help: &str) {
    ui.label(title).on_hover_text(help);
    ui.label(
        RichText::new(bytes(value))
            .strong()
            .monospace()
            .color(ACCENT),
    )
    .on_hover_text(format!("{value} bytes\n{help}"));
}
fn tree(
    ui: &mut egui::Ui,
    node: &FileTree,
    filter: &str,
    id: &str,
    parent: &str,
    selected: &mut Option<String>,
) {
    fn matches(node: &FileTree, filter: &str) -> bool {
        node.name.to_lowercase().contains(filter)
            || node.children.iter().any(|c| matches(c, filter))
    }
    if !matches(node, filter) {
        return;
    }
    let path = if id == "project" {
        String::new()
    } else if parent.is_empty() {
        node.name.clone()
    } else {
        format!("{parent}/{}", node.name)
    };
    let label = format!(
        "{}  {} / {}",
        node.name,
        bytes(node.usage.flash),
        bytes(node.usage.ram)
    );
    let help = format!(
        "{}\nFlash: {} B\nRAM: {} B",
        node.name, node.usage.flash, node.usage.ram
    );
    if node.children.is_empty() {
        if ui
            .selectable_label(false, label)
            .on_hover_text(help)
            .clicked()
        {
            *selected = Some(path);
        }
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
                    tree(
                        ui,
                        child,
                        next_filter,
                        &format!("{id}/{index}"),
                        &path,
                        selected,
                    );
                }
            })
            .header_response
            .on_hover_text(help);
    }
}
