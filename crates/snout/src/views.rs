use super::display::{build_relative_path, display_path};
use super::snapshots::{stack_key, symbol_key};
use super::{Explorer, View};
use eframe::egui::{self, RichText};
use egui_extras::{Column, TableBuilder};
use snout_core::{format_bytes as bytes, Analysis, FileTree};
use std::{
    cell::{OnceCell, RefCell},
    rc::Rc,
};

pub const ACCENT: egui::Color32 = egui::Color32::from_rgb(82, 224, 164);
pub(super) const TEXT_SELECTION: egui::Color32 = egui::Color32::from_rgb(48, 105, 163);
pub(super) const MEMORY_BAR: egui::Color32 = egui::Color32::from_rgba_premultiplied(23, 23, 23, 45);
pub(super) const FLASH_HELP: &str = "Allocated bytes stored in the load image. Initialized RAM data also needs initial values in Flash. Gaps and programmer-specific overhead are excluded.";
pub(super) const RAM_HELP: &str = "Static memory required while running. Includes initialized data, zero-filled storage and explicit reservations. Additional heap and stack demand is not automatically known.";
struct Row {
    cells: Vec<String>,
    search_cells: Vec<String>,
    cell_tips: std::collections::HashMap<usize, String>,
    source_paths: Option<Rc<super::display::SourcePaths>>,
    values: Vec<Option<i128>>,
    tip: OnceCell<String>,
    tip_builder: Option<Box<dyn Fn() -> String>>,
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
    show_address_changes: bool,
    build_root: Option<std::path::PathBuf>,
}

type CachedRows = RefCell<Option<(RowKey, Rc<Vec<Row>>)>>;

#[derive(Default)]
pub(super) struct TableCache {
    rows: [CachedRows; View::ALL.len()],
    prepared: [Option<Rc<PreparedTable>>; View::ALL.len()],
    sizing: [Option<ColumnSizing>; View::ALL.len()],
}

struct ColumnSizing {
    source: Rc<Vec<Row>>,
    headers: Vec<String>,
    fonts: [egui::FontId; 3],
    padding: egui::Vec2,
    pixels_per_point: f32,
    desired: Vec<f32>,
    applied_width: Option<f32>,
}

