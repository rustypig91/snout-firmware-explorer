use super::display::{build_relative_path, display_path};
use super::snapshots::{stack_key, symbol_key};
use super::{Explorer, View};
use eframe::egui::{self, RichText};
use egui_extras::{Column, TableBuilder};
use firmware_analysis_core::{format_bytes as bytes, Analysis, FileTree};
use std::{cell::RefCell, rc::Rc};

pub const ACCENT: egui::Color32 = egui::Color32::from_rgb(113, 185, 219);
pub(super) const TEXT_SELECTION: egui::Color32 = egui::Color32::from_rgb(48, 105, 163);
pub(super) const MEMORY_BAR: egui::Color32 = egui::Color32::from_rgba_premultiplied(23, 23, 23, 45);
pub(super) const FLASH_HELP: &str = "Allocated bytes stored in the load image. Initialized RAM data also needs initial values in Flash. Gaps and programmer-specific overhead are excluded.";
pub(super) const RAM_HELP: &str = "Static memory required while running. Includes initialized data, zero-filled storage and explicit reservations. Additional heap and stack demand is not automatically known.";
struct Row {
    cells: Vec<String>,
    search_cells: Vec<String>,
    values: Vec<Option<i128>>,
    tip: String,
    action: Option<String>,
    bar_columns: Vec<usize>,
}

#[derive(PartialEq, Eq)]
struct RowKey {
    revision: u64,
    source: usize,
    view: View,
    file: Option<String>,
    region: Option<usize>,
    kind: String,
    unresolved: bool,
    comparison_group: usize,
    build_root: Option<std::path::PathBuf>,
}

#[derive(Default)]
pub(super) struct TableCache {
    rows: RefCell<Option<(RowKey, Rc<Vec<Row>>)>>,
    prepared: Option<Rc<PreparedTable>>,
}

struct PreparedTable {
    source: Rc<Vec<Row>>,
    search: String,
    sort: usize,
    descending: bool,
    indices: Vec<usize>,
    bar_maxima: Vec<i128>,
}

struct OrderedRows<'a>(&'a PreparedTable);
impl OrderedRows<'_> {
    fn len(&self) -> usize {
        self.0.indices.len()
    }
    fn is_empty(&self) -> bool {
        self.0.indices.is_empty()
    }
    fn iter(&self) -> impl Iterator<Item = &Row> {
        self.0.indices.iter().map(|&i| &self.0.source[i])
    }
}
impl std::ops::Index<usize> for OrderedRows<'_> {
    type Output = Row;
    fn index(&self, index: usize) -> &Row {
        &self.0.source[self.0.indices[index]]
    }
}
impl Row {
    fn new(cells: Vec<String>, numbers: &[(usize, i128)], tip: String) -> Self {
        let mut values = vec![None; cells.len()];
        for &(column, value) in numbers {
            values[column] = Some(value);
        }
        Self {
            search_cells: cells.iter().map(|cell| cell.to_lowercase()).collect(),
            cells,
            values,
            tip,
            action: None,
            bar_columns: vec![],
        }
    }

    fn with_bars(mut self, columns: &[usize]) -> Self {
        self.bar_columns.extend_from_slice(columns);
        self
    }
}
impl Explorer {
    fn cached_rows(&self, source: usize, build: impl FnOnce() -> Vec<Row>) -> Rc<Vec<Row>> {
        let key = RowKey {
            revision: self.report_revision,
            source,
            view: self.view,
            file: self.selected_file.clone(),
            region: self.selected_region,
            kind: self.kind_filter.clone(),
            unresolved: self.stack_show_unresolved,
            comparison_group: self.comparison_group,
            build_root: self.build.as_ref().map(|b| b.root.clone()),
        };
        let mut cache = self.table_cache.rows.borrow_mut();
        if let Some((old, rows)) = &*cache {
            if *old == key {
                return rows.clone();
            }
        }
        let rows = Rc::new(build());
        *cache = Some((key, rows.clone()));
        rows
    }

