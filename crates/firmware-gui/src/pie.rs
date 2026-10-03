use super::Explorer;
use eframe::egui;
use firmware_analysis_core::{format_bytes as bytes, Analysis};
use std::f32::consts::{FRAC_PI_2, TAU};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum UnitKey {
    Dwarf(String),
    Elf(String),
    Other,
}

impl UnitKey {
    fn label(&self) -> &str {
        match self {
            Self::Dwarf(path) | Self::Elf(path) => path,
            Self::Other => "Other / unconnected",
        }
    }
}

fn unit_key(s: &firmware_analysis_core::Symbol) -> UnitKey {
    if let Some(path) = &s.dwarf_compilation_unit {
        UnitKey::Dwarf(path.clone())
    } else if let Some(label) = &s.compilation_unit {
        UnitKey::Elf(label.clone())
    } else {
        UnitKey::Other
    }
}

#[derive(Clone)]
enum Target {
    Section(usize),
    Unit(UnitKey),
}

struct Slice {
    name: String,
    size: u64,
    target: Option<Target>,
    tip: String,
}

fn slices(a: &Analysis, section: Option<usize>, unit: Option<&UnitKey>) -> Vec<Slice> {
    let mut slices = Vec::new();
    if let Some(section) = section.and_then(|index| a.sections.iter().find(|s| s.index == index)) {
        let symbols: Vec<_> = a
            .symbols
            .iter()
            .filter(|s| s.section_index == section.index)
            .collect();
        let remaining = section
            .size
            .saturating_sub(symbols.iter().map(|s| s.usage.flash.max(s.usage.ram)).sum());
        if let Some(unit) = unit {
            for s in symbols.iter().filter(|s| unit_key(s) == *unit) {
                slices.push(Slice {
                    name: s.demangled_name.clone(),
                    size: s.usage.flash.max(s.usage.ram),
                    target: None,
                    tip: format!(
                        "{} ({})\nAddress: {:#x}\nELF size: {}\n{}",
                        s.name,
                        s.kind,
                        s.normalized_address,
                        bytes(s.size),
                        s.source_file.as_deref().unwrap_or("Unknown source")
                    ),
                });
            }
            if *unit == UnitKey::Other && remaining > 0 {
                slices.push(Slice {
                    name: "[padding / unknown contents]".into(),
                    size: remaining,
                    target: None,
                    tip: "Bytes not covered by sized symbols, including padding and reservations."
                        .into(),
                });
            }
        } else {
            let mut units = std::collections::BTreeMap::<UnitKey, u64>::new();
            for s in symbols {
                *units.entry(unit_key(s)).or_default() += s.usage.flash.max(s.usage.ram);
            }
            if remaining > 0 {
                *units.entry(UnitKey::Other).or_default() += remaining;
            }
            for (key, size) in units {
                slices.push(Slice {
                    name: key.label().to_owned(), size,
                    tip: match &key {
                        UnitKey::Dwarf(_) => "DWARF compilation unit. Click to inspect functions and data symbols.",
                        UnitKey::Elf(_) => "ELF compilation-unit label (not a proven object path). Click to inspect symbols.",
                        UnitKey::Other => "Symbols without a known compilation unit, padding and reservations.",
                    }.into(),
                    target: Some(Target::Unit(key)),
                });
            }
        }
    } else {
        slices = a
            .sections
            .iter()
            .filter(|s| s.allocated)
            .map(|s| Slice {
                name: s.name.clone(),
                size: s.size,
                target: Some(Target::Section(s.index)),
                tip: format!(
                    "Flash: {} / RAM: {}\n{}",
                    bytes(s.usage.flash),
                    bytes(s.usage.ram),
                    s.evidence
                ),
            })
            .collect();
    }
    slices.sort_by(|a, b| b.size.cmp(&a.size).then_with(|| a.name.cmp(&b.name)));
    slices
}

fn color(index: usize) -> egui::Color32 {
    egui::ecolor::Hsva::new((index as f32 * 0.618_034) % 1.0, 0.78, 0.70, 1.0).into()
}

