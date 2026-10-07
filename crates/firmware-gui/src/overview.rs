use super::snapshots::symbol_key;
use super::{egui, Explorer, View};
#[cfg(test)]
use firmware_analysis_core::Section;
use firmware_analysis_core::{Analysis, Usage};
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Metric {
    Flash,
    Ram,
    #[cfg(test)]
    All,
}
impl Metric {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Flash => "Flash",
            Self::Ram => "RAM",
            #[cfg(test)]
            Self::All => "All sections",
        }
    }
    pub(super) fn value(self, usage: Usage) -> u64 {
        match self {
            Self::Flash => usage.flash,
            Self::Ram => usage.ram,
            #[cfg(test)]
            Self::All => usage.flash.max(usage.ram),
        }
    }
    #[cfg(test)]
    pub(super) fn section_size(self, s: &Section) -> u64 {
        if self == Self::All {
            s.size
        } else {
            self.value(s.usage)
        }
    }
}
pub(super) const MUTED: egui::Color32 = egui::Color32::from_rgb(164, 185, 213);

pub(super) fn clipped_text(
    ui: &egui::Ui,
    pos: egui::Pos2,
    text: &str,
    width: f32,
    size: f32,
    color: egui::Color32,
) {
    let mut job = egui::text::LayoutJob::simple(
        text.into(),
        egui::FontId::proportional(size),
        color,
        width.max(1.0),
    );
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    let galley = ui.painter().layout_job(job);
    ui.painter().galley(pos, galley, color);
}

pub(super) fn right_text(
    ui: &egui::Ui,
    top_right: egui::Pos2,
    text: &str,
    width: f32,
    size: f32,
    color: egui::Color32,
) {
    let mut job = egui::text::LayoutJob::simple(
        text.into(),
        egui::FontId::proportional(size),
        color,
        width.max(1.0),
    );
    job.wrap.max_rows = 1;
    let galley = ui.painter().layout_job(job);
    ui.painter()
        .galley(top_right - egui::vec2(galley.size().x, 0.0), galley, color);
}

pub(super) fn occupancy_bar(
    ui: &mut egui::Ui,
    used: u64,
    capacity: u64,
    kind: firmware_analysis_core::MemoryKind,
) -> egui::Response {
    let (bar, response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 14.0), egui::Sense::hover());
    ui.painter()
        .rect_filled(bar, 3.0, egui::Color32::from_rgb(45, 60, 79));
    let fraction = used as f64 / capacity.max(1) as f64;
    let color = if fraction > 1.0 {
        egui::Color32::LIGHT_RED
    } else {
        match kind {
            firmware_analysis_core::MemoryKind::Flash => super::views::ACCENT,
            firmware_analysis_core::MemoryKind::Ram => egui::Color32::from_rgb(76, 156, 250),
        }
    };
    ui.painter().rect_filled(
        egui::Rect::from_min_size(
            bar.min,
            egui::vec2(bar.width() * fraction.clamp(0.0, 1.0) as f32, bar.height()),
        ),
        3.0,
        color,
    );
    ui.painter().text(
        bar.center(),
        egui::Align2::CENTER_CENTER,
        format!("{:.1}%", fraction * 100.0),
        egui::FontId::proportional(12.0),
        egui::Color32::WHITE,
    );
    response
}

