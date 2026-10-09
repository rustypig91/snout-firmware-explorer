use snout_core::{analyze_bytes, regions::region_usage, AnalysisOptions, MemoryKind, MemoryRegion};

fn fixture(stripped: bool) -> snout_core::Analysis {
    let data: &[u8] = if stripped {
        include_bytes!("../../../fixtures/build/gcc/cortex-m-stripped.elf")
    } else {
        include_bytes!("../../../fixtures/build/gcc/cortex-m.elf")
    };
    analyze_bytes(data, "fixture", &AnalysisOptions::default()).unwrap()
}

#[test]
fn physical_regions_count_sections_once_and_include_copied_symbols() {
    let a = fixture(false);
    let flash = MemoryRegion {
        name: "Flash".into(),
        start: 0x08000000,
        size: 262144,
        kind: MemoryKind::Flash,
    };
    let ram = MemoryRegion {
        name: "RAM".into(),
        start: 0x20000000,
        size: 65536,
        kind: MemoryKind::Ram,
    };
    let f = region_usage(&a, &flash);
    let r = region_usage(&a, &ram);
    assert_eq!(f.used, a.totals.flash);
    assert_eq!(r.used, a.totals.ram);
    assert_eq!(f.free + f.used, flash.size);
    assert_eq!(r.free + r.used, ram.size);
    let data = a.sections.iter().find(|s| s.name == ".data").unwrap();
    let index = a
        .symbols
        .iter()
        .position(|s| s.section_index == data.index && s.size > 0)
        .unwrap();
    let load = f.symbols.iter().find(|s| s.symbol_index == index).unwrap();
    let run = r.symbols.iter().find(|s| s.symbol_index == index).unwrap();
    assert_eq!(load.placement, "Load image");
    assert_eq!(run.placement, "Runtime");
    assert_eq!(
        load.address - data.load_address.unwrap(),
        run.address - data.address
    );
    let stripped = fixture(true);
    assert_eq!(region_usage(&stripped, &flash).used, f.used);
    assert_eq!(region_usage(&stripped, &ram).used, r.used);
    assert!(region_usage(&stripped, &ram).symbols.is_empty());
}

#[test]
fn partial_regions_empty_regions_and_aliases_have_correct_occupancy() {
    let mut a = fixture(false);
    let data = a
        .sections
        .iter()
        .find(|s| s.name == ".data")
        .unwrap()
        .clone();
    let region = MemoryRegion {
        name: "Partial".into(),
        start: data.address + 1,
        size: 2,
        kind: MemoryKind::Ram,
    };
    let before = region_usage(&a, &region);
    assert_eq!((before.used, before.free), (2, 0));
    assert!(!before.symbols.is_empty());
    let mut alias = a.symbols[before.symbols[0].symbol_index].clone();
    alias.name = "alias".into();
    a.symbols.push(alias);
    let after = region_usage(&a, &region);
    assert_eq!(after.used, before.used);
    assert_eq!(after.symbols.len(), before.symbols.len() + 1);
    let empty = MemoryRegion {
        start: 0x40000000,
        ..region
    };
    let usage = region_usage(&a, &empty);
    assert_eq!((usage.used, usage.free), (0, 2));
    assert!(usage.symbols.is_empty());
}
