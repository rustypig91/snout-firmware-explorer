use super::Explorer;
use eframe::egui::{self, RichText};
use egui_extras::{Column, TableBuilder};
use firmware_analysis_core::{memory::MemoryByte, Analysis, MemoryKind};
use std::sync::Arc;

const LINE_BYTES: u64 = 16;
const ROW_HEIGHT: f32 = 24.0;
// This bounds UI coordinates, not data storage. Only visible rows are decoded.
const WINDOW_ROWS: u64 = 1024;

#[derive(Default, Debug, Clone, Copy)]
struct ScrollPosition {
    row: u64,
    pixels: f32,
}
impl ScrollPosition {
    fn limit(total: u64, height: f32, stride: f32) -> Self {
        let covered = (height + (stride - ROW_HEIGHT)) / stride;
        let rows = covered.ceil() as u64;
        if total < rows {
            return Self::default();
        }
        Self {
            row: total - rows,
            pixels: (rows as f32 - covered) * stride,
        }
    }
    fn clamp(&mut self, limit: Self) {
        if self.row > limit.row || (self.row == limit.row && self.pixels > limit.pixels) {
            *self = limit;
        }
    }
    fn window(self, total: u64, height: f32, stride: f32) -> (u64, usize, f32) {
        let count = WINDOW_ROWS.max((height / stride).ceil() as u64 + WINDOW_ROWS);
        // Rebase in pairs of rows to keep the virtual viewport stable.
        let origin = self
            .row
            .saturating_sub(WINDOW_ROWS / 2)
            .min(total.saturating_sub(count))
            & !1;
        let count = (total - origin).min(count + 1) as usize;
        (
            origin,
            count,
            (self.row - origin) as f32 * stride + self.pixels,
        )
    }
    fn read_offset(&mut self, origin: u64, offset: f32, stride: f32) {
        let local_row = (offset / stride).floor() as u64;
        self.row = origin + local_row;
        self.pixels = (offset - local_row as f32 * stride).max(0.0);
    }
    fn move_rows(&mut self, rows: i64) {
        self.row = self.row.saturating_add_signed(rows);
        self.pixels = 0.0;
    }
}

#[derive(Clone)]
struct Range {
    label: String,
    kind: MemoryKind,
    start: u64,
    end: u64,
}
struct Annotation {
    address: u64,
    kind: MemoryKind,
    name: String,
    lookup_name: String,
    size: u64,
    detail: String,
}
const SYMBOL_HIGHLIGHT: egui::Color32 = super::views::ACCENT;

#[derive(Clone, Copy)]
struct HoverRange {
    address: u64,
    size: u64,
}
impl HoverRange {
    fn contains(self, address: u64) -> bool {
        address >= self.address && address - self.address < self.size
    }
}
struct ByteRun {
    rect: egui::Rect,
    address: u64,
    count: usize,
    byte_width: f32,
    reversed: bool,
}
impl ByteRun {
    fn byte_at(&self, display_index: usize) -> Option<(u64, egui::Rect)> {
        let offset = if self.reversed {
            self.count - 1 - display_index
        } else {
            display_index
        };
        let address = self.address.checked_add(offset as u64)?;
        let rect = egui::Rect::from_min_size(
            self.rect.min + egui::vec2(display_index as f32 * self.byte_width, 0.0),
            egui::vec2(self.byte_width, self.rect.height()),
        );
        Some((address, rect))
    }
    fn highlights(&self, hovered: HoverRange) -> impl Iterator<Item = egui::Rect> + '_ {
        (0..self.count).filter_map(move |display_index| {
            let (address, rect) = self.byte_at(display_index)?;
            hovered.contains(address).then_some(rect)
        })
    }
}
// Reserve outlines before painting text, then draw them after the hovered
// annotation is known. This highlights every visible line in the same frame.
struct ByteCell {
    painter: egui::Painter,
    background: egui::layers::ShapeIdx,
    runs: Vec<ByteRun>,
}
impl ByteCell {
    fn new(ui: &egui::Ui) -> Self {
        Self {
            painter: ui.painter().clone(),
            background: ui.painter().add(egui::Shape::Noop),
            runs: Vec::new(),
        }
    }
    fn highlight(&self, hovered: HoverRange) {
        // Merge adjacent bytes and full groups into a contour for each visible
        // range, including spaces between groups. Leave the text background clear.
        let mut contours: Vec<egui::Rect> = Vec::new();
        let mut previous_end = None;
        for run in &self.runs {
            if let Some(rect) = run.highlights(hovered).reduce(|a, b| a.union(b)) {
                let merge = previous_end.is_some_and(|end: f32| {
                    contours.last().is_some_and(|previous| {
                        (previous.right() - end).abs() < 0.5
                            && (rect.left() - run.rect.left()).abs() < 0.5
                    })
                });
                if merge {
                    let previous = contours.last_mut().unwrap();
                    *previous = previous.union(rect);
                } else {
                    contours.push(rect);
                }
            }
            previous_end = Some(run.rect.left() + run.count as f32 * run.byte_width);
        }
        let shapes = contours
            .into_iter()
            .map(|rect| {
                egui::Shape::rect_stroke(
                    rect.expand2(egui::vec2(2.0, 3.0)),
                    3.0,
                    egui::Stroke::new(1.0_f32, SYMBOL_HIGHLIGHT),
                )
            })
            .collect();
        self.painter.set(self.background, egui::Shape::Vec(shapes));
    }
}

