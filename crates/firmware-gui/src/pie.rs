use super::display::SourcePaths;
use super::Explorer;
use eframe::egui;
use firmware_analysis_core::{format_bytes as bytes, Analysis};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
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

#[derive(Clone, PartialEq, Eq, Hash)]
enum SliceIdentity {
    Section(String),
    Unit(UnitKey),
    Symbol(String),
    Padding,
}

#[derive(Clone)]
struct Slice {
    identity: SliceIdentity,
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
    let paths = app
        .map(|app| app.source_paths(a))
        .unwrap_or_else(|| std::rc::Rc::new(SourcePaths::new(a, None, None)));
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
                    identity: SliceIdentity::Symbol(super::snapshots::symbol_key(s)),
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
                        paths.full(s.source_file.as_deref().unwrap_or("Unknown source"))
                    ),
                });
            }
            if *unit == UnitKey::Other && remaining > 0 {
                slices.push(Slice {
                    identity: SliceIdentity::Padding,
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
                    identity: SliceIdentity::Unit(key.clone()),
                    name: paths.short(key.label()), size,
                    tip: match &key {
                        UnitKey::Dwarf(_) => "DWARF compilation unit. Click to inspect functions and data symbols.",
                        UnitKey::Elf(_) => "ELF compilation-unit label (not a proven object path). Click to inspect symbols.",
                        UnitKey::Other => "Symbols without a known compilation unit, padding and reservations.",
                    }.to_owned() + "\n" + &paths.full(key.label()),
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
                    && (metric == super::overview::Metric::All
                        || metric.section_size(s) > 0
                        || app.is_some_and(|app| {
                            app.snapshot_removed("section", &s.name)
                                && app
                                    .snapshot_old(
                                        "section",
                                        &s.name,
                                        if metric == super::overview::Metric::Ram {
                                            "usage.ram"
                                        } else {
                                            "usage.flash"
                                        },
                                    )
                                    .is_some_and(|size| size > 0)
                        }))
            })
            .map(|s| Slice {
                identity: SliceIdentity::Section(s.name.clone()),
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

fn baseline_slices(
    current: &Analysis,
    baseline: &Analysis,
    section: Option<usize>,
    unit: Option<&UnitKey>,
    metric: super::overview::Metric,
) -> Result<Vec<Slice>, ()> {
    let section = if let Some(index) = section {
        let Some(selected) = current.sections.iter().find(|s| s.index == index) else {
            return Ok(Vec::new());
        };
        // A name must identify exactly one section in both builds. Otherwise
        // each current duplicate would compare against the same baseline bytes.
        if current
            .sections
            .iter()
            .filter(|s| s.name == selected.name)
            .count()
            != 1
        {
            return Err(());
        }
        let mut matches = baseline.sections.iter().filter(|s| s.name == selected.name);
        let Some(old) = matches.next() else {
            return Ok(Vec::new());
        };
        if matches.next().is_some() {
            return Err(());
        }
        Some(old.index)
    } else {
        None
    };
    Ok(metric_slices(baseline, section, unit, metric))
}

// A display label can be shared by different units or symbols. Only compare
// stable identities, and leave duplicate identities unknown rather than summing
// multiple baseline rows into the delta for one current row.
fn baseline_sizes(baseline: &[Slice]) -> std::collections::HashMap<SliceIdentity, Result<u64, ()>> {
    let mut sizes = std::collections::HashMap::with_capacity(baseline.len());
    for item in baseline {
        sizes
            .entry(item.identity.clone())
            .and_modify(|size| *size = Err(()))
            .or_insert(Ok(item.size));
    }
    sizes
}

#[cfg(test)]
fn baseline_size(item: &Slice, baseline: &[Slice]) -> Result<u64, ()> {
    baseline_sizes(baseline)
        .get(&item.identity)
        .copied()
        .unwrap_or(Ok(0))
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
    pub(super) fn overview_pie(&mut self, ui: &mut egui::Ui, current: &Analysis) {
        let display = self.baseline_display_analysis();
        let a = display.as_deref().unwrap_or(current);
        let paths = self.source_paths(a);
        if self.overview_section.is_some() {
            ui.horizontal_wrapped(|ui| {
                if self.overview_section.is_some() && ui.button("Back").clicked() {
                    self.overview_back();
                }
                if let Some(section) = self
                    .overview_section
                    .and_then(|i| a.sections.iter().find(|s| s.index == i))
                {
                    ui.label(&section.name);
                }
                if let Some(unit) = &self.overview_unit {
                    ui.label(format!("/ {}", paths.short(unit.label())))
                        .on_hover_text(paths.full(unit.label()));
                }
            });
        }
        let mut items = metric_slices_for_display(
            a,
            self.overview_section,
            self.overview_unit.as_ref(),
            self.overview_metric,
            Some(self),
        );
        let baseline = self.snapshot_analysis().map(|old| {
            baseline_slices(
                a,
                old,
                self.overview_section,
                self.overview_unit.as_ref(),
                self.overview_metric,
            )
        });
        let ambiguous_section = matches!(baseline, Some(Err(())));
        let baseline_items = baseline.as_ref().and_then(|items| items.as_ref().ok());
        // Presence is independent of byte size: zero-sized current labels are
        // still current, while baseline-only rows take no current chart space.
        let current_section = self
            .overview_section
            .and_then(|index| current.sections.iter().find(|s| s.index == index));
        let current_items: std::collections::HashSet<_> =
            if baseline.is_none() || self.overview_section.is_none() || current_section.is_none() {
                Default::default()
            } else {
                metric_slices(
                    current,
                    current_section.map(|s| s.index),
                    self.overview_unit.as_ref(),
                    self.overview_metric,
                )
                .into_iter()
                .map(|item| item.identity)
                .collect()
            };
        let baseline_sizes = baseline_items.map(|items| baseline_sizes(items));
        if let Some(baseline_items) = baseline_items {
            let mut present: std::collections::HashSet<_> =
                items.iter().map(|item| item.identity.clone()).collect();
            for old in baseline_items {
                if present.insert(old.identity.clone()) {
                    let mut item = old.clone();
                    item.size = 0;
                    if let SliceIdentity::Section(name) = &item.identity {
                        item.target = a
                            .sections
                            .iter()
                            .find(|s| &s.name == name)
                            .map(|s| Target::Section(s.index));
                    }
                    items.push(item);
                }
            }
        }
        let total: u64 = items.iter().map(|s| s.size).sum();
        if self.diffs_active() {
            // Keep the navigation path to changed symbols even when section
            // and unit totals are unchanged. Include baseline-only symbols
            // from the display analysis, whose section indexes are remapped.
            let mut changed_sections = std::collections::HashSet::new();
            let mut changed_units = std::collections::HashSet::new();
            for symbol in &a.symbols {
                if self.diff_visible("symbol", &super::snapshots::symbol_key(symbol)) {
                    changed_sections.insert(symbol.section_index);
                    if Some(symbol.section_index) == self.overview_section {
                        changed_units.insert(unit_key(symbol));
                    }
                }
            }
            items.retain(|item| match &item.identity {
                SliceIdentity::Section(name) => {
                    self.diff_visible("section", name)
                        || matches!(item.target, Some(Target::Section(index)) if changed_sections.contains(&index))
                }
                SliceIdentity::Symbol(id) => self.diff_visible("symbol", id),
                SliceIdentity::Unit(unit) => {
                    changed_units.contains(unit) || ambiguous_section
                        || baseline_sizes.as_ref().is_some_and(|sizes| {
                            sizes.get(&item.identity).copied() != Some(Ok(item.size))
                        })
                }
                SliceIdentity::Padding => {
                    ambiguous_section
                        || baseline_sizes.as_ref().is_some_and(|sizes| {
                            sizes.get(&item.identity).copied() != Some(Ok(item.size))
                        })
                }
            });
        }
        self.visible_rows = items.len();
        if items.is_empty() && self.diffs_active() {
            ui.weak("No differences in this breakdown.");
        }
        ui.label(format!(
            "{}: {}",
            self.overview_metric.label(),
            if ambiguous_section {
                format!("{} (baseline ambiguous)", bytes(total))
            } else {
                self.snapshot_difference(
                    total,
                    baseline_items.map(|items| items.iter().map(|s| s.size).sum()),
                )
            }
        ));
        let mut selected = None;
        for item in &items {
            let fraction = if total == 0 {
                0.0
            } else {
                item.size as f32 / total as f32
            };
            let old_size = if ambiguous_section {
                Some(Err(()))
            } else {
                baseline_sizes
                    .as_ref()
                    .map(|sizes| sizes.get(&item.identity).copied().unwrap_or(Ok(0)))
            };
            let removed = baseline.is_some()
                && match &item.identity {
                    SliceIdentity::Section(name) => {
                        !current.sections.iter().any(|s| &s.name == name)
                    }
                    _ => !current_items.contains(&item.identity),
                };
            let size_label = if removed {
                super::snapshots::removed_bytes(old_size.and_then(Result::ok))
            } else if old_size == Some(Err(())) {
                format!("{} (baseline ambiguous)", bytes(item.size))
            } else if baseline_sizes
                .as_ref()
                .is_some_and(|sizes| !sizes.contains_key(&item.identity))
            {
                format!("{} (new)", bytes(item.size))
            } else {
                self.snapshot_difference(item.size, old_size.and_then(Result::ok))
            };
            let label = format!("{} - {} ({:.1}%)", item.name, size_label, fraction * 100.0);
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
    fn baseline_slices_match_symbol_and_unit_identities_instead_of_display_labels() {
        let mut analysis = firmware_analysis_core::analyze_path(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../fixtures/build/cortex-m.elf"),
            &Default::default(),
        )
        .unwrap();
        let mut first = analysis
            .symbols
            .iter()
            .find(|s| s.size > 0)
            .unwrap()
            .clone();
        let section = first.section_index;
        first.name = "first".into();
        first.demangled_name = "same label".into();
        first.source_file = Some("first.c".into());
        first.dwarf_compilation_unit = None;
        first.compilation_unit = Some("unit.c".into());
        first.usage.flash = 10;
        let mut second = first.clone();
        second.name = "second".into();
        second.source_file = Some("second.c".into());
        second.usage.flash = 20;
        analysis.symbols = vec![first, second];
        let unit = UnitKey::Elf("unit.c".into());
        let baseline = metric_slices(
            &analysis,
            Some(section),
            Some(&unit),
            super::super::overview::Metric::Flash,
        );
        analysis.symbols[0].usage.flash = 15;
        analysis.symbols[0].address += 1024;
        analysis.symbols[0].source_line = Some(999);
        let current = metric_slices(
            &analysis,
            Some(section),
            Some(&unit),
            super::super::overview::Metric::Flash,
        );
        let changed = current.iter().find(|s| s.size == 15).unwrap();
        assert_eq!(baseline_size(changed, &baseline), Ok(10));
        let unchanged = current.iter().find(|s| s.size == 20).unwrap();
        assert_eq!(baseline_size(unchanged, &baseline), Ok(20));

        // ELF and DWARF units can have the same displayed path but are distinct.
        analysis.symbols[1].dwarf_compilation_unit = Some("unit.c".into());
        let units = metric_slices(
            &analysis,
            Some(section),
            None,
            super::super::overview::Metric::Flash,
        );
        let elf = units
            .iter()
            .find(|s| s.identity == SliceIdentity::Unit(unit.clone()))
            .unwrap();
        let dwarf = units
            .iter()
            .find(|s| s.identity == SliceIdentity::Unit(UnitKey::Dwarf("unit.c".into())))
            .unwrap();
        assert_eq!(elf.name, dwarf.name);
        assert_eq!(baseline_size(elf, &units), Ok(15));
        assert_eq!(baseline_size(dwarf, &units), Ok(20));
    }

    #[test]
    fn drilldown_does_not_choose_an_arbitrary_duplicate_baseline_section() {
        let current = firmware_analysis_core::analyze_path(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../fixtures/build/cortex-m.elf"),
            &Default::default(),
        )
        .unwrap();
        let section = current.sections.iter().find(|s| s.usage.flash > 0).unwrap();
        let mut baseline = current.clone();
        let mut duplicate = section.clone();
        duplicate.index = baseline.sections.iter().map(|s| s.index).max().unwrap() + 1;
        baseline.sections.push(duplicate);
        for unit in [None, Some(UnitKey::Other)] {
            assert!(baseline_slices(
                &current,
                &baseline,
                Some(section.index),
                unit.as_ref(),
                super::super::overview::Metric::Flash,
            )
            .is_err());
        }
        // The inverse is equally ambiguous: two current sections cannot both
        // claim the contents of one baseline section, even with different indexes.
        for duplicate in baseline.sections.iter().filter(|s| s.name == section.name) {
            for unit in [None, Some(UnitKey::Other)] {
                assert!(baseline_slices(
                    &baseline,
                    &current,
                    Some(duplicate.index),
                    unit.as_ref(),
                    super::super::overview::Metric::Flash,
                )
                .is_err());
            }
        }
        baseline.sections.retain(|s| s.name != section.name);
        assert!(baseline_slices(
            &current,
            &baseline,
            Some(section.index),
            None,
            super::super::overview::Metric::Flash,
        )
        .unwrap()
        .is_empty());
    }

    #[test]
    fn duplicate_baseline_slice_identities_are_unknown_and_missing_slices_are_zero() {
        let item = Slice {
            identity: SliceIdentity::Symbol("symbol".into()),
            name: "symbol".into(),
            size: 10,
            target: None,
            tip: String::new(),
        };
        assert_eq!(baseline_size(&item, &[]), Ok(0));
        assert_eq!(baseline_size(&item, std::slice::from_ref(&item)), Ok(10));
        assert_eq!(baseline_size(&item, &[item.clone(), item.clone()]), Err(()));
        let padding = Slice {
            identity: SliceIdentity::Padding,
            ..item.clone()
        };
        assert_eq!(baseline_size(&padding, std::slice::from_ref(&item)), Ok(0));
        let sizes = baseline_sizes(&[item.clone(), padding.clone(), item]);
        assert_eq!(sizes.get(&padding.identity), Some(&Ok(10)));
        assert_eq!(
            sizes.get(&SliceIdentity::Symbol("symbol".into())),
            Some(&Err(()))
        );
    }

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
