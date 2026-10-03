use super::{Explorer, View};
use eframe::egui::{self, RichText};
use egui_extras::{Column, TableBuilder};
use firmware_analysis_core::{format_bytes as bytes, Analysis, FileTree};

pub const ACCENT: egui::Color32 = egui::Color32::from_rgb(113, 185, 219);
pub(super) const TEXT_SELECTION: egui::Color32 = egui::Color32::from_rgb(48, 105, 163);
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
        let editing_text = ui
            .memory(|m| m.focused())
            .is_some_and(|id| egui::TextEdit::load_state(ui.ctx(), id).is_some());
        let navigate = if !editing_text && !ui.memory(|m| m.any_popup_open()) {
            ui.input_mut(|input| {
                if input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown) {
                    Some(true)
                } else if input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp) {
                    Some(false)
                } else {
                    None
                }
            })
        } else {
            None
        };
        let mut scroll_to = None;
        if let Some(down) = navigate.filter(|_| !rows.is_empty()) {
            let current = rows.iter().position(|item| {
                self.details
                    .as_ref()
                    .is_some_and(|(name, tip)| name == &item.cells[0] && tip == &item.tip)
            });
            let next = match current {
                None => 0,
                Some(index) if down => (index + 1).min(rows.len() - 1),
                Some(index) => index.saturating_sub(1),
            };
            self.details = Some((rows[next].cells[0].clone(), rows[next].tip.clone()));
            scroll_to = Some(next);
        }
        let mut clicked = None;
        let height = ui.available_height();
        egui::ScrollArea::horizontal()
            .id_salt("table_horizontal")
            .show(ui, |ui| {
                ui.set_min_width(
                    (250.0 + (headers.len() - 2) as f32 * 96.0 + 100.0).max(ui.available_width()),
                );
                ui.set_min_height(height);
                let table_clip = ui.clip_rect();
                let mut expanded_detail = None;
                let mut table = TableBuilder::new(ui)
                    .striped(false)
                    .sense(egui::Sense::click())
                    .resizable(true)
                    .cell_layout(egui::Layout::top_down(egui::Align::Min));
                if let Some(index) = scroll_to {
                    table = table.scroll_to_row(index, None);
                }
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
                    .body(|mut body| {
                        let width = body.widths().iter().sum::<f32>()
                            + (headers.len() - 1) as f32 * body.ui_mut().spacing().item_spacing.x;
                        let expanded = rows.iter().position(|item| {
                            self.details.as_ref().is_some_and(|(name, tip)| {
                                name == &item.cells[0] && tip == &item.tip
                            })
                        });
                        let detail_text = expanded.map(|index| {
                            let ui = body.ui_mut();
                            ui.painter().layout(
                                rows[index].tip.clone(),
                                egui::TextStyle::Body.resolve(ui.style()),
                                ui.visuals().text_color(),
                                (width - 24.0).max(100.0),
                            )
                        });
                        let heights = (0..rows.len()).map(|i| {
                            if Some(i) == expanded {
                                23.0 + detail_text.as_ref().map_or(0.0, |t| t.size().y)
                                    + 20.0
                                    + if rows[i].action.is_some() { 28.0 } else { 0.0 }
                            } else {
                                23.0
                            }
                        });
                        body.heterogeneous_rows(heights, |mut row| {
                            let item = &rows[row.index()];
                            let open = Some(row.index()) == expanded;
                            row.set_selected(open);
                            let mut line_clicked = false;
                            let mut detail_ui = None;
                            for (index, cell) in item.cells.iter().enumerate() {
                                row.col(|ui| {
                                    if index == 0 && open {
                                        let rect = egui::Rect::from_min_size(
                                            ui.max_rect().min + egui::vec2(12.0, 29.0),
                                            egui::vec2(width - 24.0, ui.max_rect().height() - 29.0),
                                        );
                                        let clip = egui::Rect::from_min_max(
                                            egui::pos2(rect.left(), ui.clip_rect().top()),
                                            egui::pos2(rect.right(), ui.clip_rect().bottom()),
                                        );
                                        let mut child = ui.new_child(
                                            egui::UiBuilder::new()
                                                .id_salt("row_details")
                                                .max_rect(rect)
                                                .layout(egui::Layout::top_down(egui::Align::Min)),
                                        );
                                        child.set_clip_rect(clip.intersect(rect));
                                        let background = egui::Rect::from_min_size(
                                            ui.max_rect().min + egui::vec2(0.0, 23.0),
                                            egui::vec2(width, ui.max_rect().height() - 23.0),
                                        );
                                        let background_clip = egui::Rect::from_min_max(
                                            egui::pos2(background.left(), ui.clip_rect().top()),
                                            egui::pos2(background.right(), ui.clip_rect().bottom()),
                                        )
                                        .intersect(background)
                                        .intersect(table_clip);
                                        child
                                            .set_clip_rect(child.clip_rect().intersect(table_clip));
                                        detail_ui = Some((child, background, background_clip));
                                    }
                                    let numeric = item.values[index].is_some();
                                    ui.allocate_ui_with_layout(
                                        egui::vec2(ui.available_width(), 23.0),
                                        if numeric {
                                            egui::Layout::right_to_left(egui::Align::Center)
                                        } else {
                                            egui::Layout::left_to_right(egui::Align::Center)
                                        },
                                        |ui| {
                                            let display = if self.view == View::Files && index == 0
                                            {
                                                path_tail(ui, cell)
                                            } else {
                                                cell.clone()
                                            };
                                            let label = if numeric || index == 0 {
                                                RichText::new(display).monospace()
                                            } else {
                                                RichText::new(display)
                                            };
                                            let response = ui.add(
                                                egui::Label::new(label)
                                                    .truncate()
                                                    .selectable(false)
                                                    .sense(egui::Sense::hover()),
                                            );
                                            line_clicked |= response.clicked();
                                            response.on_hover_text(cell);
                                        },
                                    );
                                });
                            }
                            let response = row.response();
                            line_clicked |= response.clicked()
                                && response
                                    .interact_pointer_pos()
                                    .is_some_and(|p| p.y <= response.rect.top() + 23.0);
                            if line_clicked {
                                self.details = if open {
                                    None
                                } else {
                                    Some((item.cells[0].clone(), item.tip.clone()))
                                };
                            }
                            if let Some((ui, background, clip)) = detail_ui {
                                expanded_detail = Some((
                                    ui,
                                    background,
                                    clip,
                                    detail_text.clone(),
                                    item.action.clone(),
                                ));
                            }
                        });
                    });
                // TableBuilder paints its full-height resize dividers after the body.
                // Paint the spanning detail area last so those dividers cannot cross
                // its text or capture selection gestures over the detail widgets.
                if let Some((mut ui, background, clip, text, action)) = expanded_detail {
                    ui.painter().with_clip_rect(clip).rect_filled(
                        background,
                        0.0,
                        ui.visuals().selection.bg_fill,
                    );
                    ui.visuals_mut().selection.bg_fill = TEXT_SELECTION;
                    if let Some(text) = text {
                        ui.add(egui::Label::new(text).selectable(true));
                    }
                    if let Some(file) = action {
                        if ui.small_button("Show symbols for this file").clicked() {
                            clicked = Some(file);
                        }
                    }
                }
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
        for region in &a.options.regions {
            let usage = firmware_analysis_core::regions::region_usage(a, region);
            ui.add(
                egui::ProgressBar::new((usage.used as f64 / region.size.max(1) as f64) as f32)
                    .desired_height(14.0)
                    .text(format!(
                        "{}: {} used / {} free / {} total",
                        region.name,
                        bytes(usage.used),
                        bytes(usage.free),
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
        self.overview_pie(ui, a);
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
                    String::new(),
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
        let rows = a
            .symbols
            .iter()
            .filter(|s| self.kind_filter == "All" || s.kind == self.kind_filter)
            .filter(|s| {
                self.selected_file.as_ref().is_none_or(|file| {
                    let owner = s
                        .source_file
                        .as_ref()
                        .or(s.compilation_unit.as_ref())
                        .map(|s| s.replace('\\', "/"))
                        .unwrap_or_else(|| "[unattributed]".into());
                    &owner == file
                })
            })
            .map(|s| {
                Row::new(
                    vec![
                        s.demangled_name.clone(),
                        bytes(s.size),
                        bytes(s.usage.flash),
                        bytes(s.usage.ram),
                        s.kind.clone(),
                        s.section.clone(),
                        format!("{:#010x}", s.address),
                    ],
                    &[
                        (1, s.size.into()),
                        (2, s.usage.flash.into()),
                        (3, s.usage.ram.into()),
                        (6, s.address.into()),
                    ],
                    format!(
                        "{}{}Weak symbol: {}\nSource: {}:{}\nCompilation unit: {}\n{}",
                        if s.name != s.demangled_name {
                            format!("Linker name: {}\n", s.name)
                        } else {
                            String::new()
                        },
                        if s.address != s.normalized_address {
                            format!("Normalized address: {:#010x}\n", s.normalized_address)
                        } else {
                            String::new()
                        },
                        s.weak,
                        s.source_file.as_deref().unwrap_or("Unknown"),
                        s.source_line
                            .map(|l| l.to_string())
                            .unwrap_or_else(|| "?".into()),
                        s.compilation_unit.as_deref().unwrap_or("Unknown"),
                        s.attribution
                    ),
                )
            })
            .collect();
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
        let rows = a.sections.iter().map(|s| Row::new(vec![s.name.clone(), bytes(s.size), bytes(s.usage.flash), bytes(s.usage.ram), format!("{:#x}",s.address), load_address(s.load_address, s.load_size), classification(s.classification).into()], &[(1,s.size.into()),(2,s.usage.flash.into()),(3,s.usage.ram.into()),(4,s.address.into()),(5,s.load_address.unwrap_or(0).into())],
            format!("Load size: {} B / runtime size: {} B\nAlignment: {} / flags: {:#x}\nAllocated: {} / writable: {} / executable: {}\n{}", s.load_size,s.runtime_size,s.alignment,s.flags,s.allocated,s.writable,s.executable,s.evidence))).collect();
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
        if a.options.regions.is_empty() {
            ui.label("Region capacity and free space are unknown. Use Layout → Load memory regions to load a target layout JSON.");
        } else {
            if self
                .selected_region
                .is_some_and(|i| i >= a.options.regions.len())
            {
                self.selected_region = None;
            }
            ui.small("Select a region to inspect its symbols. Free space excludes static ELF occupancy only; runtime heap and stack demand may use it.");
            let previous = self.selected_region;
            ui.selectable_value(&mut self.selected_region, None, "All address ranges");
            egui::ScrollArea::vertical().id_salt("region_summary").max_height(180.0).show(ui, |ui| {
                for (index, region) in a.options.regions.iter().enumerate() {
                    let usage = firmware_analysis_core::regions::region_usage(a, region);
                    ui.selectable_value(&mut self.selected_region, Some(index), format!(
                        "{} ({:?})  {:#010x}–{:#010x}  |  {} used / {} free / {} total  ({:.1}%)",
                        region.name, region.kind, region.start, region.start.saturating_add(region.size),
                        bytes(usage.used), bytes(usage.free), bytes(region.size),
                        usage.used as f64 * 100.0 / region.size.max(1) as f64));
                }
            });
            if previous != self.selected_region {
                self.details = None;
                self.search.clear();
                self.sort_column = 1;
                self.descending = false;
            }
            if let Some(index) = self.selected_region {
                let region = &a.options.regions[index];
                let usage = firmware_analysis_core::regions::region_usage(a, region);
                ui.label(format!("Symbols in {}", region.name));
                ui.small("Usage includes section padding and reservations. Aliases and zero-sized labels are listed; symbol sizes do not sum to region usage. Boundary-crossing ranges count only bytes inside the region.");
                if usage.symbols.is_empty() {
                    ui.label("No symbols available in this region. Stripped firmware can still occupy space.");
                }
                let rows = usage
                    .symbols
                    .iter()
                    .map(|entry| {
                        let s = &a.symbols[entry.symbol_index];
                        Row::new(
                            vec![
                                s.demangled_name.clone(),
                                format!("{:#010x}", entry.address),
                                bytes(s.size),
                                s.section.clone(),
                                entry.placement.into(),
                                s.source_file
                                    .as_ref()
                                    .or(s.compilation_unit.as_ref())
                                    .cloned()
                                    .unwrap_or_else(|| "[unattributed]".into()),
                            ],
                            &[(1, entry.address.into()), (2, s.size.into())],
                            format!(
                                "Linker name: {}\nRuntime address: {:#010x}\nELF size: {} B\n{}",
                                s.name, s.normalized_address, s.size, s.attribution
                            ),
                        )
                    })
                    .collect();
                self.table(
                    ui,
                    &[
                        ("Symbol", "Symbols intersecting the selected region"),
                        (
                            "Address",
                            "Placement address in this region; Thumb bit normalized",
                        ),
                        ("ELF size", "Full declared symbol size; aliases may overlap"),
                        ("Section", "Containing ELF section"),
                        ("Placement", "Runtime storage or initial load image"),
                        (
                            "Source",
                            "Source file or compilation-unit label when available",
                        ),
                    ],
                    rows,
                );
                return;
            }
        }
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
                    r.evidence.clone(),
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
            ui.label("Build with -fstack-usage, then rescan the build folder and select firmware to load discovered reports automatically. Use reports from the same firmware build.");
            return;
        };
        ui.small("Reports may span multiple targets. Expand a row for its report path; inspect analysis notes for matching limitations.");
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
                        "Flash: {} to {}\nRAM: {} to {}",
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
/// Keep the end of a file path visible as its column is resized.
fn path_tail(ui: &egui::Ui, path: &str) -> String {
    let font = egui::TextStyle::Monospace.resolve(ui.style());
    let width = ui.available_width();
    let measure = |text: &str| {
        ui.painter()
            .layout_no_wrap(text.into(), font.clone(), ui.visuals().text_color())
            .size()
            .x
    };
    if measure(path) <= width {
        return path.into();
    }
    // Search only UTF-8 boundaries, retaining the longest suffix that fits.
    let boundaries: Vec<_> = path
        .char_indices()
        .map(|(i, _)| i)
        .chain(std::iter::once(path.len()))
        .collect();
    let mut low = 0;
    let mut high = boundaries.len() - 1;
    while low < high {
        let middle = low + (high - low) / 2;
        if measure(&format!("...{}", &path[boundaries[middle]..])) <= width {
            high = middle;
        } else {
            low = middle + 1;
        }
    }
    format!("...{}", &path[boundaries[low]..])
}

fn load_address(address: Option<u64>, load_size: u64) -> String {
    match address {
        Some(address) => format!("0x{address:08X}"),
        None if load_size == 0 => "Not applicable (no load payload)".into(),
        None => "Unknown (no matching load segment)".into(),
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