// Grow all columns fairly until their content fits or the viewport is full.
// Keep usable minimum widths when horizontal scrolling is necessary.
fn fit_columns(desired: &[f32], available: f32) -> Vec<f32> {
    let mut widths: Vec<_> = (0..desired.len())
        .map(|i| {
            if i == 0 {
                120.0
            } else if i == desired.len() - 1 {
                100.0
            } else {
                65.0
            }
        })
        .collect();
    let mut remaining = (available - widths.iter().sum::<f32>()).max(0.0);
    loop {
        let growing: Vec<_> = (0..widths.len())
            .filter(|&i| desired[i] - widths[i] > 0.5)
            .collect();
        if growing.is_empty() || remaining <= 0.5 {
            break;
        }
        let share = remaining / growing.len() as f32;
        let mut used = 0.0;
        for i in growing {
            let extra = (desired[i] - widths[i]).min(share);
            widths[i] += extra;
            used += extra;
        }
        remaining -= used;
    }
    if let Some(last) = widths.last_mut() {
        *last += remaining;
    }
    widths
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
            cell_tips: Default::default(),
            source_paths: None,
            values,
            tip: OnceCell::from(tip),
            tip_builder: None,
            action: None,
            bar_columns: vec![],
        }
    }

    fn with_source_paths(mut self, paths: Rc<super::display::SourcePaths>) -> Self {
        self.source_paths = Some(paths);
        self
    }

    fn detail_text(&self) -> String {
        self.source_paths
            .as_ref()
            .map_or_else(|| self.tip().clone(), |p| p.short_detail(self.tip()))
    }

    fn with_search(mut self, text: &str) -> Self {
        self.search_cells.push(text.to_lowercase());
        self
    }

    fn with_path(mut self, column: usize, full: String) -> Self {
        self.search_cells.push(full.to_lowercase());
        self.cell_tips.insert(column, full);
        self
    }

    fn with_lazy_tip(mut self, build: impl Fn() -> String + 'static) -> Self {
        self.tip = OnceCell::new();
        self.tip_builder = Some(Box::new(build));
        self
    }

    fn tip(&self) -> &String {
        self.tip
            .get_or_init(|| self.tip_builder.as_ref().unwrap()())
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
            show_address_changes: self.show_address_changes[self.view as usize],
            build_root: self.build.as_ref().map(|b| b.root.clone()),
        };
        let mut cache = self.table_cache.rows[self.view as usize].borrow_mut();
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
        if let Some(cached) = &self.table_cache.prepared[self.view as usize] {
            if Rc::ptr_eq(&cached.source, &rows)
                && cached.search == search
                && cached.sort == sort
                && cached.descending == self.descending
            {
                return cached.clone();
            }
        }
        let filtered = self.table_cache.prepared[self.view as usize]
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
        self.table_cache.prepared[self.view as usize] = Some(prepared.clone());
        prepared
    }

    fn column_widths(
        &mut self,
        ui: &egui::Ui,
        headers: &[(&str, &str)],
        source: Rc<Vec<Row>>,
        available: f32,
    ) -> (Vec<f32>, bool) {
        let fonts = [
            egui::TextStyle::Body.resolve(ui.style()),
            egui::TextStyle::Monospace.resolve(ui.style()),
            egui::TextStyle::Button.resolve(ui.style()),
        ];
        let padding = ui.spacing().button_padding;
        let pixels_per_point = ui.ctx().pixels_per_point();
        let slot = &mut self.table_cache.sizing[self.view as usize];
        let valid = slot.as_ref().is_some_and(|s| {
            Rc::ptr_eq(&s.source, &source)
                && s.fonts == fonts
                && s.padding == padding
                && s.pixels_per_point == pixels_per_point
                && s.headers.len() == headers.len()
                && s.headers.iter().zip(headers).all(|(a, (b, _))| a == b)
        });
        if !valid {
            let desired = ui.fonts(|f| {
                let mono_advance =
                    (f.glyph_width(&fonts[1], 'M') * pixels_per_point).round() / pixels_per_point;
                headers
                    .iter()
                    .enumerate()
                    .map(|(i, (title, _))| {
                        // Reserve the sort arrow on every header so sorting is stable.
                        let header = f
                            .layout_no_wrap((*title).into(), fonts[2].clone(), egui::Color32::WHITE)
                            .size()
                            .x
                            + padding.x * 2.0
                            + 16.0;
                        let mut maximum = header;
                        let mut measured = std::collections::HashSet::new();
                        for row in source.iter() {
                            let Some(cell) = row.cells.get(i) else {
                                continue;
                            };
                            let mono = row.values[i].is_some()
                                || i == 0
                                || (self.view == View::Stack && i == 3);
                            if measured.insert((mono, cell.as_str())) {
                                // ASCII monospace cells dominate large symbol tables.
                                // Measure their glyph advances without laying out thousands
                                // of invisible rows. Other text uses the renderer's layout.
                                let width =
                                    if mono && cell.is_ascii() && !cell.contains(['\t', '\r']) {
                                        cell.split('\n')
                                            .map(|line| line.len() as f32 * mono_advance)
                                            .fold(0.0_f32, f32::max)
                                    } else {
                                        let font = &fonts[usize::from(mono)];
                                        f.layout_no_wrap(
                                            cell.clone(),
                                            font.clone(),
                                            egui::Color32::WHITE,
                                        )
                                        .size()
                                        .x
                                    };
                                maximum = maximum.max(width + 8.0);
                            }
                        }
                        maximum.ceil()
                    })
                    .collect()
            });
            *slot = Some(ColumnSizing {
                source,
                headers: headers.iter().map(|(title, _)| (*title).into()).collect(),
                fonts,
                padding,
                pixels_per_point,
                desired,
                applied_width: None,
            });
        }
        let sizing = slot.as_mut().unwrap();
        let reset = sizing
            .applied_width
            .is_none_or(|old| (old - available).abs() > 0.5);
        if reset {
            sizing.applied_width = Some(available);
        }
        (fit_columns(&sizing.desired, available), reset)
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
            ui.weak(if self.diffs_active() && self.search.is_empty() {
                "No differences for the current selection."
            } else if self.search.is_empty() {
                "No entries for the current selection."
            } else {
                "No entries match the filter. Clear it to see available entries."
            });
        }
        let editing_text = ui
            .memory(|m| m.focused())
            .is_some_and(|id| egui::TextEdit::load_state(ui.ctx(), id).is_some());
        let navigate = if self.snapshot_dialog.is_none()
            && self.map_warning.is_none()
            && ui.memory(|m| m.allows_interaction(ui.layer_id()))
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
                    .is_some_and(|(name, tip)| name == &item.cells[0] && tip == item.tip())
            });
            let next = match current {
                None => 0,
                Some(index) if down => (index + 1).min(rows.len() - 1),
                Some(index) => index.saturating_sub(1),
            };
            self.details = Some((rows[next].cells[0].clone(), rows[next].tip().clone()));
            scroll_to = Some(next);
        }
        let mut clicked = None;
        let height = ui.available_height();
        let viewport_width = ui.available_width();
        let spacing = (headers.len() - 1) as f32 * ui.spacing().item_spacing.x;
        let scrollbar = ui.spacing().scroll.allocated_width();
        let (column_widths, reset_widths) = self.column_widths(
            ui,
            headers,
            prepared.source.clone(),
            viewport_width - spacing - scrollbar,
        );
        let minimum_width = fit_columns(&vec![0.0; headers.len()], 0.0)
            .iter()
            .sum::<f32>()
            + spacing
            + scrollbar;
        egui::ScrollArea::horizontal()
            .id_salt("table_horizontal")
            .show(ui, |ui| {
                ui.set_min_width(minimum_width.max(viewport_width));
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
                if reset_widths {
                    table.reset();
                }
                for (index, &width) in column_widths.iter().enumerate() {
                    let minimum = if index == 0 {
                        120.0
                    } else if index == headers.len() - 1 {
                        100.0
                    } else {
                        65.0
                    };
                    table = table.column(Column::initial(width).at_least(minimum).clip(true));
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
                                .position(|item| name == &item.cells[0] && tip == item.tip())
                        });
                        let detail_text = expanded.map(|index| {
                            let ui = body.ui_mut();
                            ui.painter().layout(
                                rows[index].detail_text(),
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
                                            response.on_hover_text(
                                                item.cell_tips.get(&index).unwrap_or(cell),
                                            );
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
                                    Some((item.cells[0].clone(), item.tip().clone()))
                                };
                            }
                            if let Some((ui, background, clip)) = detail_ui {
                                expanded_detail = Some((
                                    ui,
                                    background,
                                    clip,
                                    detail_text.clone(),
                                    item.action.clone(),
                                    item.tip().clone(),
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
                if let Some((mut ui, background, clip, text, action, full_detail)) = expanded_detail
                {
                    ui.painter().with_clip_rect(clip).rect_filled(
                        background,
                        0.0,
                        ui.visuals().selection.bg_fill,
                    );
                    ui.visuals_mut().selection.bg_fill = TEXT_SELECTION;
                    if let Some(text) = text {
                        ui.add(egui::Label::new(text).selectable(true))
                            .on_hover_text(full_detail);
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
        let display = self.baseline_display_analysis();
        let a = display.as_deref().unwrap_or(a);
        let mut selected = None;
        egui::ScrollArea::both().show(ui, |ui| {
            let paths = self.source_paths(a);
            tree(
                ui,
                &a.tree,
                "",
                "project",
                "",
                &mut selected,
                &TreeDisplay {
                    app: self,
                    paths: &paths,
                },
            );
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
        let display = self.baseline_display_analysis();
        let a = display.as_deref().unwrap_or(a);
        let paths = self.source_paths(a);
        let rows = self.cached_rows(a as *const Analysis as usize, || {
            a.files
                .iter()
                .filter(|f| self.diff_visible("file", &f.path))
                .map(|f| {
                    let mut row = Row::new(
                        vec![
                            paths.short(&f.path),
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
                            paths.full(&f.path),
                            f.attribution,
                            self.snapshot_bytes("file", &f.path, "usage.flash", f.usage.flash),
                            self.snapshot_bytes("file", &f.path, "usage.ram", f.usage.ram)
                        ),
                    )
                    .with_source_paths(paths.clone())
                    .with_path(0, paths.full(&f.path))
                    .with_search(&f.path)
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
        let display = self.baseline_display_analysis();
        let a = display.as_deref().unwrap_or(a);
        let paths = self.source_paths(a);
        if let Some(file) = &self.selected_file {
            ui.weak(paths.short(file)).on_hover_text(paths.full(file));
        }
        let comparing = self.snapshot_label().is_some();
        let rows = self.cached_rows(a as *const Analysis as usize, || {
            // Keep symbol details in the existing shared report and format them
            // only when a row is opened. Standalone renderers may supply a borrow.
            let source = display.clone().or_else(|| self.analysis.clone())
                .filter(|source| std::ptr::eq(source.as_ref(), a))
                .unwrap_or_else(|| std::sync::Arc::new(a.clone()));
            a.symbols
                .iter()
                .enumerate()
                .filter(|(_, s)| self.diff_visible("symbol", &symbol_key(s)))
                .filter(|(_, s)| self.kind_filter == "All" || s.kind == self.kind_filter)
                .filter(|(_, s)| {
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
                .map(|(index, s)| {
                    // Identity serialization and snapshot lookups are only needed
                    // when a baseline is selected. Reuse the identity for all fields.
                    let id = if comparing { symbol_key(s) } else { String::new() };
                    let symbol_bytes = |field, value| {
                        if comparing { self.snapshot_bytes("symbol", &id, field, value) } else { bytes(value) }
                    };
                    let symbol_address = |field, value| {
                        if comparing { self.snapshot_address("symbol", &id, field, value) } else { format!("{value:#010x}") }
                    };
                    Row::new(
                        vec![
                            s.demangled_name.clone(),
                            symbol_bytes("size", s.size),
                            symbol_bytes("usage.flash", s.usage.flash),
                            symbol_bytes("usage.ram", s.usage.ram),
                            s.kind.clone(),
                            s.section.clone(),
                            symbol_address("address", s.address),
                        ],
                        &[
                            (1, s.size.into()),
                            (2, s.usage.flash.into()),
                            (3, s.usage.ram.into()),
                            (6, s.address.into()),
                        ],
                        String::new(),
                    )
                    .with_search(&s.name)
                    .with_source_paths(paths.clone())
                    .with_lazy_tip({
                        let source = source.clone();
                        let paths = paths.clone();
                        let address = symbol_address("address", s.address);
                        let normalized = (s.address != s.normalized_address)
                            .then(|| symbol_address("normalized_address", s.normalized_address));
                        move || {
                            let s = &source.symbols[index];
                            format!(
                                "Address: {}\nSection: {} (index {})\n{}{}Weak symbol: {}\nSource: {}:{}\nCompilation unit: {}\n{}",
                                address,
                                s.section,
                                s.section_index,
                                if s.name != s.demangled_name {
                                    format!("Linker name: {}\n", s.name)
                                } else {
                                    String::new()
                                },
                                if s.address != s.normalized_address {
                                    format!("Normalized address: {}\n", normalized.as_deref().unwrap())
                                } else {
                                    String::new()
                                },
                                s.weak,
                                paths.full(s.source_file.as_deref().unwrap_or("Unknown")),
                                s.source_line
                                    .map(|l| l.to_string())
                                    .unwrap_or_else(|| "?".into()),
                                paths.full(s.dwarf_compilation_unit.as_deref().or(s.compilation_unit.as_deref()).unwrap_or("Unknown")),
                                s.attribution
                            )
                        }
                    })
                    .with_bars(&[1, 2, 3])
                })
                .collect()
        });
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
        let display = self.baseline_display_analysis();
        let a = display.as_deref().unwrap_or(a);
        if let Some(tls) = &a.tls {
            let reveal = std::mem::take(&mut self.reveal_tls_details);
            egui::CollapsingHeader::new("Thread-local storage (TLS)")
                .default_open(true)
                .open(reveal.then_some(true))
                .show(ui, |ui| {
                    let mut scroll = egui::ScrollArea::vertical()
                        .id_salt("tls_details")
                        .max_height(160.0);
                    if reveal {
                        scroll = scroll.vertical_scroll_offset(0.0);
                    }
                    scroll.show(ui, |ui| {
                            ui.label(format!(
                                "Template per thread: {} — {} initialized, {} zero-initialized; alignment {}",
                                self.snapshot_bytes("tls", "", "template_size", tls.template_size),
                                self.snapshot_bytes("tls", "", "initialized_size", tls.initialized_size),
                                self.snapshot_bytes("tls", "", "zero_initialized_size", tls.zero_initialized_size),
                                self.snapshot_bytes("tls", "", "alignment", tls.alignment),
                            ));
                            ui.label("Total TLS RAM is unknown. Static RAM excludes TLS templates; allocation may be inside existing stack reservations.");
                            ui.small("Variable offsets are within the per-thread template, not physical runtime addresses.");
                            for symbol in &tls.symbols {
                                if !self.diff_visible("tls_symbol", &symbol.name) {
                                    continue;
                                }
                                ui.monospace(format!(
                                    "{}  {}  {} [{}]",
                                    self.snapshot_address("tls_symbol", &symbol.name, "offset", symbol.offset),
                                    self.snapshot_bytes("tls_symbol", &symbol.name, "size", symbol.size),
                                    symbol.name,
                                    symbol.section,
                                ));
                            }
                        });
                });
            ui.separator();
        }
        let rows = self.cached_rows(a as *const Analysis as usize, || { a.sections.iter().filter(|s| self.diff_visible("section", &s.name)).map(|s| Row::new(vec![s.name.clone(), self.snapshot_bytes("section", &s.name, "size", s.size), self.snapshot_bytes("section", &s.name, "usage.flash", s.usage.flash), self.snapshot_bytes("section", &s.name, "usage.ram", s.usage.ram), self.snapshot_address("section", &s.name, "address", s.address), s.load_address.map(|v| self.snapshot_address("section", &s.name, "load_address", v)).unwrap_or_else(|| load_address(None, s.load_size)), classification(s.classification).into()], &[(1,s.size.into()),(2,s.usage.flash.into()),(3,s.usage.ram.into()),(4,s.address.into()),(5,s.load_address.unwrap_or(0).into())],
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
        let display = self.baseline_display_analysis();
        let a = display.as_deref().unwrap_or(a);
        self.ensure_region_cache(a);
        if a.options.regions.is_empty() {
            self.map_selection_notice(ui);
        } else {
            ui.small("Physical occupancy includes section padding and reservations. Free space may be needed by runtime heap and stack.");
            self.visible_rows = 0;
            egui::ScrollArea::vertical()
                .id_salt("region_summary")
                .show(ui, |ui| {
                    for (index, region) in a.options.regions.iter().enumerate() {
                        if !self.diff_visible("region", &region.name)
                            || !region
                                .name
                                .to_lowercase()
                                .contains(&self.search.to_lowercase())
                        {
                            continue;
                        }
                        self.visible_rows += 1;
                        let usage = &self.region_cache[index];
                        ui.label(
                            egui::RichText::new(format!("{} ({:?})", region.name, region.kind))
                                .strong(),
                        );
                        ui.small(format!(
                            "{}–{}",
                            self.snapshot_address("region", &region.name, "start", region.start),
                            self.snapshot_address(
                                "region",
                                &region.name,
                                "end",
                                region.start.saturating_add(region.size)
                            )
                        ));
                        super::overview::occupancy_bar(ui, usage.used, region.size, region.kind);
                        ui.label(format!(
                            "{} used / {} total · {} · {} free",
                            self.snapshot_bytes("region", &region.name, "used", usage.used),
                            self.snapshot_bytes("region", &region.name, "size", region.size),
                            self.snapshot_region_percentage(&region.name, usage.used, region.size),
                            self.snapshot_bytes("region", &region.name, "free", usage.free)
                        ));
                        ui.separator();
                    }
                    if self.visible_rows == 0 {
                        ui.weak("No matching regions.");
                    }
                });
            return;
        }
        ui.small("Load and runtime are separate address spaces").on_hover_text("Do not add load and runtime ranges together; the same storage can appear in both views.");
        let rows = self.cached_rows(a as *const Analysis as usize, || {
            a.memory_map
                .iter()
                .filter(|r| {
                    self.diff_visible(
                        "range",
                        &serde_json::to_string(&(&r.name, &r.space)).unwrap(),
                    )
                })
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
        let display_stack = self.baseline_display.as_ref().and_then(|d| d.stack.clone());
        let Some(report) = display_stack.as_deref().or(self.stack.as_ref()) else {
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
            .filter(|e| {
                !self.snapshot_removed("stack", &stack_key(e)) && e.symbol_candidates.is_empty()
            })
            .count();
        let no_matches = unresolved == self.stack.as_ref().map_or(0, |s| s.entries.len());
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
        let display = self.baseline_display_analysis();
        let analysis = display.as_deref().or(self.analysis.as_deref());
        let paths = analysis.map(|a| self.source_paths(a));
        let rows = self.cached_rows(report as *const _ as usize, || {
            report
                .entries
                .iter()
                .filter(|e| self.diff_visible("stack", &stack_key(e)))
                .filter(|e| {
                    self.snapshot_removed("stack", &stack_key(e))
                        || no_matches
                        || self.stack_show_unresolved
                        || !e.symbol_candidates.is_empty()
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
                                paths.as_ref().map_or_else(
                                    || super::display::short_path(
                                        &e.source_file,
                                        report.entries.iter().map(|e| e.source_file.as_str())
                                    ),
                                    |p| p.short(&e.source_file)
                                ),
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
                    .with_path(
                        3,
                        format!(
                            "{}:{}",
                            paths.as_ref().map_or_else(
                                || super::display::absolute_path(&e.source_file, build_root),
                                |p| p.full(&e.source_file)
                            ),
                            e.source_line
                        ),
                    )
                    .with_search(&format!("{}:{}", e.source_file, e.source_line))
                    .with_bars(&[1])
                })
                .collect()
        });
        self.table(ui, &[("Function","Compiler function label"),("Local frame","Compiler reported bytes, not a call-chain estimate. Gray bars compare each frame with the largest visible frame (100%)."),("Qualifier","static: fixed frame; dynamic,bounded: compiler bound; dynamic: total may be unbounded"),("Source","Location reported by the compiler")], rows);
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
fn classification(value: snout_core::Classification) -> &'static str {
    use snout_core::Classification::*;
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
struct TreeDisplay<'a> {
    app: &'a Explorer,
    paths: &'a super::display::SourcePaths,
}

fn tree(
    ui: &mut egui::Ui,
    node: &FileTree,
    filter: &str,
    id: &str,
    parent: &str,
    selected: &mut Option<String>,
    display: &TreeDisplay<'_>,
) {
    let TreeDisplay { app, paths } = display;
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
    if !app.diff_visible("tree", &path) {
        return;
    }
    let label = format!(
        "{}  {} / {}",
        display_path(&node.name),
        app.snapshot_bytes("tree", &path, "flash", node.usage.flash),
        app.snapshot_bytes("tree", &path, "ram", node.usage.ram)
    );
    let help = format!(
        "{}\nFlash: {} B\nRAM: {} B",
        paths.tree_full(&path),
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
                        display,
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

    #[test]
    fn growth_link_search_finds_mangled_symbols_without_formatting_details() {
        let mut analysis = snout_core::analyze_bytes(
            include_bytes!("../../../fixtures/build/gcc/cortex-m.elf"),
            "fixture.elf",
            &Default::default(),
        )
        .unwrap();
        let mut symbol = analysis.symbols[0].clone();
        symbol.name = "_ZN6driver4pollEv".into();
        symbol.demangled_name = "driver::poll()".into();
        analysis.symbols = vec![symbol];
        let analysis = std::sync::Arc::new(analysis);
        let mut app = Explorer {
            analysis: Some(analysis.clone()),
            view: View::Symbols,
            search: "_ZN6driver4pollEv".into(),
            ..Default::default()
        };
        let ctx = egui::Context::default();
        let _ = ctx.run(Default::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| app.symbols(ui, &analysis));
        });
        assert_eq!(app.visible_rows, 1);
        let cache = app.table_cache.rows[View::Symbols as usize].borrow();
        let rows = &cache.as_ref().unwrap().1;
        assert!(rows[0].tip.get().is_none(), "Search must keep details lazy");
    }

    #[test]
    fn fitting_uses_spare_width_without_forcing_wide_content_to_scroll() {
        let desired = [420.0, 210.0, 160.0, 280.0];
        let wide = fit_columns(&desired, 1400.0);
        assert!(wide
            .iter()
            .zip(desired)
            .all(|(actual, needed)| *actual >= needed));
        assert!((wide.iter().sum::<f32>() - 1400.0).abs() < 0.01);
        let balanced = [130.0, 70.0, 80.0, 450.0];
        let fitted = fit_columns(&balanced, 800.0);
        assert!(fitted
            .iter()
            .zip(balanced)
            .all(|(actual, needed)| *actual >= needed));
        let compact = fit_columns(&desired, 700.0);
        assert!((compact.iter().sum::<f32>() - 700.0).abs() < 0.01);
        assert!(compact
            .iter()
            .zip(desired)
            .any(|(actual, needed)| *actual < needed));
        assert_eq!(fit_columns(&desired, 300.0), [120.0, 65.0, 65.0, 100.0]);
    }

    #[test]
    fn wide_tables_show_full_names_and_snapshot_deltas_without_resizing() {
        let cells: Vec<String> = [
            "namespace::module::a_reasonably_long_function_name()",
            "128.00 KiB (+64.00 KiB; removed)",
            "64.00 KiB (+32.00 KiB)",
            "src/drivers/a_long_source_file_name.c",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        let source = Rc::new(vec![Row::new(
            cells.clone(),
            &[(1, 131072), (2, 65536)],
            String::new(),
        )]);
        let mut app = Explorer {
            view: View::Symbols,
            ..Default::default()
        };
        let ctx = egui::Context::default();
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1800.0, 500.0),
                )),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    app.table(
                        ui,
                        &[("Symbol", ""), ("Flash", ""), ("RAM", ""), ("Source", "")],
                        source.clone(),
                    );
                });
            },
        );
        for cell in &cells {
            let text = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text() == cell => Some(text),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("Missing cell {cell}"));
            assert!(!text.galley.elided, "Autofit clipped {cell}");
        }
    }

    #[test]
    fn sizing_measures_offscreen_rows_and_keeps_widths_during_search_and_sort() {
        let mut source_rows = (0..200)
            .map(|_| Row::new(vec!["short".into(), "1 B".into()], &[(1, 1)], String::new()))
            .collect::<Vec<_>>();
        source_rows.push(Row::new(
            vec![
                "a_very_long_name_that_only_appears_below_the_visible_rows".into(),
                "1.00 MiB (+512.00 KiB)".into(),
            ],
            &[(1, 1048576)],
            String::new(),
        ));
        let source = Rc::new(source_rows);
        let mut app = Explorer::default();
        let ctx = egui::Context::default();
        let _ = ctx.run(Default::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let headers = [("Name", ""), ("Size", "")];
                let (first, reset) = app.column_widths(ui, &headers, source.clone(), 1200.0);
                assert!(reset);
                assert!(first[0] > 250.0);
                let measurements = app.table_cache.sizing[app.view as usize]
                    .as_ref()
                    .unwrap()
                    .desired
                    .as_ptr();
                app.search = "short".into();
                app.descending = !app.descending;
                let (same, reset) = app.column_widths(ui, &headers, source.clone(), 1200.0);
                assert!(!reset, "Preserve manual adjustments between sizing changes");
                assert_eq!(first, same);
                assert_eq!(
                    measurements,
                    app.table_cache.sizing[app.view as usize]
                        .as_ref()
                        .unwrap()
                        .desired
                        .as_ptr()
                );
                let (_, reset) = app.column_widths(ui, &headers, source.clone(), 1500.0);
                assert!(reset, "Refit when the window gains space");
            });
        });
    }

    #[test]
    fn switching_tabs_retains_rows_and_prepared_order() {
        let mut app = Explorer {
            view: View::Symbols,
            ..Default::default()
        };
        let symbols = app.cached_rows(1, rows);
        let ordered = app.prepare_table(symbols.clone(), 2);
        app.change_view(View::Sections);
        let sections = app.cached_rows(1, rows);
        app.prepare_table(sections, 2);
        app.change_view(View::Symbols);
        let reused = app.cached_rows(1, || panic!("switching tabs rebuilt symbols"));
        assert!(Rc::ptr_eq(&symbols, &reused));
        assert!(Rc::ptr_eq(&ordered, &app.prepare_table(reused, 2)));
        app.report_revision += 1;
        assert!(!Rc::ptr_eq(&symbols, &app.cached_rows(1, rows)));
    }

    #[test]
    fn file_rows_keep_absolute_hover_search_and_selection_with_short_labels() {
        let mut a = snout_core::analyze_bytes(
            include_bytes!("../../../fixtures/build/gcc/cortex-m.elf"),
            "fixture.elf",
            &Default::default(),
        )
        .unwrap();
        a.files = ["/project/app/src/main.c", "/project/lib/src/main.c"]
            .into_iter()
            .map(|path| snout_core::FileUsage {
                path: path.into(),
                attribution: "DWARF".into(),
                usage: Default::default(),
                symbol_count: 1,
            })
            .collect();
        a.symbols.clear();
        a.dependencies.nodes.clear();
        let a = std::sync::Arc::new(a);
        let mut app = Explorer {
            view: View::Files,
            analysis: Some(a.clone()),
            ..Default::default()
        };
        let ctx = egui::Context::default();
        let _ = ctx.run(Default::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| app.files(ui, &a));
        });
        let rows = app.table_cache.rows[View::Files as usize]
            .borrow()
            .as_ref()
            .unwrap()
            .1
            .clone();
        assert_eq!(rows[0].cells[0], "app/src/main.c");
        assert_eq!(rows[1].cells[0], "lib/src/main.c");
        assert_eq!(rows[0].cell_tips[&0], "/project/app/src/main.c");
        assert_eq!(rows[0].action.as_deref(), Some("/project/app/src/main.c"));
        assert!(rows[0].detail_text().starts_with("app/src/main.c\n"));
        app.search = "/project/app".into();
        let filtered = app.prepare_table(rows, 5);
        assert_eq!(filtered.indices, [0]);
        assert_eq!(filtered.source[0].cells[0], "app/src/main.c");
    }

    #[test]
    fn file_search_keeps_recorded_paths_after_resolution() {
        let mut a = snout_core::analyze_bytes(
            include_bytes!("../../../fixtures/build/gcc/cortex-m.elf"),
            "fixture.elf",
            &Default::default(),
        )
        .unwrap();
        let recorded = "/project/build/../src/main.c";
        a.symbols.truncate(1);
        a.symbols[0].source_file = Some(recorded.into());
        a.symbols[0].dwarf_compilation_unit = None;
        a.symbols[0].compilation_unit = None;
        a.files = vec![snout_core::FileUsage {
            path: recorded.into(),
            attribution: "DWARF".into(),
            usage: Default::default(),
            symbol_count: 1,
        }];
        a.dependencies.nodes.clear();
        let old = a.clone();
        a.files[0].usage.flash = 1;
        let a = std::sync::Arc::new(a);
        let mut app = Explorer {
            analysis: Some(a.clone()),
            comparison: Some(super::super::compare(&old, &a)),
            ..Default::default()
        };
        let ctx = egui::Context::default();
        app.change_view(View::Files);
        let _ = ctx.run(Default::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| app.files(ui, &a));
        });
        let rows = app.table_cache.rows[View::Files as usize]
            .borrow()
            .as_ref()
            .unwrap()
            .1
            .clone();
        assert_eq!(rows[0].cells[0], "src/main.c");
        assert_eq!(rows[0].cell_tips[&0], "/project/src/main.c");
        // Growth-summary navigation searches by the recorded identity.
        for query in [recorded, "/project/src/main.c"] {
            app.search = query.into();
            assert_eq!(app.prepare_table(rows.clone(), 5).indices, [0]);
        }
    }

    #[test]
    fn symbol_details_are_formatted_only_when_opened() {
        let analysis = std::sync::Arc::new(
            snout_core::analyze_bytes(
                include_bytes!("../../../fixtures/build/gcc/cortex-m.elf"),
                "fixture.elf",
                &Default::default(),
            )
            .unwrap(),
        );
        let mut app = Explorer {
            view: View::Symbols,
            analysis: Some(analysis.clone()),
            ..Default::default()
        };
        let ctx = egui::Context::default();
        let _ = ctx.run(Default::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| app.symbols(ui, &analysis));
        });
        let cache = app.table_cache.rows[View::Symbols as usize].borrow();
        let rows = &cache.as_ref().unwrap().1;
        assert!(!rows.is_empty());
        assert!(rows.iter().all(|row| row.tip.get().is_none()));
        assert!(rows[0].tip().contains("Source:"));
        assert!(rows[0].tip().contains("Address:"));
        assert!(rows.iter().skip(1).all(|row| row.tip.get().is_none()));
    }

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
        let changes: [fn(&mut Explorer); 7] = [
            |a| a.view = View::Symbols,
            |a| a.kind_filter = "Function".into(),
            |a| a.selected_file = Some("src/main.c".into()),
            |a| a.selected_region = Some(1),
            |a| a.stack_show_unresolved = true,
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