impl Explorer {
    pub(super) fn overview_back(&mut self) {
        if self.overview_unit.take().is_none() {
            self.overview_section = None;
        }
    }

    pub(super) fn overview_pie(&mut self, ui: &mut egui::Ui, a: &Analysis) {
        ui.horizontal(|ui| {
            if self.overview_section.is_some() && ui.button("Back").clicked() {
                self.overview_back();
            }
            let section = self
                .overview_section
                .and_then(|index| a.sections.iter().find(|s| s.index == index));
            ui.strong(section.map_or("Allocated sections", |s| s.name.as_str()));
            if let Some(unit) = &self.overview_unit {
                ui.label(" / ");
                ui.strong(unit.label());
            }
        });
        ui.weak(if self.overview_unit.is_some() {
            "Functions and data symbols by unique bytes. Aliases and zero-sized labels remain in the list. Function arguments have no separate section sizes."
        } else if self.overview_section.is_some() {
            "Compilation units by unique bytes in this section. Click to explore; mouse Back moves up one level."
        } else {
            "Section sizes, counted once. Click a slice or its legend to explore its contents."
        });
        let items = slices(a, self.overview_section, self.overview_unit.as_ref());
        self.visible_rows = items.len();
        let total: u64 = items.iter().map(|s| s.size).sum();
        if total == 0 {
            ui.label("No sized contents available.");
        }
        let mut selected = None;
        egui::ScrollArea::vertical()
            .id_salt("overview_pie")
            .show(ui, |ui| {
                ui.horizontal_top(|ui| {
                    let side = (ui.available_width() * 0.45).min(360.0);
                    let (rect, response) =
                        ui.allocate_exact_size(egui::vec2(side, side), egui::Sense::click());
                    let center = rect.center();
                    let radius = side * 0.46;
                    let project = |theta: f32| {
                        center + egui::vec2(theta.cos() * radius, theta.sin() * radius)
                    };
                    let pointer = response.hover_pos().map(|p| p - center);
                    let mut angle = 0.0;
                    let mut rim = Vec::new();
                    let mut dividers = Vec::new();
                    let mut hovered_slice = None;
                    let mut slice_meshes = Vec::new();
                    for (index, item) in items.iter().enumerate().filter(|(_, s)| s.size > 0) {
                        let sweep = item.size as f32 / total as f32 * TAU;
                        let hovered = pointer.is_some_and(|p| {
                            let theta = (p.y.atan2(p.x) + FRAC_PI_2).rem_euclid(TAU);
                            p.length() <= radius && theta >= angle && theta < angle + sweep
                        });
                        let steps = ((sweep / TAU * 180.0).ceil() as usize).max(1);
                        let mut mesh = egui::Mesh::default();
                        let fill = color(index);
                        mesh.colored_vertex(center, fill);
                        dividers.push(project(angle - FRAC_PI_2));
                        for step in 0..=steps {
                            let theta = angle + sweep * step as f32 / steps as f32 - FRAC_PI_2;
                            let point = project(theta);
                            mesh.colored_vertex(point, fill);
                            if step < steps {
                                rim.push(point);
                            }
                        }
                        for step in 0..steps {
                            mesh.add_triangle(0, step as u32 + 1, step as u32 + 2);
                        }
                        // Reserve the fill's paint order, then resolve legend hover below.
                        let shape = ui.painter().add(egui::Shape::Noop);
                        slice_meshes.push((index, shape, mesh));
                        if hovered {
                            hovered_slice = Some(index);
                            response.clone().on_hover_text(format!(
                                "{}: {} ({:.2}%)\n{}",
                                item.name,
                                bytes(item.size),
                                item.size as f64 / total as f64 * 100.0,
                                item.tip
                            ));
                            if item.target.is_some() {
                                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                            }
                            if response.clicked() {
                                selected = item.target.clone();
                            }
                        }
                        angle += sweep;
                    }
                    // Separate radial segments avoid acute stroke joins at the center.
                    // Paint each boundary once, after all fills, so it stays visible.
                    let stroke = egui::Stroke::new(1.0_f32, egui::Color32::BLACK);
                    if dividers.len() > 1 {
                        for end in dividers {
                            ui.painter().line_segment([center, end], stroke);
                        }
                    }
                    if !rim.is_empty() {
                        ui.painter().add(egui::Shape::closed_line(rim, stroke));
                    }
                    let mut hovered_legend = None;
                    ui.vertical(|ui| {
                        ui.label(format!("Total: {}", bytes(total)));
                        for (index, item) in items.iter().enumerate() {
                            ui.horizontal(|ui| {
                                let swatch = ui.colored_label(color(index), "●");
                                let label = format!(
                                    "{} — {} ({:.2}%)",
                                    item.name,
                                    bytes(item.size),
                                    item.size as f64 / total.max(1) as f64 * 100.0
                                );
                                let label = egui::RichText::new(label);
                                let label = if hovered_slice == Some(index) {
                                    label.underline()
                                } else {
                                    label
                                };
                                let entry = if item.target.is_some() {
                                    ui.link(label)
                                } else {
                                    ui.label(label)
                                }
                                .on_hover_text(&item.tip);
                                if entry.clicked() {
                                    selected = item.target.clone();
                                }
                                if entry.hovered() || swatch.hovered() {
                                    hovered_legend = Some(index);
                                }
                            });
                        }
                    });
                    // Resolve both directions in this frame without storing stale hover state.
                    let highlighted = hovered_slice.or(hovered_legend);
                    for (index, shape, mut mesh) in slice_meshes {
                        if highlighted == Some(index) {
                            let base = color(index);
                            let fill = egui::Color32::from_rgb(
                                base.r().saturating_add(32),
                                base.g().saturating_add(32),
                                base.b().saturating_add(32),
                            );
                            for vertex in &mut mesh.vertices {
                                vertex.color = fill;
                            }
                        }
                        ui.painter().set(shape, egui::Shape::mesh(mesh));
                    }
                });
            });
        if let Some(target) = selected {
            match target {
                Target::Section(section) => {
                    self.overview_section = Some(section);
                    self.overview_unit = None;
                }
                Target::Unit(unit) => self.overview_unit = Some(unit),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_drilldown_reconciles_and_preserves_all_symbols() {
        for fixture in ["cortex-m.elf", "cortex-m-stripped.elf"] {
            let a = firmware_analysis_core::analyze_path(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../fixtures")
                    .join(fixture),
                &Default::default(),
            )
            .unwrap();
            for section in a.sections.iter().filter(|s| s.allocated) {
                let units = slices(&a, Some(section.index), None);
                assert_eq!(units.iter().map(|s| s.size).sum::<u64>(), section.size);
                let mut symbol_count = 0;
                for unit in units {
                    let Some(Target::Unit(key)) = unit.target else {
                        panic!("Expected unit target")
                    };
                    let contents = slices(&a, Some(section.index), Some(&key));
                    assert_eq!(contents.iter().map(|s| s.size).sum::<u64>(), unit.size);
                    symbol_count += contents
                        .iter()
                        .filter(|s| s.name != "[padding / unknown contents]")
                        .count();
                }
                assert_eq!(
                    symbol_count,
                    a.symbols
                        .iter()
                        .filter(|s| s.section_index == section.index)
                        .count()
                );
            }
        }
    }

    #[test]
    fn unknown_ownership_is_not_inferred_from_source_filename() {
        let mut a = firmware_analysis_core::analyze_path(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/cortex-m.elf"),
            &Default::default(),
        )
        .unwrap();
        let s = a.symbols.iter_mut().find(|s| s.size > 0).unwrap();
        s.compilation_unit = None;
        s.dwarf_compilation_unit = None;
        s.source_file = Some("header.h".into());
        assert_eq!(unit_key(s), UnitKey::Other);
        s.compilation_unit = Some("legacy.c".into());
        assert_eq!(unit_key(s), UnitKey::Elf("legacy.c".into()));
        s.dwarf_compilation_unit = Some("/src/main.c".into());
        assert_eq!(unit_key(s), UnitKey::Dwarf("/src/main.c".into()));
    }
}