    fn prepare_table(&mut self, rows: Rc<Vec<Row>>, columns: usize) -> Rc<PreparedTable> {
        let search = self.search.to_lowercase();
        let sort = self.sort_column.min(columns - 1);
        if let Some(cached) = &self.table_cache.prepared {
            if Rc::ptr_eq(&cached.source, &rows)
                && cached.search == search
                && cached.sort == sort
                && cached.descending == self.descending
            {
                return cached.clone();
            }
        }
        let filtered = self
            .table_cache
            .prepared
            .as_ref()
            .filter(|cached| Rc::ptr_eq(&cached.source, &rows) && cached.search == search);
        let (mut indices, bar_maxima) = if let Some(cached) = filtered {
            // A sort-only interaction reuses filtering and numeric scales.
            // Restore source order to retain stable ties in either direction.
            let mut indices = cached.indices.clone();
            indices.sort_unstable();
            (indices, cached.bar_maxima.clone())
        } else {
            let indices: Vec<_> = (0..rows.len())
                .filter(|&i| {
                    search.is_empty() || rows[i].search_cells.iter().any(|c| c.contains(&search))
                })
                .collect();
            let mut maxima = vec![0; columns];
            for &i in &indices {
                for &column in &rows[i].bar_columns {
                    maxima[column] = maxima[column].max(rows[i].values[column].unwrap_or(0));
                }
            }
            (indices, maxima)
        };
        indices.sort_by(|&a, &b| {
            let order = match (rows[a].values[sort], rows[b].values[sort]) {
                (Some(a), Some(b)) => a.cmp(&b),
                _ => rows[a].cells[sort].cmp(&rows[b].cells[sort]),
            };
            if self.descending {
                order.reverse()
            } else {
                order
            }
        });
        let prepared = Rc::new(PreparedTable {
            source: rows,
            search,
            sort,
            descending: self.descending,
            indices,
            bar_maxima,
        });
        self.table_cache.prepared = Some(prepared.clone());
        prepared
    }