#[derive(Default)]
pub(super) struct MemoryView {
    source: Option<Arc<Analysis>>,
    ranges: Vec<Range>,
    annotations: Vec<Annotation>,
    range: usize,
    scroll: ScrollPosition,
    scrollbar_grab: Option<f32>,
    unit: usize,
    little: bool,
    jump: String,
    error: Option<String>,
    scroll_to: Option<u64>,
    target: Option<u64>,
}
impl MemoryView {
    fn prepare(&mut self, analysis: &Arc<Analysis>) {
        if self
            .source
            .as_ref()
            .is_some_and(|old| Arc::ptr_eq(old, analysis))
        {
            return;
        }
        self.source = Some(analysis.clone());
        self.range = 0;
        self.scroll = ScrollPosition::default();
        self.scrollbar_grab = None;
        self.unit = usize::from(analysis.metadata.bitness / 8).clamp(1, 8);
        self.little = analysis.metadata.endianness == "Little";
        self.error = None;
        self.jump.clear();
        self.scroll_to = None;
        self.target = None;
        self.ranges.clear();
        self.annotations.clear();
        let Some(image) = &analysis.memory_image else {
            return;
        };
        for kind in [MemoryKind::Flash, MemoryKind::Ram] {
            let blocks: Vec<_> = image.blocks.iter().filter(|b| b.kind == kind).collect();
            let regions: Vec<_> = analysis
                .options
                .regions
                .iter()
                .filter(|r| r.kind == kind)
                .collect();
            let start = blocks
                .iter()
                .map(|b| b.address)
                .chain(regions.iter().map(|r| r.start))
                .min();
            let end = blocks
                .iter()
                .map(|b| b.address + b.size)
                .chain(regions.iter().map(|r| r.start + r.size))
                .max();
            if let Some((start, end)) = start.zip(end) {
                self.ranges.push(Range {
                    label: format!("All {}", kind_name(kind)),
                    kind,
                    start,
                    end,
                });
            }
            for region in regions {
                self.ranges.push(Range {
                    label: format!("{} · {}", region.name, kind_name(kind)),
                    kind,
                    start: region.start,
                    end: region.start + region.size,
                });
            }
        }
        let sections: std::collections::HashMap<_, _> =
            analysis.sections.iter().map(|s| (s.index, s)).collect();
        let mut symbols_by_section: std::collections::HashMap<_, Vec<_>> =
            std::collections::HashMap::new();
        for symbol in &analysis.symbols {
            symbols_by_section
                .entry(symbol.section_index)
                .or_default()
                .push(symbol);
        }
        for block in &image.blocks {
            let Some(section) = sections.get(&block.section_index) else {
                continue;
            };
            self.annotations.push(Annotation {
                address: block.address,
                kind: block.kind,
                name: section.name.clone(),
                lookup_name: section.name.clone(),
                size: section.size,
                detail: format!(
                    "Section {} · {} bytes{}",
                    section.name,
                    section.size,
                    if block.kind == MemoryKind::Flash && section.address != block.address {
                        " · load image"
                    } else {
                        ""
                    }
                ),
            });
            for symbol in symbols_by_section.get(&section.index).into_iter().flatten() {
                self.annotations.push(Annotation {
                    address: block.address + (symbol.normalized_address - section.address),
                    kind: block.kind,
                    name: symbol.demangled_name.clone(),
                    lookup_name: symbol.name.clone(),
                    size: symbol.size,
                    detail: format!(
                        "{} · {} · {} bytes",
                        symbol.kind, symbol.demangled_name, symbol.size
                    ),
                });
            }
        }
        self.annotations.sort_by_key(|a| a.address);
    }
    fn byte_tooltip(&self, kind: MemoryKind, address: u64, byte: MemoryByte) -> String {
        let contents = match byte {
            MemoryByte::File(value) => {
                format!("0x{value:02X} · {value} decimal · ELF initial value")
            }
            MemoryByte::InferredZero => {
                "0x00 · inferred BSS zero; startup code is not verified".into()
            }
            MemoryByte::Unknown => "?? · unknown contents".into(),
        };
        let mut text = format!("Address 0x{address:X} · {}\n{contents}", kind_name(kind));
        let end = self.annotations.partition_point(|a| a.address <= address);
        let mut matched = false;
        for annotation in self.annotations[..end]
            .iter()
            .filter(|a| a.kind == kind && (a.address == address || address - a.address < a.size))
        {
            matched = true;
            text.push_str(&format!(
                "\n{} · starts at 0x{:X} · offset +0x{:X}{}",
                annotation.detail,
                annotation.address,
                address - annotation.address,
                if annotation.size == 0 {
                    " · no byte extent is defined"
                } else {
                    ""
                },
            ));
        }
        if !matched {
            text.push_str("\nNo associated section or symbol.");
        }
        text
    }
    fn navigate(&mut self) {
        let input = self.jump.trim();
        let explicit_address = input
            .strip_prefix("0x")
            .or_else(|| input.strip_prefix("0X"));
        let address = u64::from_str_radix(explicit_address.unwrap_or(input), 16).ok();
        let current_kind = self.ranges[self.range].kind;
        // Names such as `adc` are valid hexadecimal too. Prefer an exact name
        // unless the user explicitly requests an address with a 0x prefix.
        let symbol_target = if explicit_address.is_none() {
            let matches: Vec<_> = self
                .annotations
                .iter()
                .filter(|a| a.name == input || a.lookup_name == input)
                .collect();
            matches
                .iter()
                .find(|a| a.kind == current_kind)
                .or(matches.first())
                .map(|a| (a.address, a.kind))
        } else {
            None
        };
        let target = symbol_target.or_else(|| address.map(|a| (a, current_kind)));
        let Some((address, kind)) = target else {
            self.error =
                Some("Enter a hexadecimal address or an exact symbol/section name.".into());
            return;
        };
        let current = &self.ranges[self.range];
        if current.kind != kind || address < current.start || address >= current.end {
            let Some(index) = self
                .ranges
                .iter()
                .position(|r| r.kind == kind && address >= r.start && address < r.end)
            else {
                self.error =
                    Some("Address is outside the known memory layout for this space.".into());
                return;
            };
            self.range = index;
        }
        self.scroll_to = Some((address - (self.ranges[self.range].start & !15)) / LINE_BYTES);
        self.target = Some(address);
        self.error = None;
    }
}
// The scrollbar represents the entire address span; its thumb is approximate for
// enormous ranges, while wheel scrolling and jumps retain exact u64 row positions.
fn memory_scrollbar(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    position: &mut ScrollPosition,
    limit: ScrollPosition,
    total: u64,
    visible: f32,
    grab: &mut Option<f32>,
) -> egui::Response {
    let response = ui.interact(
        rect,
        ui.id().with("memory_scrollbar"),
        egui::Sense::click_and_drag(),
    );
    if limit.row == 0 && limit.pixels == 0.0 {
        return response;
    }
    let thumb_height = (rect.height() * (visible as f64 / total as f64) as f32)
        .clamp(18.0_f32.min(rect.height()), rect.height());
    let travel = (rect.height() - thumb_height).max(0.0);
    let maximum =
        limit.row as f64 + limit.pixels as f64 / (ROW_HEIGHT + ui.spacing().item_spacing.y) as f64;
    let ratio = (position.row as f64
        + position.pixels as f64 / (ROW_HEIGHT + ui.spacing().item_spacing.y) as f64)
        / maximum;
    let thumb_top = rect.top() + travel * ratio as f32;
    if let Some(pointer) = response.interact_pointer_pos() {
        if response.drag_started() || response.clicked() {
            *grab = Some(
                if pointer.y >= thumb_top && pointer.y <= thumb_top + thumb_height {
                    pointer.y - thumb_top
                } else {
                    thumb_height / 2.0
                },
            );
        }
        let on_thumb = pointer.y >= thumb_top && pointer.y <= thumb_top + thumb_height;
        if response.clicked() || response.drag_started() {
            response.request_focus();
        }
        if response.dragged() || (response.clicked() && !on_thumb) {
            let ratio = ((pointer.y - rect.top() - grab.unwrap_or(thumb_height / 2.0))
                / travel.max(1.0))
            .clamp(0.0, 1.0);
            if ratio >= 1.0 {
                *position = limit;
            } else if ratio <= 0.0 {
                *position = ScrollPosition::default();
            } else {
                *position = ScrollPosition {
                    row: (ratio as f64 * maximum).round() as u64,
                    pixels: 0.0,
                };
            }
        }
    }
    let visuals = ui.style().interact(&response);
    ui.painter()
        .rect_filled(rect, 4.0, ui.visuals().extreme_bg_color);
    let ratio = (position.row as f64 / maximum).clamp(0.0, 1.0);
    ui.painter().rect_filled(
        egui::Rect::from_min_size(
            egui::pos2(rect.left(), rect.top() + travel * ratio as f32),
            egui::vec2(rect.width(), thumb_height),
        ),
        4.0,
        visuals.bg_fill,
    );
    response
}

