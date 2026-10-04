use firmware_analysis_core::{Analysis, Classification, Section};

pub(super) fn ram_role(section: &Section) -> &'static str {
    let name = section.name.to_ascii_lowercase();
    if section.executable {
        "RAM code"
    } else if section.classification == Classification::InitializedRam {
        "Initialized data"
    } else if name.contains("reserved") || name.contains("heap") || name.contains("stack") {
        "Reservations (by section name)"
    } else if name == ".bss"
        || name.starts_with(".bss.")
        || name == ".sbss"
        || name.starts_with(".sbss.")
    {
        "Zero-initialized data (BSS)"
    } else {
        "Other RAM / no-load storage"
    }
}

/// Unique symbol contributions already exclude alias overlap. A section's
/// remaining bytes explain file attribution gaps without guessing their owner.
pub(super) fn section_unattributed(a: &Analysis, section: &Section, ram: bool) -> (u64, u64) {
    let value = |usage: firmware_analysis_core::Usage| if ram { usage.ram } else { usage.flash };
    let mut owned = 0;
    let mut covered = 0;
    for symbol in a
        .symbols
        .iter()
        .filter(|s| s.section_index == section.index)
    {
        covered += value(symbol.usage);
        if symbol.source_file.is_some() || symbol.compilation_unit.is_some() {
            owned += value(symbol.usage);
        }
    }
    (
        value(section.usage).saturating_sub(owned),
        value(section.usage).saturating_sub(covered),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ram_roles_and_attribution_gaps_reconcile_including_stripped_firmware() {
        for name in ["cortex-m-grown.elf", "cortex-m-stripped.elf"] {
            let a = firmware_analysis_core::analyze_path(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../fixtures")
                    .join(name),
                &Default::default(),
            )
            .unwrap();
            for ram in [false, true] {
                assert_eq!(
                    a.sections
                        .iter()
                        .map(|s| section_unattributed(&a, s, ram).0)
                        .sum::<u64>(),
                    if ram {
                        a.unattributed.ram
                    } else {
                        a.unattributed.flash
                    }
                );
            }
            let total: u64 = a
                .sections
                .iter()
                .filter(|s| s.usage.ram > 0)
                .map(|s| {
                    assert!(!ram_role(s).is_empty());
                    s.usage.ram
                })
                .sum();
            assert_eq!(total, a.totals.ram);
            if name == "cortex-m-grown.elf" {
                let reserved = a.sections.iter().find(|s| s.name == ".reserved").unwrap();
                assert_eq!(ram_role(reserved), "Reservations (by section name)");
                assert_eq!(section_unattributed(&a, reserved, true), (128, 128));
            }
        }
    }
}