fn compact_card(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    title: &str,
    contents: impl FnOnce(&mut egui::Ui),
) {
    ui.painter().rect(
        rect,
        7.0,
        egui::Color32::from_rgb(16, 25, 35),
        egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(31, 45, 60)),
    );
    ui.allocate_new_ui(egui::UiBuilder::new().max_rect(rect.shrink(14.0)), |ui| {
        ui.set_clip_rect(rect.shrink(10.0).intersect(ui.clip_rect()));
        ui.add(egui::Label::new(egui::RichText::new(title).size(16.0).strong()).wrap());
        ui.add_space(8.0);
        contents(ui);
    });
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
        if matches!(self.view, View::Overview) {
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
        self.visible_rows = 0;
        let (area, _) = ui.allocate_exact_size(ui.available_size(), egui::Sense::hover());
        let gap = 12.0;
        let top_height = (area.height() * 0.43).min(270.0);
        let top_width = (area.width() - gap * 3.0) / 4.0;
        for (index, title) in [
            "Binary information",
            "Memory usage",
            "Section type distribution",
            "Section type distribution",
        ]
        .into_iter()
        .enumerate()
        {
            let rect = egui::Rect::from_min_size(
                area.min + egui::vec2(index as f32 * (top_width + gap), 0.0),
                egui::vec2(top_width, top_height),
            );
            compact_card(ui, rect, title, |ui| match index {
                0 => self.compact_binary(ui, a),
                1 => self.compact_memory(ui, a),
                2 => self.section_distribution(ui, a, Metric::Flash),
                _ => self.section_distribution(ui, a, Metric::Ram),
            });
        }
        let bottom_width = (area.width() - gap) / 2.0;
        let bottom_height = (area.height() - top_height - gap).max(0.0);
        for (index, title) in ["Top functions by size", "Top files by size"]
            .into_iter()
            .enumerate()
        {
            let rect = egui::Rect::from_min_size(
                area.min + egui::vec2(index as f32 * (bottom_width + gap), top_height + gap),
                egui::vec2(bottom_width, bottom_height),
            );
            compact_card(ui, rect, title, |ui| {
                self.compact_ranking(ui, a, index == 0);
            });
        }
    }

    fn compact_binary(&mut self, ui: &mut egui::Ui, a: &Analysis) {
        let rows = [
            (
                "Format",
                format!(
                    "ELF ({}-bit, {} endian)",
                    a.metadata.bitness, a.metadata.endianness
                ),
            ),
            ("Architecture", a.metadata.architecture.clone()),
            (
                "Entry point",
                self.snapshot_address("metadata", "", "entry_point", a.metadata.entry_point),
            ),
            (
                "ELF file size",
                self.snapshot_bytes("metadata", "", "file_size", a.metadata.file_size),
            ),
            (
                "Debug info",
                if a.metadata.has_dwarf {
                    "DWARF present"
                } else {
                    "Unavailable"
                }
                .into(),
            ),
        ];
        let label_width = (ui.available_width() * 0.31).clamp(76.0, 116.0);
        for (label, value) in rows {
            let (rect, response) = ui
                .allocate_exact_size(egui::vec2(ui.available_width(), 22.0), egui::Sense::hover());
            clipped_text(ui, rect.min, label, label_width, 14.0, MUTED);
            clipped_text(
                ui,
                rect.min + egui::vec2(label_width, 0.0),
                &value,
                (rect.width() - label_width).max(1.0),
                14.0,
                ui.visuals().text_color(),
            );
            response.on_hover_text(format!("{label}: {value}"));
        }
    }

    pub(super) fn select_overview_region(&mut self, a: &Analysis, slot: usize, name: String) {
        self.overview_regions[slot] = Some(name);
        if let Some(build) = &self.build {
            self.build_settings
                .entry(build.root.clone())
                .or_default()
                .overview_regions
                .insert(
                    std::path::PathBuf::from(&a.path),
                    self.overview_regions.clone(),
                );
            self.persist_preferences();
        }
    }

    pub(super) fn restore_overview_regions(&mut self) {
        self.overview_regions = self
            .build
            .as_ref()
            .zip(self.analysis.as_ref())
            .and_then(|(build, a)| {
                self.build_settings
                    .get(&build.root)?
                    .overview_regions
                    .get(std::path::Path::new(&a.path))
            })
            .cloned()
            .unwrap_or_default();
    }

    pub(super) fn overview_region(
        &self,
        a: &Analysis,
        slot: usize,
        kind: firmware_analysis_core::MemoryKind,
    ) -> Option<usize> {
        let candidates = || {
            a.options
                .regions
                .iter()
                .enumerate()
                .filter(|(_, r)| r.kind == kind)
        };
        if let Some(name) = &self.overview_regions[slot] {
            if let Some((index, _)) = candidates().find(|(_, r)| &r.name == name) {
                return Some(index);
            }
        }
        // Compare exact integer ratios; ties keep the linker layout order.
        candidates()
            .max_by(|(i, left), (j, right)| {
                (u128::from(self.region_cache[*i].used) * u128::from(right.size.max(1)))
                    .cmp(&(u128::from(self.region_cache[*j].used) * u128::from(left.size.max(1))))
                    .then_with(|| j.cmp(i))
            })
            .map(|(index, _)| index)
    }

    pub(super) fn compact_memory(&mut self, ui: &mut egui::Ui, a: &Analysis) {
        use firmware_analysis_core::MemoryKind;
        let block_height = ((ui.available_height() - 28.0) / 2.0).clamp(54.0, 74.0);
        for (slot, (kind, label, total, field, help)) in [
            (
                MemoryKind::Flash,
                "Flash payload",
                a.totals.flash,
                "flash",
                super::views::FLASH_HELP,
            ),
            (
                MemoryKind::Ram,
                "Static RAM",
                a.totals.ram,
                "ram",
                super::views::RAM_HELP,
            ),
        ]
        .into_iter()
        .enumerate()
        {
            let (rect, _) = ui.allocate_exact_size(
                egui::vec2(ui.available_width(), block_height),
                egui::Sense::hover(),
            );
            ui.allocate_new_ui(egui::UiBuilder::new().max_rect(rect), |ui| {
                let total = self.snapshot_bytes("totals", "", field, total);
                let tip = format!("{help}\n{label}: {total}");
                let Some(mut index) = self.overview_region(a, slot, kind) else {
                    ui.weak("Capacity unknown").on_hover_text(tip);
                    return;
                };
                let selected = a.options.regions[index].name.clone();
                egui::ComboBox::from_id_salt(("overview_region", slot))
                    .width(ui.available_width() - 8.0)
                    .selected_text(selected)
                    .show_ui(ui, |ui| {
                        for (i, region) in a.options.regions.iter().enumerate().filter(|(_, r)| r.kind == kind) {
                            if ui.selectable_value(&mut index, i, &region.name).clicked() {
                                self.select_overview_region(a, slot, region.name.clone());
                            }
                        }
                    }).response.on_hover_text(tip);
                let region = &a.options.regions[index];
                let usage = &self.region_cache[index];
                occupancy_bar(ui, usage.used, region.size, kind).on_hover_text(format!(
                    "{} physical occupancy: {} / {}; {} free\nFree space excludes runtime heap and additional stack demand.",
                    region.name, firmware_analysis_core::format_bytes(usage.used), firmware_analysis_core::format_bytes(region.size), firmware_analysis_core::format_bytes(usage.free)
                ));
                let detail = format!("{} / {} · {} · {} free",
                    self.snapshot_bytes("region", &region.name, "used", usage.used),
                    self.snapshot_bytes("region", &region.name, "size", region.size),
                    self.snapshot_region_percentage(&region.name, usage.used, region.size),
                    self.snapshot_bytes("region", &region.name, "free", usage.free));
                let (detail_rect, response) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 16.0), egui::Sense::hover());
                let used_capacity = format!("{} / {}", self.snapshot_bytes("region", &region.name, "used", usage.used), self.snapshot_bytes("region", &region.name, "size", region.size));
                let free = format!("{} free", self.snapshot_bytes("region", &region.name, "free", usage.free));
                clipped_text(ui, detail_rect.min, &used_capacity, detail_rect.width() * 0.65, 12.0, MUTED);
                right_text(ui, detail_rect.right_top(), &free, detail_rect.width() * 0.35, 12.0, MUTED);
                response.on_hover_text(detail);
            });
        }
        if ui
            .link(format!("View all regions ({})", a.options.regions.len()))
            .clicked()
        {
            self.change_view(View::MemoryMap);
        }
    }

    fn compact_ranking(&mut self, ui: &mut egui::Ui, a: &Analysis, functions: bool) {
        let paths = self.source_paths(a);
        let mut indices: Vec<_> = if functions {
            (0..a.symbols.len())
                .filter(|&i| {
                    a.symbols[i].kind == "Function"
                        && a.symbols[i].usage.flash > 0
                        && self.diff_visible_in_view(
                            "symbol",
                            &symbol_key(&a.symbols[i]),
                            View::Overview,
                        )
                })
                .collect()
        } else {
            (0..a.files.len())
                .filter(|&i| {
                    a.files[i].path != "[unattributed]"
                        && a.files[i].usage.flash > 0
                        && self.diff_visible_in_view("file", &a.files[i].path, View::Overview)
                })
                .collect()
        };
        let flash = |i: usize| {
            if functions {
                a.symbols[i].usage.flash
            } else {
                a.files[i].usage.flash
            }
        };
        indices.sort_by_key(|&i| (std::cmp::Reverse(flash(i)), i));
        let maximum = indices.first().map_or(1, |&i| flash(i)).max(1);
        ui.horizontal(|ui| {
            ui.weak(if functions {
                "Function · unique Flash bytes"
            } else {
                "Source file · unique Flash bytes"
            });
        });
        let count = ((ui.available_height() - 32.0) / 29.0).floor().max(0.0) as usize;
        let limit = count.min(10);
        self.visible_rows += indices.len().min(limit);
        if indices.is_empty() {
            ui.weak("No matching attributed entries.");
        }
        for (rank, &index) in indices.iter().take(limit).enumerate() {
            let (name, size, tip) = if functions {
                let symbol = &a.symbols[index];
                (
                    symbol.demangled_name.clone(),
                    self.snapshot_bytes(
                        "symbol",
                        &symbol_key(symbol),
                        "usage.flash",
                        symbol.usage.flash,
                    ),
                    format!(
                        "{}\nAddress: {:#x}\nELF size: {}",
                        symbol.demangled_name,
                        symbol.normalized_address,
                        firmware_analysis_core::format_bytes(symbol.size)
                    ),
                )
            } else {
                let file = &a.files[index];
                (
                    paths.short(&file.path),
                    self.snapshot_bytes("file", &file.path, "usage.flash", file.usage.flash),
                    format!(
                        "{}\nFlash: {} · RAM: {}",
                        paths.full(&file.path),
                        firmware_analysis_core::format_bytes(file.usage.flash),
                        firmware_analysis_core::format_bytes(file.usage.ram)
                    ),
                )
            };
            let (rect, response) = ui
                .allocate_exact_size(egui::vec2(ui.available_width(), 23.0), egui::Sense::click());
            if response.hovered() {
                ui.painter()
                    .rect_filled(rect, 4.0, egui::Color32::from_rgb(23, 40, 49));
            }
            let font = egui::FontId::proportional(14.0);
            let text_color = ui.visuals().text_color();
            ui.painter().text(
                rect.left_center() + egui::vec2(2.0, 0.0),
                egui::Align2::LEFT_CENTER,
                format!("{}", rank + 1),
                font.clone(),
                egui::Color32::from_rgb(147, 171, 201),
            );
            let size_width = 115.0_f32.min(rect.width() * 0.35);
            let bar_width = if rect.width() >= 430.0 { 70.0 } else { 0.0 };
            let name_width = (rect.width() - size_width - bar_width - 38.0).max(1.0);
            let mut job =
                egui::text::LayoutJob::simple(name.clone(), font.clone(), text_color, name_width);
            job.wrap.max_rows = 1;
            job.wrap.break_anywhere = true;
            let galley = ui.painter().layout_job(job);
            ui.painter().galley(
                rect.left_center() + egui::vec2(26.0, -galley.size().y * 0.5),
                galley,
                text_color,
            );
            if bar_width > 0.0 {
                let bar = egui::Rect::from_min_size(
                    egui::pos2(
                        rect.right() - size_width - bar_width - 8.0,
                        rect.center().y - 6.0,
                    ),
                    egui::vec2(bar_width, 12.0),
                );
                ui.painter()
                    .rect_filled(bar, 3.0, egui::Color32::from_rgb(33, 47, 62));
                ui.painter().rect_filled(
                    egui::Rect::from_min_size(
                        bar.min,
                        egui::vec2(
                            bar.width() * (flash(index) as f64 / maximum as f64) as f32,
                            bar.height(),
                        ),
                    ),
                    3.0,
                    super::views::ACCENT,
                );
            }
            let mut job = egui::text::LayoutJob::simple(size, font, text_color, size_width);
            job.wrap.max_rows = 1;
            let galley = ui.painter().layout_job(job);
            ui.painter().galley(
                egui::pos2(
                    rect.right() - galley.size().x,
                    rect.center().y - galley.size().y * 0.5,
                ),
                galley,
                text_color,
            );
            let clicked = response
                .on_hover_text(tip)
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .clicked();
            if clicked {
                if functions {
                    self.change_view(View::Symbols);
                    self.search = name;
                    self.selected_file = None;
                    self.kind_filter = "Function".into();
                } else {
                    self.show_file_symbols(a.files[index].path.clone());
                }
            }
        }
        if ui
            .link(if functions {
                "View all functions"
            } else {
                "View all files"
            })
            .clicked()
        {
            self.change_view(if functions {
                View::Symbols
            } else {
                View::Files
            });
            self.search.clear();
            self.selected_file = None;
            if functions {
                self.kind_filter = "Function".into();
            }
        }
    }
}