    fn table(
        &mut self,
        ui: &mut egui::Ui,
        headers: &[(&str, &str)],
        rows: Rc<Vec<Row>>,
    ) -> Option<String> {
        let sort = self.sort_column.min(headers.len() - 1);
        let prepared = self.prepare_table(rows, headers.len());
        let rows = OrderedRows(&prepared);
        let bar_maxima = &prepared.bar_maxima;
        self.visible_rows = rows.len();
        if rows.is_empty() {
            ui.weak(if self.search.is_empty() {
                "No entries for the current selection."
            } else {
                "No entries match the filter. Clear it to see available entries."
            });
        }
        let editing_text = ui
            .memory(|m| m.focused())
            .is_some_and(|id| egui::TextEdit::load_state(ui.ctx(), id).is_some());
        let navigate = if self.snapshot_dialog.is_none()
            && !editing_text
            && !ui.memory(|m| m.any_popup_open())
        {
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
                        let expanded = self.details.as_ref().and_then(|(name, tip)| {
                            rows.iter()
                                .position(|item| name == &item.cells[0] && tip == &item.tip)
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
                        let draw_row = |mut row: egui_extras::TableRow<'_, '_>| {
                            let item = &rows[row.index()];
                            let open = Some(row.index()) == expanded;
                            row.set_selected(open);
                            let mut line_clicked = false;
                            let mut detail_ui = None;
                            for (index, cell) in item.cells.iter().enumerate() {
                                row.col(|ui| {
                                    if item.bar_columns.contains(&index) && bar_maxima[index] > 0 {
                                        let fraction = item.values[index].unwrap_or(0).max(0)
                                            as f64
                                            / bar_maxima[index] as f64;
                                        let bar = egui::Rect::from_min_size(
                                            ui.max_rect().min,
                                            egui::vec2(
                                                ui.max_rect().width() * fraction as f32,
                                                23.0,
                                            ),
                                        );
                                        if bar.is_positive() {
                                            ui.painter().rect_filled(bar, 0.0, MEMORY_BAR);
                                        }
                                    }
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
                                            let path_cell = (self.view == View::Files
                                                && index == 0)
                                                || (self.view == View::Stack && index == 3);
                                            let display = if path_cell {
                                                path_tail(ui, cell)
                                            } else {
                                                cell.clone()
                                            };
                                            let label = if numeric || index == 0 || path_cell {
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
                        };
                        if expanded.is_some() {
                            let heights = (0..rows.len()).map(|i| {
                                if Some(i) == expanded {
                                    23.0 + detail_text.as_ref().map_or(0.0, |t| t.size().y)
                                        + 20.0
                                        + if rows[i].action.is_some() { 28.0 } else { 0.0 }
                                } else {
                                    23.0
                                }
                            });
                            body.heterogeneous_rows(heights, draw_row);
                        } else {
                            body.rows(23.0, rows.len(), draw_row);
                        }
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
    pub(super) fn directory_tree(&mut self, ui: &mut egui::Ui, a: &Analysis) {
        let mut selected = None;
        egui::ScrollArea::both().show(ui, |ui| {
            tree(ui, &a.tree, "", "project", "", &mut selected, self);
        });
        if let Some(path) = selected {
            if let Some(file) = a
                .files
                .iter()
                .find(|f| f.path.trim_start_matches('/') == path)
            {
                self.show_file_symbols(file.path.clone());
            }
        }
    }
    pub(super) fn files(&mut self, ui: &mut egui::Ui, a: &Analysis) {
        let build_root = self.build.as_ref().map(|build| build.root.as_path());
        let rows = self.cached_rows(a as *const Analysis as usize, || {
            a.files
                .iter()
                .map(|f| {
                    let mut row = Row::new(
                        vec![
                            build_relative_path(&f.path, build_root),
                            self.snapshot_bytes("file", &f.path, "usage.flash", f.usage.flash),
                            self.snapshot_bytes("file", &f.path, "usage.ram", f.usage.ram),
                            self.snapshot_count(
                                "file",
                                &f.path,
                                "symbol_count",
                                f.symbol_count as u64,
                            ),
                            f.attribution.clone(),
                        ],
                        &[
                            (1, f.usage.flash.into()),
                            (2, f.usage.ram.into()),
                            (3, f.symbol_count as i128),
                        ],
                        format!(
                            "{}\n{}\nFlash: {} / RAM: {}",
                            build_relative_path(&f.path, build_root),
                            f.attribution,
                            self.snapshot_bytes("file", &f.path, "usage.flash", f.usage.flash),
                            self.snapshot_bytes("file", &f.path, "usage.ram", f.usage.ram)
                        ),
                    )
                    .with_bars(&[1, 2]);
                    row.action = Some(f.path.clone());
                    row
                })
                .collect()
        });
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
            self.show_file_symbols(file);
        }
    }
    pub(super) fn symbols(&mut self, ui: &mut egui::Ui, a: &Analysis) {
        if let Some(file) = &self.selected_file {
            ui.weak(display_path(file));
        }
        let rows = self.cached_rows(a as *const Analysis as usize, || { a
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
                        self.snapshot_bytes("symbol", &symbol_key(s), "size", s.size),
                        self.snapshot_bytes("symbol", &symbol_key(s), "usage.flash", s.usage.flash),
                        self.snapshot_bytes("symbol", &symbol_key(s), "usage.ram", s.usage.ram),
                        s.kind.clone(),
                        s.section.clone(),
                        self.snapshot_address("symbol", &symbol_key(s), "address", s.address),
                    ],
                    &[
                        (1, s.size.into()),
                        (2, s.usage.flash.into()),
                        (3, s.usage.ram.into()),
                        (6, s.address.into()),
                    ],
                    format!(
                        "Address: {}\nSection: {} (index {})\n{}{}Weak symbol: {}\nSource: {}:{}\nCompilation unit: {}\n{}",
                        self.snapshot_address("symbol", &symbol_key(s), "address", s.address),
                        s.section,
                        s.section_index,
                        if s.name != s.demangled_name {
                            format!("Linker name: {}\n", s.name)
                        } else {
                            String::new()
                        },
                        if s.address != s.normalized_address {
                            format!("Normalized address: {}\n", self.snapshot_address("symbol", &symbol_key(s), "normalized_address", s.normalized_address))
                        } else {
                            String::new()
                        },
                        s.weak,
                        display_path(s.source_file.as_deref().unwrap_or("Unknown")),
                        s.source_line
                            .map(|l| l.to_string())
                            .unwrap_or_else(|| "?".into()),
                        display_path(s.compilation_unit.as_deref().unwrap_or("Unknown")),
                        s.attribution
                    ),
                )
                .with_bars(&[1, 2, 3])
            })
            .collect() });
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
        let rows = self.cached_rows(a as *const Analysis as usize, || { a.sections.iter().map(|s| Row::new(vec![s.name.clone(), self.snapshot_bytes("section", &s.name, "size", s.size), self.snapshot_bytes("section", &s.name, "usage.flash", s.usage.flash), self.snapshot_bytes("section", &s.name, "usage.ram", s.usage.ram), self.snapshot_address("section", &s.name, "address", s.address), s.load_address.map(|v| self.snapshot_address("section", &s.name, "load_address", v)).unwrap_or_else(|| load_address(None, s.load_size)), classification(s.classification).into()], &[(1,s.size.into()),(2,s.usage.flash.into()),(3,s.usage.ram.into()),(4,s.address.into()),(5,s.load_address.unwrap_or(0).into())],
            format!("Load size: {} / runtime size: {}\nAlignment: {} / flags: {:#x}\nAllocated: {} / writable: {} / executable: {}\n{}", self.snapshot_bytes("section", &s.name, "load_size", s.load_size),self.snapshot_bytes("section", &s.name, "runtime_size", s.runtime_size),self.snapshot_bytes("section", &s.name, "alignment", s.alignment),s.flags,s.allocated,s.writable,s.executable,s.evidence)).with_bars(&[1, 2, 3])).collect() });
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
        ui.add_enabled_ui(self.receiver.is_none(), |ui| {
            ui.horizontal_wrapped(|ui| {
                if ui.button("Load memory regions...").clicked() {
                    if let Some(path) = rfd::FileDialog::new()
                        .add_filter("Linker map", &["map"])
                        .pick_file()
                    {
                        self.apply_map(path);
                    }
                }
                if ui.button("Discover layout from matching map").clicked() {
                    self.discover_layout();
                }
                if ui.button("Use ELF inference").clicked() {
                    self.configure(None);
                }
            });
        });
        ui.separator();
        self.ensure_region_cache(a);
        if a.options.regions.is_empty() {
            ui.label("Region capacity and free space are unknown. Use Load memory regions above to import a linker map.");
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
            egui::ScrollArea::vertical()
                .id_salt("region_summary")
                .max_height(180.0)
                .show(ui, |ui| {
                    for (index, region) in a.options.regions.iter().enumerate() {
                        let usage = &self.region_cache[index];
                        let label = format!(
                            "{} ({:?})  {}–{}  |  {} used / {} free / {} total  ({})",
                            region.name,
                            region.kind,
                            self.snapshot_address("region", &region.name, "start", region.start),
                            self.snapshot_address(
                                "region",
                                &region.name,
                                "end",
                                region.start.saturating_add(region.size)
                            ),
                            self.snapshot_bytes("region", &region.name, "used", usage.used),
                            self.snapshot_bytes("region", &region.name, "free", usage.free),
                            self.snapshot_bytes("region", &region.name, "size", region.size),
                            self.snapshot_region_percentage(&region.name, usage.used, region.size)
                        );
                        ui.selectable_value(&mut self.selected_region, Some(index), label);
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
                let usage = &self.region_cache[index];
                ui.label(format!("Symbols in {}", region.name));
                ui.small("Usage includes section padding and reservations. Aliases and zero-sized labels are listed; symbol sizes do not sum to region usage. Boundary-crossing ranges count only bytes inside the region.");
                if usage.symbols.is_empty() {
                    ui.label("No symbols available in this region. Stripped firmware can still occupy space.");
                }
                let rows = self.cached_rows(a as *const Analysis as usize, || {
                    usage
                        .symbols
                        .iter()
                        .map(|entry| {
                            let s = &a.symbols[entry.symbol_index];
                            Row::new(
                                vec![
                                    s.demangled_name.clone(),
                                    self.snapshot_address(
                                        "symbol",
                                        &symbol_key(s),
                                        &super::snapshots::placement_field(
                                            &region.name,
                                            entry.placement,
                                        ),
                                        entry.address,
                                    ),
                                    self.snapshot_bytes("symbol", &symbol_key(s), "size", s.size),
                                    s.section.clone(),
                                    entry.placement.into(),
                                    s.source_file
                                        .as_ref()
                                        .or(s.compilation_unit.as_ref())
                                        .map(|path| display_path(path).into_owned())
                                        .unwrap_or_else(|| "[unattributed]".into()),
                                ],
                                &[(1, entry.address.into()), (2, s.size.into())],
                                format!(
                                    "Linker name: {}\nRuntime address: {}\nELF size: {}\n{}",
                                    s.name,
                                    self.snapshot_address(
                                        "symbol",
                                        &symbol_key(s),
                                        "normalized_address",
                                        s.normalized_address
                                    ),
                                    self.snapshot_bytes("symbol", &symbol_key(s), "size", s.size),
                                    s.attribution
                                ),
                            )
                        })
                        .collect()
                });
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
        let rows = self.cached_rows(a as *const Analysis as usize, || {
            a.memory_map
                .iter()
                .map(|r| {
                    Row::new(
                        vec![
                            r.name.clone(),
                            self.snapshot_address(
                                "range",
                                &serde_json::to_string(&(&r.name, &r.space)).unwrap(),
                                "address",
                                r.address,
                            ),
                            self.snapshot_address(
                                "range",
                                &serde_json::to_string(&(&r.name, &r.space)).unwrap(),
                                "end",
                                r.address + r.size,
                            ),
                            self.snapshot_bytes(
                                "range",
                                &serde_json::to_string(&(&r.name, &r.space)).unwrap(),
                                "size",
                                r.size,
                            ),
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
                .collect()
        });
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
        ui.collapsing("Why is total stack unknown?", |ui| {
            ui.label("The app reads compiler .su reports but does not construct a call graph. Local frames cannot establish the maximum nested call chain or interrupt overhead.");
            if let Some(report) = &self.stack {
                for call in &report.call_graph.unresolved {
                    ui.label(format!("{}: {}", call.function.as_deref().unwrap_or("Whole firmware"), call.reason.label()));
                }
                let dynamic = report.entries.iter().filter(|e| e.qualifier == "dynamic").count();
                if dynamic > 0 { ui.label(format!("{dynamic} dynamic frames have no compiler-provided upper bound.")); }
            }
            ui.label("Use .su reports from this build (-fstack-usage). Inspect missing ELF functions and ambiguous matches below; call-chain analysis and runtime stack high-water measurements are needed for total stack sizing.");
        });
        let Some(report) = &self.stack else {
            self.visible_rows = 0;
            ui.add_space(15.0);
            ui.label("Build with -fstack-usage, then select report files or directories under Stack usage in the left menu.");
            return;
        };
        if let Some(analysis) = &self.analysis {
            let matched: std::collections::HashSet<_> = report
                .entries
                .iter()
                .flat_map(|e| e.symbol_candidates.iter())
                .collect();
            let missing: Vec<_> = analysis
                .symbols
                .iter()
                .filter(|s| s.kind == "Function")
                .filter(|s| !matched.contains(&format!("{} @ {:#x}", s.name, s.address)))
                .collect();
            ui.collapsing(
                format!(
                    "{} ELF functions without stack reports (size unknown)",
                    missing.len()
                ),
                |ui| {
                    for symbol in missing {
                        ui.label(&symbol.demangled_name);
                    }
                },
            );
        }
        for warning in &report.warnings {
            if warning.starts_with("Multiple report files")
                || warning.starts_with("Ambiguous reports:")
                || warning.starts_with("Could not confidently select stack reports")
                || warning.starts_with("Skipped stack report while guessing:")
            {
                ui.colored_label(egui::Color32::YELLOW, warning);
            }
        }
        let mut files_by_symbol: std::collections::HashMap<&str, std::collections::HashSet<&str>> =
            Default::default();
        for entry in &report.entries {
            for symbol in &entry.symbol_candidates {
                files_by_symbol
                    .entry(symbol)
                    .or_default()
                    .insert(&entry.report_file);
            }
        }
        if files_by_symbol.values().any(|files| files.len() > 1) {
            ui.colored_label(egui::Color32::YELLOW, "Ambiguous reports: multiple files match the same ELF functions. Choose reports to resolve build ownership.");
        }
        let unresolved = report
            .entries
            .iter()
            .filter(|e| e.symbol_candidates.is_empty())
            .count();
        let no_matches = unresolved == report.entries.len();
        if no_matches && unresolved > 0 {
            ui.weak("No reports match symbols in this ELF. Showing all reports; verify that they belong to this build.");
        } else {
            ui.checkbox(
                &mut self.stack_show_unresolved,
                format!("Show unresolved reports ({unresolved})"),
            );
        }
        ui.weak("Expand a frame for evidence").on_hover_text("Multiple ELF candidates are ambiguous. Each expanded row includes matching evidence and its report path.");
        let build_root = self.build.as_ref().map(|build| build.root.as_path());
        let rows = self.cached_rows(report as *const _ as usize, || {
            report
                .entries
                .iter()
                .filter(|e| {
                    no_matches || self.stack_show_unresolved || !e.symbol_candidates.is_empty()
                })
                .map(|e| {
                    Row::new(
                        vec![
                            e.function.clone(),
                            self.snapshot_bytes(
                                "stack",
                                &stack_key(e),
                                "local_bytes",
                                e.local_bytes,
                            ),
                            e.qualifier.clone(),
                            format!(
                                "{}:{}",
                                build_relative_path(&e.source_file, build_root),
                                e.source_line
                            ),
                        ],
                        &[(1, e.local_bytes.into())],
                        format!(
                            "{}\n{}\nELF matches: {:?}",
                            build_relative_path(&e.report_file, build_root),
                            e.evidence,
                            e.symbol_candidates
                        ),
                    )
                    .with_bars(&[1])
                })
                .collect()
        });
        self.table(ui, &[("Function","Compiler function label"),("Local frame","Compiler reported bytes, not a call-chain estimate. Gray bars compare each frame with the largest visible frame (100%)."),("Qualifier","static: fixed frame; dynamic,bounded: compiler bound; dynamic: total may be unbounded"),("Source","Location reported by the compiler")], rows);
    }
    pub(super) fn compare_view(&mut self, ui: &mut egui::Ui) {
        let Some(c) = &self.comparison else {
            self.visible_rows = 0;
            ui.weak("Select a snapshot baseline to compare against the current firmware.");
            if ui.button("Select baseline...").clicked() {
                self.open_snapshot_manager();
            }
            return;
        };
        ui.horizontal_wrapped(|ui| {
            if let Some(name) = self.snapshot_label() {
                ui.label(format!("Baseline: {name}"));
                ui.separator();
            }
            ui.label(format!("Flash {:+} B", c.flash_delta))
                .on_hover_text(format!("{} to {}", bytes(c.old.flash), bytes(c.new.flash)));
            ui.separator();
            ui.label(format!("RAM {:+} B", c.ram_delta))
                .on_hover_text(format!("{} to {}", bytes(c.old.ram), bytes(c.new.ram)));
            ui.separator();
            ui.weak("Current minus baseline").on_hover_text(format!(
                "Baseline: {}\nCurrent: {}",
                display_path(&c.old_path),
                display_path(&c.new_path)
            ));
        });
        let changes = match self.comparison_group {
            1 => &c.symbols,
            2 => &c.sections,
            _ => &c.files,
        };
        let rows = self.cached_rows(c as *const _ as usize, || {
            changes
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
                .collect()
        });
        if ui.button("Change baseline...").clicked() {
            self.open_snapshot_manager();
        }
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
        ThreadLocal => "TLS template (per thread)",
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
fn tree(
    ui: &mut egui::Ui,
    node: &FileTree,
    filter: &str,
    id: &str,
    parent: &str,
    selected: &mut Option<String>,
    app: &Explorer,
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
        display_path(&node.name),
        app.snapshot_bytes("tree", &path, "flash", node.usage.flash),
        app.snapshot_bytes("tree", &path, "ram", node.usage.ram)
    );
    let help = format!(
        "{}\nFlash: {} B\nRAM: {} B",
        display_path(&node.name),
        node.usage.flash,
        node.usage.ram
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
                        app,
                    );
                }
            })
            .header_response
            .on_hover_text(help);
    }
}

#[cfg(test)]
mod cache_tests {
    use super::*;

    fn rows() -> Vec<Row> {
        [("Alpha", 100), ("Beta", 9), ("beta", 9)]
            .into_iter()
            .map(|(name, value)| {
                Row::new(
                    vec![name.into(), value.to_string()],
                    &[(1, value)],
                    name.into(),
                )
                .with_bars(&[1])
            })
            .collect()
    }

    #[test]
    fn repaints_reuse_rows_and_order_while_search_and_sort_preserve_scales_and_ties() {
        let mut app = Explorer::default();
        let source = app.cached_rows(1, rows);
        let prepared = app.prepare_table(source.clone(), 2);
        assert_eq!(prepared.indices, [0, 1, 2]);
        assert_eq!(prepared.bar_maxima[1], 100);
        let same_rows = app.cached_rows(1, || panic!("repaint rebuilt rows"));
        assert!(Rc::ptr_eq(&source, &same_rows));
        assert!(Rc::ptr_eq(&prepared, &app.prepare_table(same_rows, 2)));
        app.search = "BETA".into();
        let filtered = app.prepare_table(source.clone(), 2);
        assert_eq!(filtered.indices, [1, 2]);
        assert_eq!(filtered.bar_maxima[1], 9);
        app.descending = false;
        let sorted = app.prepare_table(source.clone(), 2);
        assert_eq!(sorted.indices, [1, 2]);
        assert_eq!(sorted.bar_maxima[1], 9);
        assert!(!Rc::ptr_eq(&filtered, &sorted));
        app.search.clear();
        assert_eq!(app.prepare_table(source, 2).indices, [1, 2, 0]);
    }

    #[test]
    fn modes_selections_sources_and_report_revision_invalidate_rows() {
        let mut app = Explorer::default();
        let mut previous = app.cached_rows(1, rows);
        let changes: [fn(&mut Explorer); 8] = [
            |a| a.view = View::Symbols,
            |a| a.kind_filter = "Function".into(),
            |a| a.selected_file = Some("src/main.c".into()),
            |a| a.selected_region = Some(1),
            |a| a.stack_show_unresolved = true,
            |a| a.comparison_group = 1,
            |a| a.report_revision += 1,
            |a| a.view = View::Sections,
        ];
        for change in changes {
            change(&mut app);
            let next = app.cached_rows(1, rows);
            assert!(!Rc::ptr_eq(&previous, &next));
            previous = next;
        }
        assert!(!Rc::ptr_eq(&previous, &app.cached_rows(2, rows)));
    }
}
