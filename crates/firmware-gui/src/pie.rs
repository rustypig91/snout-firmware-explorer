use super::display::display_path;
use super::Explorer;
use eframe::egui;
use firmware_analysis_core::{format_bytes as bytes, Analysis};

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

#[cfg(test)]
fn slices(a: &Analysis, section: Option<usize>, unit: Option<&UnitKey>) -> Vec<Slice> {
    metric_slices(a, section, unit, super::overview::Metric::All)
}
fn metric_slices(
    a: &Analysis,
    section: Option<usize>,
    unit: Option<&UnitKey>,
    metric: super::overview::Metric,
) -> Vec<Slice> {
    metric_slices_for_display(a, section, unit, metric, None)
}
fn metric_slices_for_display(
    a: &Analysis,
    section: Option<usize>,
    unit: Option<&UnitKey>,
    metric: super::overview::Metric,
    app: Option<&Explorer>,
) -> Vec<Slice> {
    let mut slices = Vec::new();
    if let Some(section) = section.and_then(|index| a.sections.iter().find(|s| s.index == index)) {
        let symbols: Vec<_> = a
            .symbols
            .iter()
            .filter(|s| s.section_index == section.index)
            .collect();
        let remaining = metric
            .section_size(section)
            .saturating_sub(symbols.iter().map(|s| metric.value(s.usage)).sum());
        if let Some(unit) = unit {
            for s in symbols.iter().filter(|s| unit_key(s) == *unit) {
                slices.push(Slice {
                    name: s.demangled_name.clone(),
                    size: metric.value(s.usage),
                    target: None,
                    tip: format!(
                        "{} ({})\nAddress: {}\nELF size: {}\n{}",
                        s.name,
                        s.kind,
                        app.map(|app| app.snapshot_address(
                            "symbol",
                            &super::snapshots::symbol_key(s),
                            "normalized_address",
                            s.normalized_address
                        ))
                        .unwrap_or_else(|| format!("{:#x}", s.normalized_address)),
                        app.map(|app| app.snapshot_bytes(
                            "symbol",
                            &super::snapshots::symbol_key(s),
                            "size",
                            s.size
                        ))
                        .unwrap_or_else(|| bytes(s.size)),
                        display_path(s.source_file.as_deref().unwrap_or("Unknown source"))
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
                *units.entry(unit_key(s)).or_default() += metric.value(s.usage);
            }
            if remaining > 0 {
                *units.entry(UnitKey::Other).or_default() += remaining;
            }
            for (key, size) in units {
                slices.push(Slice {
                    name: display_path(key.label()).into_owned(), size,
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
            .filter(|s| {
                s.allocated
                    && (metric == super::overview::Metric::All || metric.section_size(s) > 0)
            })
            .map(|s| Slice {
                name: s.name.clone(),
                size: metric.section_size(s),
                target: Some(Target::Section(s.index)),
                tip: format!(
                    "Flash: {} / RAM: {}\n{}",
                    app.map(|app| app.snapshot_bytes(
                        "section",
                        &s.name,
                        "usage.flash",
                        s.usage.flash
                    ))
                    .unwrap_or_else(|| bytes(s.usage.flash)),
                    app.map(|app| app.snapshot_bytes("section", &s.name, "usage.ram", s.usage.ram))
                        .unwrap_or_else(|| bytes(s.usage.ram)),
                    s.evidence
                ),
            })
            .collect();
    }
    slices.sort_by(|a, b| b.size.cmp(&a.size).then_with(|| a.name.cmp(&b.name)));
    slices
}

pub(super) fn color(name: &str) -> egui::Color32 {
    let hash = name.bytes().fold(2166136261u32, |h, b| {
        (h ^ u32::from(b)).wrapping_mul(16777619)
    });
    egui::ecolor::Hsva::new((hash % 360) as f32 / 360.0, 0.55, 0.85, 1.0).into()
}
impl Explorer {
    pub(super) fn overview_back(&mut self) {
        if self.overview_unit.take().is_none() {
            self.overview_section = None;
        }
    }
    pub(super) fn overview_pie(&mut self, ui: &mut egui::Ui, a: &Analysis) {
        ui.horizontal_wrapped(|ui| {
            if self.overview_section.is_some() && ui.button("Back").clicked() {
                self.overview_back();
            }
            if ui.link("Sections").clicked() {
                self.overview_section = None;
                self.overview_unit = None;
            }
            if let Some(section) = self
                .overview_section
                .and_then(|i| a.sections.iter().find(|s| s.index == i))
            {
                ui.label(format!("/ {}", section.name));
            }
            if let Some(unit) = &self.overview_unit {
                ui.label(format!("/ {}", display_path(unit.label())));
            }
        });
        let items = metric_slices_for_display(
            a,
            self.overview_section,
            self.overview_unit.as_ref(),
            self.overview_metric,
            Some(self),
        );
        let baseline_items = self.snapshot_analysis().map(|old| {
            let section = self
                .overview_section
                .and_then(|i| a.sections.iter().find(|s| s.index == i))
                .and_then(|s| old.sections.iter().find(|o| o.name == s.name))
                .map(|s| s.index);
            // A section absent from the baseline has no previous slices.
            if self.overview_section.is_some() && section.is_none() {
                Vec::new()
            } else {
                metric_slices(
                    old,
                    section,
                    self.overview_unit.as_ref(),
                    self.overview_metric,
                )
            }
        });
        self.visible_rows = items.len();
        let total: u64 = items.iter().map(|s| s.size).sum();
        ui.label(format!(
            "{}: {}",
            self.overview_metric.label(),
            self.snapshot_difference(
                total,
                baseline_items
                    .as_ref()
                    .map(|items| items.iter().map(|s| s.size).sum())
            )
        ));
        ui.weak("Select a row to explore")
            .on_hover_text("Aliases share unique bytes; zero-sized labels remain listed.");
        let mut selected = None;
        let baseline_total: u64 = baseline_items
            .as_ref()
            .map_or(0, |items| items.iter().map(|s| s.size).sum());
        for item in &items {
            let fraction = if total == 0 {
                0.0
            } else {
                item.size as f32 / total as f32
            };
            let label = format!(
                "{} - {} ({})",
                item.name,
                self.snapshot_difference(
                    item.size,
                    baseline_items.as_ref().map(|items| items
                        .iter()
                        .filter(|old| old.name == item.name)
                        .map(|s| s.size)
                        .sum())
                ),
                self.snapshot_percentage(
                    fraction as f64 * 100.0,
                    baseline_items.as_ref().map(|items| {
                        let size: u64 = items
                            .iter()
                            .filter(|old| old.name == item.name)
                            .map(|s| s.size)
                            .sum();
                        if baseline_total == 0 {
                            0.0
                        } else {
                            size as f64 * 100.0 / baseline_total as f64
                        }
                    })
                )
            );
            if ui
                .add(
                    egui::Button::new(label)
                        .wrap()
                        .min_size(egui::vec2(ui.available_width(), 24.0)),
                )
                .on_hover_text(&item.tip)
                .clicked()
            {
                selected = item.target.clone();
            }
            ui.add(
                egui::ProgressBar::new(fraction)
                    .fill(color(&item.name))
                    .desired_height(5.0),
            );
        }
        if items.is_empty() {
            ui.label("No contents in this memory space.");
        }
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
                    .join("../../fixtures/build")
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
    fn memory_modes_reconcile_through_every_drilldown() {
        use super::super::overview::Metric;
        for fixture in [
            "cortex-m.elf",
            "cortex-m-grown.elf",
            "cortex-m-stripped.elf",
        ] {
            let a = firmware_analysis_core::analyze_path(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../fixtures/build")
                    .join(fixture),
                &Default::default(),
            )
            .unwrap();
            for metric in [Metric::Flash, Metric::Ram] {
                let sections = metric_slices(&a, None, None, metric);
                assert_eq!(
                    sections.iter().map(|s| s.size).sum::<u64>(),
                    metric.value(a.totals)
                );
                for section in sections {
                    let Some(Target::Section(index)) = section.target else {
                        panic!("section");
                    };
                    let units = metric_slices(&a, Some(index), None, metric);
                    assert_eq!(units.iter().map(|s| s.size).sum::<u64>(), section.size);
                    for unit in units {
                        let Some(Target::Unit(key)) = unit.target else {
                            panic!("unit");
                        };
                        assert_eq!(
                            metric_slices(&a, Some(index), Some(&key), metric)
                                .iter()
                                .map(|s| s.size)
                                .sum::<u64>(),
                            unit.size
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn unknown_ownership_is_not_inferred_from_source_filename() {
        let mut a = firmware_analysis_core::analyze_path(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../fixtures/build/cortex-m.elf"),
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