fn kind_name(kind: MemoryKind) -> &'static str {
    match kind {
        MemoryKind::Flash => "Flash",
        MemoryKind::Ram => "RAM startup",
    }
}
fn group_hex(bytes: &[MemoryByte], little: bool) -> String {
    let pairs: Vec<_> = bytes
        .iter()
        .map(|b| {
            b.value()
                .map_or_else(|| "??".into(), |v| format!("{v:02X}"))
        })
        .collect();
    if little {
        pairs.into_iter().rev().collect()
    } else {
        pairs.concat()
    }
}
impl Explorer {
    pub(super) fn memory_view(&mut self, ui: &mut egui::Ui, analysis: &Arc<Analysis>) {
        let state = &mut self.memory_view;
        state.prepare(analysis);
        let Some(image) = &analysis.memory_image else {
            ui.label("Memory contents are unavailable in this report. Open or refresh the firmware ELF to load its bytes.");
            return;
        };
        if state.ranges.is_empty() {
            ui.label("No addressable Flash or RAM ranges are available.");
            return;
        }
        ui.horizontal_wrapped(|ui| {
            let previous = state.range;
            egui::ComboBox::from_id_salt("memory_space")
                .selected_text(&state.ranges[state.range].label)
                .show_ui(ui, |ui| {
                    for (index, range) in state.ranges.iter().enumerate() {
                        ui.selectable_value(&mut state.range, index, &range.label);
                    }
                });
            if previous != state.range {
                state.scroll = ScrollPosition::default();
                state.scrollbar_grab = None;
                state.error = None;
                state.target = None;
                state.scroll_to = None;
            }
            egui::ComboBox::from_id_salt("memory_unit")
                .selected_text(format!("{} bytes", state.unit))
                .show_ui(ui, |ui| {
                    for (unit, label) in [
                        (1, "Byte (8-bit)"),
                        (2, "Halfword (16-bit)"),
                        (4, "Word (32-bit)"),
                        (8, "Doubleword (64-bit)"),
                    ] {
                        ui.selectable_value(&mut state.unit, unit, label);
                    }
                });
            egui::ComboBox::from_id_salt("memory_endian")
                .selected_text(if state.little {
                    "Little endian"
                } else {
                    "Big endian"
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut state.little, true, "Little endian");
                    ui.selectable_value(&mut state.little, false, "Big endian");
                });
            if ui.small_button("Firmware defaults").clicked() {
                state.unit = usize::from(analysis.metadata.bitness / 8).clamp(1, 8);
                state.little = analysis.metadata.endianness == "Little";
            }
        });
        ui.horizontal_wrapped(|ui| {
            let input = ui.add(
                egui::TextEdit::singleline(&mut state.jump)
                    .desired_width(250.0)
                    .hint_text("Hex address or exact symbol name"),
            );
            if ui.button("Go").clicked()
                || (input.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)))
            {
                state.navigate();
            }
        });
        if let Some(error) = &state.error {
            ui.colored_label(egui::Color32::LIGHT_RED, error);
        }
        let range = state.ranges[state.range].clone();
        ui.label(if range.kind == MemoryKind::Ram {
            "RAM startup estimate: ELF initial values + conventional BSS zeros. Actual startup code, relocations, heap, stack and live RAM are not modeled."
        } else {
            "Flash load addresses from the ELF. Gaps, segment padding and bytes outside section payloads are unknown; erased Flash is not assumed."
        });
        if !analysis
            .options
            .regions
            .iter()
            .any(|r| r.kind == range.kind)
        {
            ui.weak("No configured capacity for this space: bounds cover ELF sections, not the whole chip.");
        }
        ui.horizontal_wrapped(|ui| {
            ui.monospace("?? = unknown");
            ui.colored_label(super::views::ACCENT, "Blue hex = inferred BSS zeros");
            ui.weak("ASCII stays in address order. Labels at right start within that 16-byte line; +offset gives the byte position.");
        });
        let base = range.start & !15;
        let total_rows = (range.end - base).div_ceil(LINE_BYTES);
        ui.monospace(format!(
            "Range 0x{:X}–0x{:X} · {} lines",
            range.start,
            range.end - 1,
            total_rows
        ));
        ui.separator();
        let height = ui.available_height();
        let stride = ROW_HEIGHT + ui.spacing().item_spacing.y;
        let body_height = (height - stride - ui.spacing().scroll.allocated_width()).max(1.0);
        let limit = ScrollPosition::limit(total_rows, body_height, stride);
        if let Some(row) = state.scroll_to.take() {
            state.scroll = ScrollPosition {
                row: row.saturating_sub((body_height / stride / 2.0) as u64),
                pixels: 0.0,
            };
        }
        state.scroll.clamp(limit);
        let available = ui.available_rect_before_wrap();
        let scrollbar_rect = egui::Rect::from_min_max(
            egui::pos2(available.right() - 12.0, available.top() + stride),
            egui::pos2(available.right(), available.top() + stride + body_height),
        );
        let response = memory_scrollbar(
            ui,
            scrollbar_rect,
            &mut state.scroll,
            limit,
            total_rows,
            body_height / stride,
            &mut state.scrollbar_grab,
        );
        let editing_text = ui
            .memory(|m| m.focused())
            .is_some_and(|id| egui::TextEdit::load_state(ui.ctx(), id).is_some());
        if !editing_text && (ui.rect_contains_pointer(available) || response.has_focus()) {
            ui.input_mut(|input| {
                for (key, movement) in [
                    (egui::Key::ArrowDown, 1),
                    (egui::Key::ArrowUp, -1),
                    (egui::Key::PageDown, (body_height / stride).floor() as i64),
                    (egui::Key::PageUp, -(body_height / stride).floor() as i64),
                ] {
                    if input.consume_key(egui::Modifiers::NONE, key) {
                        state.scroll.move_rows(movement);
                    }
                }
                if input.consume_key(egui::Modifiers::NONE, egui::Key::Home) {
                    state.scroll = ScrollPosition::default();
                }
                if input.consume_key(egui::Modifiers::NONE, egui::Key::End) {
                    state.scroll = limit;
                }
            });
        }
        state.scroll.clamp(limit);
        let (origin, count, offset) = state.scroll.window(total_rows, body_height, stride);
        let mut rendered_rows = 0;
        let mut byte_cells = Vec::new();
        let mut hovered_range = None;
        let digit_width = ui
            .fonts(|fonts| fonts.glyph_width(&egui::TextStyle::Monospace.resolve(ui.style()), '0'));
        let address_width = digit_width
            * if analysis.metadata.bitness == 64 {
                16.0
            } else {
                8.0
            }
            + 12.0;
        let hex_width = digit_width * 32.0 + (16 / state.unit - 1) as f32 * 10.0 + 12.0;
        let ascii_width = digit_width * 16.0 + 12.0;
        ui.allocate_ui_with_layout(
            egui::vec2((available.width() - 20.0).max(1.0), height),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                egui::ScrollArea::horizontal()
                    .id_salt("memory_horizontal")
                    .show(ui, |ui| {
                        ui.set_min_height(height);
                        ui.set_min_width(address_width + hex_width + ascii_width + 350.0);
                        let mut table = TableBuilder::new(ui)
                            .id_salt((state.range, Arc::as_ptr(analysis) as usize))
                            .vertical_scroll_offset(offset)
                            .scroll_bar_visibility(
                                egui::scroll_area::ScrollBarVisibility::AlwaysHidden,
                            )
                            .max_scroll_height(body_height)
                            .min_scrolled_height(body_height)
                            .auto_shrink([false, false])
                            .striped(false)
                            .resizable(true)
                            .column(
                                Column::initial(address_width)
                                    .at_least(address_width)
                                    .clip(true),
                            )
                            .column(Column::initial(hex_width).at_least(hex_width).clip(true))
                            .column(
                                Column::initial(ascii_width)
                                    .at_least(ascii_width)
                                    .clip(true),
                            )
                            .column(Column::remainder().at_least(300.0));
                        table = table.cell_layout(egui::Layout::left_to_right(egui::Align::Center));
                        let output = table
                            .header(ROW_HEIGHT, |mut header| {
                                for title in [
                                    "Address",
                                    "Hex values",
                                    "ASCII",
                                    "Sections / symbols starting on this line",
                                ] {
                                    header.col(|ui| {
                                        ui.strong(title);
                                    });
                                }
                            })
                            .body(|body| {
                                body.rows(ROW_HEIGHT, count, |mut row| {
                                    rendered_rows += 1;
                                    let address = base + (origin + row.index() as u64) * LINE_BYTES;
                                    row.set_selected(state.target.is_some_and(|target| {
                                        target >= address && target - address < LINE_BYTES
                                    }));
                                    let values: [MemoryByte; 16] = std::array::from_fn(|offset| {
                                        address
                                            .checked_add(offset as u64)
                                            .filter(|a| *a >= range.start && *a < range.end)
                                            .map_or(MemoryByte::Unknown, |a| {
                                                image.byte(range.kind, a)
                                            })
                                    });
                                    row.col(|ui| {
                                        ui.monospace(format!(
                                            "{address:0width$X}",
                                            width = if analysis.metadata.bitness == 64 {
                                                16
                                            } else {
                                                8
                                            }
                                        ));
                                    });
                                    row.col(|ui| {
                                        ui.spacing_mut().item_spacing.x = 10.0;
                                        let mut cell = ByteCell::new(ui);
                                        for (group_index, group) in
                                            values.chunks(state.unit).enumerate()
                                        {
                                            let text =
                                                RichText::new(group_hex(group, state.little))
                                                    .monospace();
                                            let text = if group.contains(&MemoryByte::InferredZero)
                                            {
                                                text.color(super::views::ACCENT)
                                            } else if group
                                                .iter()
                                                .all(|b| *b == MemoryByte::Unknown)
                                            {
                                                text.weak()
                                            } else {
                                                text
                                            };
                                            let response = ui.label(text);
                                            let run = ByteRun {
                                                rect: response.rect,
                                                address: address
                                                    + (group_index * state.unit) as u64,
                                                count: group.len(),
                                                byte_width: response.rect.width()
                                                    / group.len() as f32,
                                                reversed: state.little,
                                            };
                                            for display_index in 0..run.count {
                                                let Some((byte_address, rect)) =
                                                    run.byte_at(display_index)
                                                else {
                                                    continue;
                                                };
                                                let response = ui.interact(
                                                    rect,
                                                    ui.id().with(("hex_byte", byte_address)),
                                                    egui::Sense::hover(),
                                                );
                                                if response.hovered() {
                                                    hovered_range = Some(HoverRange {
                                                        address: byte_address,
                                                        size: 1,
                                                    });
                                                }
                                                response.on_hover_ui(|ui| {
                                                    let byte =
                                                        values[(byte_address - address) as usize];
                                                    ui.label(state.byte_tooltip(
                                                        range.kind,
                                                        byte_address,
                                                        byte,
                                                    ));
                                                });
                                            }
                                            cell.runs.push(run);
                                        }
                                        byte_cells.push(cell);
                                    });
                                    row.col(|ui| {
                                        let ascii: String = values
                                            .iter()
                                            .map(|b| match b.value() {
                                                Some(v @ 32..=126) => char::from(v),
                                                Some(_) => '.',
                                                None => '?',
                                            })
                                            .collect();
                                        let mut cell = ByteCell::new(ui);
                                        let response = ui.monospace(ascii);
                                        cell.runs.push(ByteRun {
                                            rect: response.rect,
                                            address,
                                            count: values.len(),
                                            byte_width: response.rect.width() / values.len() as f32,
                                            reversed: false,
                                        });
                                        byte_cells.push(cell);
                                    });
                                    row.col(|ui| {
                                        let first = state
                                            .annotations
                                            .partition_point(|a| a.address < address);
                                        let labels: Vec<_> = state.annotations[first..]
                                            .iter()
                                            .take_while(|a| a.address - address < LINE_BYTES)
                                            .filter(|a| a.kind == range.kind)
                                            .collect();
                                        ui.spacing_mut().item_spacing.x = 6.0;
                                        for label in labels {
                                            let response = ui
                                                .add(
                                                    egui::Label::new(
                                                        RichText::new(format!(
                                                            "+{:02X} {}",
                                                            label.address - address,
                                                            label.name
                                                        ))
                                                        .monospace(),
                                                    )
                                                    .truncate(),
                                                )
                                                .on_hover_text(format!(
                                                    "0x{:X}: {}{}",
                                                    label.address,
                                                    label.detail,
                                                    if label.size == 0 {
                                                        " · no byte extent is defined"
                                                    } else {
                                                        ""
                                                    }
                                                ));
                                            if response.hovered() {
                                                hovered_range = Some(HoverRange {
                                                    address: label.address,
                                                    size: label.size,
                                                });
                                            }
                                        }
                                    });
                                });
                            });
                        state
                            .scroll
                            .read_offset(origin, output.state.offset.y, stride);
                        state.scroll.clamp(limit);
                    });
            },
        );
        if let Some(hovered) = hovered_range {
            for cell in &byte_cells {
                cell.highlight(hovered);
            }
        }
        self.visible_rows = rendered_rows;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn byte_tooltips_resolve_aliases_extents_and_both_load_and_runtime_addresses() {
        let mut analysis = firmware_analysis_core::analyze_bytes(
            include_bytes!("../../../fixtures/build/cortex-m.elf"),
            "fixture",
            &Default::default(),
        )
        .unwrap();
        let section = analysis
            .sections
            .iter()
            .find(|s| s.name == ".data")
            .unwrap();
        let runtime = section.address;
        let load = section.load_address.unwrap();
        let mut symbol = analysis.symbols[0].clone();
        symbol.section_index = section.index;
        symbol.normalized_address = runtime;
        symbol.demangled_name = "data_symbol".into();
        symbol.size = 2;
        let mut alias = symbol.clone();
        alias.demangled_name = "data_alias".into();
        let mut zero = symbol.clone();
        zero.demangled_name = "zero_label".into();
        zero.size = 0;
        analysis.symbols = vec![symbol, alias, zero];
        let analysis = Arc::new(analysis);
        let mut view = MemoryView::default();
        view.prepare(&analysis);
        for (kind, address) in [(MemoryKind::Flash, load), (MemoryKind::Ram, runtime)] {
            let tooltip = view.byte_tooltip(kind, address, MemoryByte::File(42));
            assert!(tooltip.contains(&format!("Address 0x{address:X}")));
            assert!(tooltip.contains("0x2A · 42 decimal · ELF initial value"));
            for name in ["Section .data", "data_symbol", "data_alias", "zero_label"] {
                assert!(tooltip.contains(name), "{tooltip}");
            }
            let interior = view.byte_tooltip(kind, address + 1, MemoryByte::File(0));
            assert!(interior.contains("data_symbol"));
            assert!(interior.contains("data_alias"));
            assert!(interior.contains("offset +0x1"));
            assert!(!interior.contains("zero_label"));
            let end = view.byte_tooltip(kind, address + 2, MemoryByte::File(0));
            assert!(!end.contains("data_symbol"));
            assert!(!end.contains("data_alias"));
        }
        let unknown = view.byte_tooltip(MemoryKind::Flash, 0, MemoryByte::Unknown);
        assert!(unknown.contains("unknown contents"));
        assert!(unknown.contains("No associated section or symbol"));
        let inferred = view.byte_tooltip(MemoryKind::Ram, runtime, MemoryByte::InferredZero);
        assert!(inferred.contains("inferred BSS zero; startup code is not verified"));
    }

    #[test]
    fn hovering_hex_pairs_shows_the_exact_address_for_all_groupings_and_byte_orders() {
        let analysis = Arc::new(
            firmware_analysis_core::analyze_bytes(
                include_bytes!("../../../fixtures/build/cortex-m.elf"),
                "fixture",
                &Default::default(),
            )
            .unwrap(),
        );
        for unit in [1, 2, 4, 8] {
            for little in [false, true] {
                let mut app = Explorer::default();
                app.memory_view.prepare(&analysis);
                app.memory_view.unit = unit;
                app.memory_view.little = little;
                let base = app.memory_view.ranges[0].start & !15;
                let ctx = egui::Context::default();
                super::super::shell::configure_style(&ctx);
                let mut render = |time, pointer: Option<egui::Pos2>| {
                    ctx.run(
                        egui::RawInput {
                            screen_rect: Some(egui::Rect::from_min_size(
                                egui::Pos2::ZERO,
                                egui::vec2(1500.0, 820.0),
                            )),
                            time: Some(time),
                            events: pointer
                                .map(|pos| vec![egui::Event::PointerMoved(pos)])
                                .unwrap_or_default(),
                            ..Default::default()
                        },
                        |ctx| {
                            egui::CentralPanel::default()
                                .show(ctx, |ui| app.memory_view(ui, &analysis));
                        },
                    )
                };
                render(0.0, None);
                let initial = render(0.1, None);
                let row_y = initial
                    .shapes
                    .iter()
                    .find_map(|shape| match &shape.shape {
                        egui::Shape::Text(text) if text.galley.text() == format!("{base:08X}") => {
                            Some(text.pos.y)
                        }
                        _ => None,
                    })
                    .unwrap();
                let hex = initial
                    .shapes
                    .iter()
                    .filter_map(|shape| match &shape.shape {
                        egui::Shape::Text(text)
                            if (text.pos.y - row_y).abs() < 0.5
                                && text.galley.text() != format!("{base:08X}") =>
                        {
                            Some(text)
                        }
                        _ => None,
                    })
                    .min_by(|a, b| a.pos.x.total_cmp(&b.pos.x))
                    .unwrap();
                let pair_width = hex.galley.size().x / unit as f32;
                fn outlines(shape: &egui::Shape) -> Vec<egui::Rect> {
                    match shape {
                        egui::Shape::Vec(shapes) => shapes.iter().flat_map(outlines).collect(),
                        egui::Shape::Rect(rect) if rect.stroke.color == SYMBOL_HIGHLIGHT => {
                            vec![rect.rect]
                        }
                        _ => Vec::new(),
                    }
                }
                for display_index in 0..unit {
                    let pointer = hex.pos
                        + egui::vec2(
                            pair_width * (display_index as f32 + 0.5),
                            hex.galley.size().y / 2.0,
                        );
                    let time = 1.0 + display_index as f64 * 3.0;
                    let hovered = render(time, Some(pointer));
                    let rects: Vec<_> = hovered
                        .shapes
                        .iter()
                        .flat_map(|shape| outlines(&shape.shape))
                        .collect();
                    assert_eq!(
                        rects.len(),
                        2,
                        "Highlight one hex byte and its ASCII character"
                    );
                    let hex_rect = rects.iter().find(|rect| rect.contains(pointer)).unwrap();
                    assert!((hex_rect.width() - (pair_width + 4.0)).abs() < 0.5);
                    assert!((hex_rect.center().x - pointer.x).abs() < 1.0, "unit {unit}, little {little}, pair {display_index}: rect {hex_rect:?}, pointer {pointer:?}");
                    render(time + 1.0, Some(pointer));
                    let output = render(time + 2.0, Some(pointer));
                    let offset = if little {
                        unit - 1 - display_index
                    } else {
                        display_index
                    };
                    let expected = format!("Address 0x{:X} · Flash", base + offset as u64);
                    assert!(output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text().contains(&expected))),
                        "Missing {expected} for unit {unit}, little {little}, pair {display_index}");
                    let cleared = render(time + 2.5, Some(egui::pos2(5.0, 5.0)));
                    assert!(cleared
                        .shapes
                        .iter()
                        .all(|shape| outlines(&shape.shape).is_empty()));
                }
            }
        }
    }

    #[test]
    fn hover_highlights_exact_byte_positions_for_each_grouping_and_endian_mode() {
        for count in [1, 2, 4, 8] {
            for reversed in [false, true] {
                let run = ByteRun {
                    rect: egui::Rect::from_min_size(
                        egui::pos2(10.0, 20.0),
                        egui::vec2(count as f32 * 14.0, 24.0),
                    ),
                    address: 0x100,
                    count,
                    byte_width: 14.0,
                    reversed,
                };
                for offset in 0..count {
                    let rects: Vec<_> = run
                        .highlights(HoverRange {
                            address: 0x100 + offset as u64,
                            size: 1,
                        })
                        .collect();
                    assert_eq!(rects.len(), 1);
                    let display = if reversed { count - 1 - offset } else { offset };
                    assert_eq!(rects[0].left(), 10.0 + display as f32 * 14.0);
                    assert_eq!(rects[0].width(), 14.0);
                }
                assert_eq!(
                    run.highlights(HoverRange {
                        address: 0x100,
                        size: 0
                    })
                    .count(),
                    0
                );
            }
        }
        let near_max = HoverRange {
            address: u64::MAX - 2,
            size: 2,
        };
        assert!(near_max.contains(u64::MAX - 1));
        assert!(!near_max.contains(u64::MAX));
        assert!(!near_max.contains(0));
    }

    #[test]
    fn individual_symbol_hover_highlights_all_visible_bytes_and_clears_on_leave() {
        let mut analysis = firmware_analysis_core::analyze_bytes(
            include_bytes!("../../../fixtures/build/cortex-m.elf"),
            "fixture",
            &Default::default(),
        )
        .unwrap();
        let section = analysis
            .sections
            .iter()
            .find(|s| s.name == ".text")
            .unwrap();
        assert!(section.size > 20);
        let mut symbol = analysis.symbols[0].clone();
        symbol.section_index = section.index;
        symbol.section = section.name.clone();
        symbol.address = section.address + 1;
        symbol.normalized_address = symbol.address;
        symbol.name = "span".into();
        symbol.demangled_name = symbol.name.clone();
        symbol.size = 19;
        let mut alias = symbol.clone();
        alias.name = "alias".into();
        alias.demangled_name = alias.name.clone();
        alias.size = 3;
        let mut zero = symbol.clone();
        zero.name = "zero".into();
        zero.demangled_name = zero.name.clone();
        zero.size = 0;
        analysis.symbols = vec![symbol, alias, zero];
        let analysis = Arc::new(analysis);
        let mut app = Explorer::default();
        let ctx = egui::Context::default();
        super::super::shell::configure_style(&ctx);
        let mut render = |pointer: Option<egui::Pos2>| {
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1500.0, 820.0),
                    )),
                    events: pointer
                        .map(|pos| vec![egui::Event::PointerMoved(pos)])
                        .unwrap_or_default(),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| app.memory_view(ui, &analysis));
                },
            )
        };
        fn highlighted(shape: &egui::Shape) -> usize {
            match shape {
                egui::Shape::Vec(shapes) => shapes.iter().map(highlighted).sum(),
                egui::Shape::Rect(rect) if rect.stroke.color == SYMBOL_HIGHLIGHT => {
                    assert_eq!(rect.fill, egui::Color32::TRANSPARENT);
                    1
                }
                _ => 0,
            }
        }
        let count = |output: &egui::FullOutput| {
            output
                .shapes
                .iter()
                .map(|shape| highlighted(&shape.shape))
                .sum::<usize>()
        };
        render(None::<egui::Pos2>);
        let initial = render(None);
        for (name, expected) in [("span", 5), ("alias", 2), ("zero", 0)] {
            let pointer = initial
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text() == format!("+01 {name}") => {
                        Some(text.pos + text.galley.size() / 2.0)
                    }
                    _ => None,
                })
                .unwrap_or_else(|| panic!("Missing annotation {name}"));
            let output = render(Some(pointer));
            assert_eq!(count(&output), expected, "{name}");
            assert_eq!(count(&render(Some(egui::pos2(5.0, 5.0)))), 0);
        }
    }

    #[test]
    fn bounded_windows_preserve_exact_rows_and_fractional_scroll_at_large_addresses() {
        let total = 1_u64 << 60;
        let stride = 28.0;
        let limit = ScrollPosition::limit(total, 563.0, stride);
        for row in [0, 999, 1024, 100_000_000, total - 1000, limit.row] {
            let position = ScrollPosition { row, pixels: 3.25 };
            let (origin, count, offset) = position.window(total, 563.0, stride);
            assert!(count <= 1046);
            assert_eq!(origin % 2, 0);
            let mut recovered = ScrollPosition::default();
            recovered.read_offset(origin, offset, stride);
            assert_eq!(recovered.row, row);
            assert_eq!(recovered.pixels, 3.25);
            recovered.read_offset(origin, offset + stride, stride);
            assert_eq!(recovered.row, row + 1);
        }
    }

    #[test]
    fn scrollbar_reaches_both_ends_without_moving_on_a_thumb_click() {
        let ctx = egui::Context::default();
        let total = 1_u64 << 60;
        let limit = ScrollPosition {
            row: total - 25,
            pixels: 4.0,
        };
        let mut position = ScrollPosition {
            row: total / 3 + 7,
            pixels: 0.0,
        };
        let initial = position.row;
        let mut grab = None;
        let rect = egui::Rect::from_min_max(egui::pos2(800.0, 100.0), egui::pos2(812.0, 600.0));
        let thumb_center = 100.0 + 482.0 / 3.0 + 9.0;
        let click = |y: f32, pressed: bool| {
            vec![
                egui::Event::PointerMoved(egui::pos2(806.0, y)),
                egui::Event::PointerButton {
                    pos: egui::pos2(806.0, y),
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ]
        };
        let mut render = |events| {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1280.0, 820.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        memory_scrollbar(ui, rect, &mut position, limit, total, 25.0, &mut grab);
                    });
                },
            );
            position.row
        };
        render(vec![]);
        render(click(thumb_center, true));
        assert_eq!(render(click(thumb_center, false)), initial);
        render(click(599.0, true));
        assert_eq!(render(click(599.0, false)), limit.row);
        render(click(101.0, true));
        assert_eq!(render(click(101.0, false)), 0);
    }

    #[test]
    fn wheel_scroll_rebases_continuously_and_only_renders_viewport_rows() {
        let mut analysis = firmware_analysis_core::analyze_bytes(
            include_bytes!("../../../fixtures/build/cortex-m.elf"),
            "fixture",
            &Default::default(),
        )
        .unwrap();
        analysis
            .options
            .regions
            .push(firmware_analysis_core::MemoryRegion {
                name: "Large Flash".into(),
                kind: MemoryKind::Flash,
                start: 0x08000000,
                size: 512 * 1024 * 1024,
            });
        let analysis = Arc::new(analysis);
        let mut app = Explorer::default();
        app.memory_view.prepare(&analysis);
        app.memory_view.scroll.row = 1000;
        let ctx = egui::Context::default();
        super::super::shell::configure_style(&ctx);
        let mut texts = Vec::new();
        for frame in 0..5 {
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1280.0, 820.0),
                    )),
                    time: Some(frame as f64 / 60.0),
                    events: if frame == 1 {
                        vec![
                            egui::Event::PointerMoved(egui::pos2(500.0, 500.0)),
                            egui::Event::MouseWheel {
                                unit: egui::MouseWheelUnit::Point,
                                delta: egui::vec2(0.0, -150.0),
                                modifiers: egui::Modifiers::NONE,
                            },
                        ]
                    } else {
                        vec![]
                    },
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| app.memory_view(ui, &analysis));
                },
            );
            assert!(app.visible_rows > 0 && app.visible_rows < 50);
            texts = output
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) => Some(text.galley.text().to_owned()),
                    _ => None,
                })
                .collect();
        }
        assert!(app.memory_view.scroll.row > 1000);
        assert!(app.memory_view.scroll.row < 1020);
        let addresses: Vec<_> = texts
            .iter()
            .filter(|text| text.len() == 8)
            .filter_map(|text| u64::from_str_radix(text, 16).ok())
            .filter(|address| *address >= 0x08003E80 && *address < 0x08010000)
            .collect();
        assert!(addresses.len() > 10);
        assert!(addresses
            .windows(2)
            .all(|pair| pair[1] == pair[0] + LINE_BYTES));
        assert!(!texts
            .iter()
            .any(|text| text.starts_with("Page ") || text == "Next" || text == "Previous"));
    }

    #[test]
    fn groups_preserve_unknown_byte_positions_in_both_endian_modes() {
        let values = [
            MemoryByte::File(0x12),
            MemoryByte::Unknown,
            MemoryByte::File(0x34),
            MemoryByte::InferredZero,
        ];
        assert_eq!(group_hex(&values, true), "0034??12");
        assert_eq!(group_hex(&values, false), "12??3400");
    }
    #[test]
    fn sparse_64_bit_ranges_render_unknown_tail_without_overflow_or_full_range_allocation() {
        let mut analysis = firmware_analysis_core::analyze_bytes(
            include_bytes!("../../../fixtures/build/cortex-m-stripped.elf"),
            "fixture",
            &Default::default(),
        )
        .unwrap();
        analysis.metadata.bitness = 64;
        analysis
            .options
            .regions
            .push(firmware_analysis_core::MemoryRegion {
                name: "High RAM".into(),
                kind: MemoryKind::Ram,
                start: u64::MAX - 31,
                size: 31,
            });
        let analysis = Arc::new(analysis);
        let mut app = Explorer::default();
        app.memory_view.prepare(&analysis);
        app.memory_view.range = app
            .memory_view
            .ranges
            .iter()
            .position(|r| r.label.starts_with("High RAM"))
            .unwrap();
        app.memory_view.jump = format!("0x{:X}", u64::MAX - 2);
        app.memory_view.navigate();
        assert!(app.memory_view.error.is_none());
        assert_eq!(app.memory_view.scroll_to, Some(1));
        let ctx = egui::Context::default();
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1280.0, 820.0),
                )),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| app.memory_view(ui, &analysis));
            },
        );
        assert_eq!(app.visible_rows, 2);
        assert!(output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text().contains("????????????????"))));
        app.memory_view.range = app
            .memory_view
            .ranges
            .iter()
            .position(|r| r.label == "All RAM startup")
            .unwrap();
        app.memory_view.navigate();
        let output = ctx.run(Default::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| app.memory_view(ui, &analysis));
        });
        assert!(app.visible_rows < 50);
        assert!(output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == "FFFFFFFFFFFFFFF0")));
    }
    #[test]
    fn hexadecimal_symbol_names_can_be_jumped_to_without_losing_address_navigation() {
        let mut analysis = firmware_analysis_core::analyze_bytes(
            include_bytes!("../../../fixtures/build/cortex-m.elf"),
            "fixture",
            &Default::default(),
        )
        .unwrap();
        let mut symbol = analysis
            .symbols
            .iter()
            .find(|s| s.size > 0)
            .unwrap()
            .clone();
        symbol.name = "adc".into();
        symbol.demangled_name = symbol.name.clone();
        let mut prefixed_symbol = symbol.clone();
        prefixed_symbol.name = "0xADC".into();
        prefixed_symbol.demangled_name = prefixed_symbol.name.clone();
        analysis.symbols = vec![symbol, prefixed_symbol];
        let mut view = MemoryView::default();
        view.prepare(&Arc::new(analysis));
        let address = view
            .annotations
            .iter()
            .find(|a| a.name == "adc")
            .unwrap()
            .address;
        view.jump = "adc".into();
        view.navigate();
        assert!(view.error.is_none(), "{:?}", view.error);
        assert_eq!(view.target, Some(address));

        // An explicit prefix always means an address, even if a symbol matches.
        view.jump = "0xADC".into();
        view.navigate();
        assert!(view.error.is_some());
        for input in [
            format!("{address:X}"),
            format!("0x{address:X}"),
            format!("0X{address:X}"),
        ] {
            view.jump = input;
            view.navigate();
            assert!(view.error.is_none());
            assert_eq!(view.target, Some(address));
        }
    }

    #[test]
    fn defaults_and_navigation_follow_firmware_and_load_addresses() {
        let analysis = Arc::new(
            firmware_analysis_core::analyze_bytes(
                include_bytes!("../../../fixtures/build/cortex-m.elf"),
                "fixture",
                &Default::default(),
            )
            .unwrap(),
        );
        let mut view = MemoryView::default();
        view.prepare(&analysis);
        assert_eq!(view.unit, 4);
        assert!(view.little);
        view.jump = ".data".into();
        view.navigate();
        assert!(view.error.is_none());
        let data = analysis
            .sections
            .iter()
            .find(|s| s.name == ".data")
            .unwrap();
        assert!(view.annotations.iter().any(|a| a.name == ".data"
            && a.kind == MemoryKind::Flash
            && a.address == data.load_address.unwrap()));
        view.range = view
            .ranges
            .iter()
            .position(|r| r.kind == MemoryKind::Ram)
            .unwrap();
        view.navigate();
        assert_eq!(view.ranges[view.range].kind, MemoryKind::Ram);
        view.unit = 1;
        view.prepare(&analysis);
        assert_eq!(view.unit, 1); // Repaints retain user choices.
        let mut big = (*analysis).clone();
        big.metadata.bitness = 64;
        big.metadata.endianness = "Big".into();
        view.prepare(&Arc::new(big));
        assert_eq!(view.unit, 8);
        assert!(!view.little);
    }
}
