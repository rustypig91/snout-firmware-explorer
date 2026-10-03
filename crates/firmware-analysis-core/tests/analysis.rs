use firmware_analysis_core::{
    analyze_bytes, analyze_path,
    compare::compare,
    stack::{analyze_stack, parse_stack_usage},
    Analysis, AnalysisOptions, Classification, MemoryKind, MemoryRegion,
};

const ELF: &[u8] = include_bytes!("../../../fixtures/cortex-m.elf");
const GROWN: &[u8] = include_bytes!("../../../fixtures/cortex-m-grown.elf");
const STRIPPED: &[u8] = include_bytes!("../../../fixtures/cortex-m-stripped.elf");
fn analyze(data: &[u8]) -> Analysis {
    analyze_bytes(data, "fixture.elf", &AnalysisOptions::default()).unwrap()
}

#[test]
fn initialized_data_counts_in_both_memories() {
    let a = analyze(ELF);
    let data = a.sections.iter().find(|s| s.name == ".data").unwrap();
    assert_eq!(data.size, 8);
    assert_eq!(data.load_size, 8);
    assert_eq!(data.runtime_size, 8);
    assert_eq!(data.usage.flash, 8);
    assert_eq!(data.usage.ram, 8);
    assert!(data.load_address.unwrap() < data.address);
    assert_eq!(data.classification, Classification::InitializedRam);
}

#[test]
fn zero_fill_and_reservations_have_no_flash_payload() {
    let a = analyze(ELF);
    for name in [".bss", ".reserved"] {
        let s = a.sections.iter().find(|s| s.name == name).unwrap();
        assert_eq!(s.load_size, 0);
        assert_eq!(s.usage.flash, 0);
        assert_eq!(s.usage.ram, s.size);
        assert!(s.size > 0);
    }
}

#[test]
fn attributes_and_load_addresses_override_section_names() {
    let a = analyze(ELF);
    let copied = a.sections.iter().find(|s| s.name == ".ram_code").unwrap();
    assert!(copied.executable);
    assert!(!copied.writable);
    assert_eq!(copied.usage.flash, copied.size);
    assert_eq!(copied.usage.ram, copied.size);
    let unusual = a
        .sections
        .iter()
        .find(|s| s.name == ".unusual_constants")
        .unwrap();
    assert_eq!(unusual.usage.flash, 8);
    assert_eq!(unusual.usage.ram, 0);
    let debug = a.sections.iter().find(|s| s.name == ".debug_info").unwrap();
    assert_eq!(debug.usage.flash, 0);
    assert_eq!(debug.runtime_size, 0);
}

#[test]
fn thumb_functions_and_weak_aliases_are_not_double_counted() {
    let a = analyze(ELF);
    let reset = a
        .symbols
        .iter()
        .find(|s| s.name == "Reset_Handler")
        .unwrap();
    assert_eq!(reset.address & 1, 1);
    assert_eq!(reset.normalized_address & 1, 0);
    assert!(reset.size > 0);
    assert_eq!(reset.usage.flash, reset.size);
    let aliases: Vec<_> = a
        .symbols
        .iter()
        .filter(|s| s.name == "weak_callback" || s.name == "callback_alias")
        .collect();
    assert_eq!(aliases.len(), 2);
    assert!(aliases.iter().all(|s| s.weak));
    assert_eq!(
        aliases.iter().map(|s| s.usage.flash).sum::<u64>(),
        aliases[0].size
    );
}

#[test]
fn cxx_symbols_are_demangled() {
    let a = analyze(ELF);
    let symbol = a
        .symbols
        .iter()
        .find(|s| s.name == "_Z12cpp_functionj")
        .unwrap();
    assert_eq!(symbol.demangled_name, "cpp_function(unsigned int)");
}

#[test]
fn dwarf_functions_and_local_compilation_units_are_attributed_honestly() {
    let a = analyze(ELF);
    assert!(a.metadata.has_dwarf);
    for (name, file) in [("Reset_Handler", "main.c"), ("diagnose", "diag.c")] {
        let symbol = a.symbols.iter().find(|s| s.name == name).unwrap();
        assert!(symbol.source_file.as_ref().unwrap().ends_with(file));
        assert!(symbol.source_line.unwrap() > 0);
    }
    let local = a
        .symbols
        .iter()
        .find(|s| s.name == "diagnostic_count")
        .unwrap();
    assert!(local.compilation_unit.as_ref().unwrap().ends_with("diag.c"));
    let global = a.symbols.iter().find(|s| s.name == "initialized").unwrap();
    assert!(global.source_file.as_ref().unwrap().ends_with("main.c"));
    assert_eq!(global.attribution, "DWARF variable definition");
    assert!(global.compilation_unit.is_none());
}

#[test]
fn all_aggregates_reconcile() {
    let a = analyze(ELF);
    assert_eq!(
        a.files.iter().map(|f| f.usage.flash).sum::<u64>(),
        a.totals.flash
    );
    assert_eq!(
        a.files.iter().map(|f| f.usage.ram).sum::<u64>(),
        a.totals.ram
    );
    assert_eq!(a.tree.usage, a.totals);
    assert_eq!(
        a.sections.iter().map(|s| s.usage.flash).sum::<u64>(),
        a.totals.flash
    );
    assert!(a.symbols.iter().map(|s| s.usage.flash).sum::<u64>() <= a.totals.flash);
    assert!(a.unattributed.ram > 0);
}

#[test]
fn dwarf_file_groups_merge_without_changing_memory_totals() {
    let a = analyze(ELF);
    assert_eq!(
        a.files.len(),
        3,
        "Two source files plus unattributed storage"
    );
    for (name, flash, ram) in [("main.c", 164, 100), ("diag.c", 92, 4)] {
        let files: Vec<_> = a.files.iter().filter(|f| f.path.ends_with(name)).collect();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].usage.flash, flash);
        assert_eq!(files[0].usage.ram, ram);
        assert!(files[0].path.contains("fixtures/src/"));
    }
    assert_eq!(a.totals.flash, 260);
    assert_eq!(a.totals.ram, 232);
    assert_eq!(a.unattributed.flash, 4);
    assert_eq!(a.unattributed.ram, 128);
    for name in [
        "private_state",
        "diagnostic_count",
        "samples",
        "baudrate_table",
        "signature",
    ] {
        let symbol = a.symbols.iter().find(|s| s.name == name).unwrap();
        assert!(symbol.source_file.is_some(), "{name}");
        assert_eq!(symbol.attribution, "DWARF variable definition");
    }
}

#[test]
fn stripping_does_not_change_memory_totals() {
    let a = analyze(ELF);
    let stripped = analyze(STRIPPED);
    assert_eq!(a.totals, stripped.totals);
    assert!(stripped.symbols.is_empty());
    assert_eq!(stripped.unattributed, stripped.totals);
    assert!(!stripped.metadata.has_dwarf);
}

#[test]
fn comparison_tracks_growth_and_shrinkage() {
    let old = analyze(ELF);
    let new = analyze(GROWN);
    let diff = compare(&old, &new);
    assert_eq!(diff.flash_delta, 0);
    assert_eq!(diff.ram_delta, 32);
    assert!(diff
        .symbols
        .iter()
        .any(|s| s.identity.ends_with("samples") && s.ram_delta == 32));
    assert_eq!(diff.files.iter().map(|f| f.ram_delta).sum::<i128>(), 32);
    assert_eq!(compare(&new, &old).ram_delta, -32);
    assert!(compare(&old, &old).symbols.is_empty());
}

#[test]
fn comparison_marks_added_removed_and_changed_attribution() {
    let full = analyze(ELF);
    let stripped = analyze(STRIPPED);
    let removed = compare(&full, &stripped);
    assert_eq!(removed.flash_delta, 0);
    assert!(!removed.symbols.is_empty());
    assert!(removed.symbols.iter().all(|s| s.status == "removed"));
    assert!(removed
        .warnings
        .iter()
        .any(|w| w.contains("availability differs")));
    assert!(compare(&stripped, &full)
        .symbols
        .iter()
        .all(|s| s.status == "added"));
}

#[test]
fn partial_memory_layouts_report_uncovered_ranges_and_keep_options() {
    let options = AnalysisOptions {
        regions: vec![MemoryRegion {
            name: "Too small".into(),
            start: 0x08000000,
            size: 1,
            kind: MemoryKind::Flash,
        }],
    };
    let report = analyze_bytes(ELF, "fixture", &options).unwrap();
    assert_eq!(report.options, options);
    assert!(report
        .warnings
        .iter()
        .any(|w| w.contains("not fully covered")));
    assert!(compare(&analyze(ELF), &report)
        .warnings
        .iter()
        .any(|w| w.contains("different memory configurations")));
}

#[test]
fn memory_configuration_can_describe_a_ram_loaded_image() {
    let options = AnalysisOptions {
        regions: vec![MemoryRegion {
            name: "RAM-resident image".into(),
            start: 0x08000000,
            size: 0x100000,
            kind: MemoryKind::Ram,
        }],
    };
    let a = analyze_bytes(ELF, "fixture", &options).unwrap();
    assert_eq!(a.totals.flash, 0);
    assert!(a.totals.ram > analyze(ELF).totals.ram);
}

#[test]
fn invalid_configurations_are_rejected() {
    for regions in [
        vec![MemoryRegion {
            name: "overflow".into(),
            start: u64::MAX,
            size: 2,
            kind: MemoryKind::Ram,
        }],
        vec![
            MemoryRegion {
                name: "a".into(),
                start: 0,
                size: 10,
                kind: MemoryKind::Flash,
            },
            MemoryRegion {
                name: "b".into(),
                start: 5,
                size: 10,
                kind: MemoryKind::Ram,
            },
        ],
    ] {
        assert!(analyze_bytes(ELF, "fixture", &AnalysisOptions { regions }).is_err());
    }
}

#[test]
fn malformed_and_truncated_input_never_panics() {
    for bytes in [
        b"not ELF".as_slice(),
        b"\x7fELF",
        &ELF[..52],
        &ELF[..ELF.len() / 2],
    ] {
        assert!(analyze_bytes(bytes, "broken", &Default::default()).is_err());
    }
    // Mutate every byte in the ELF header, ensuring parser failures stay ordinary errors.
    for index in 0..52 {
        let mut bytes = ELF.to_vec();
        bytes[index] ^= 0xff;
        let _ = analyze_bytes(&bytes, "mutated", &Default::default());
    }
}

#[test]
fn malformed_section_range_is_rejected() {
    let mut bytes = ELF.to_vec();
    let table = u32::from_le_bytes(bytes[32..36].try_into().unwrap()) as usize;
    bytes[table + 40 + 16..table + 40 + 20].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(analyze_bytes(&bytes, "broken", &Default::default()).is_err());
}

#[test]
fn missing_path_is_a_readable_error() {
    let error = analyze_path("this-file-does-not-exist.elf", &Default::default()).unwrap_err();
    assert!(error.to_string().contains("Cannot read"));
}

#[test]
fn json_round_trip_keeps_exact_bytes() {
    let a = analyze(ELF);
    let json = serde_json::to_string(&a).unwrap();
    let restored: Analysis = serde_json::from_str(&json).unwrap();
    assert_eq!(restored.schema_version, 1);
    assert_eq!(restored.totals, a.totals);
    serde_json::to_string(&compare(&a, &analyze(GROWN))).unwrap();
}

#[test]
fn stack_records_handle_windows_cpp_and_dynamic_frames() {
    let input = "C:\\project\\a.cpp:42:3:ns::Device::run()\t48\tstatic\nfoo.c:2:1:other\t96\tdynamic,bounded\nfoo.c:3:1:unbounded\t16\tdynamic\nmalformed\n";
    let (entries, warnings) = parse_stack_usage(input, "test.su");
    assert_eq!(entries.len(), 3);
    assert_eq!(warnings.len(), 1);
    assert_eq!(entries[0].source_file, "C:\\project\\a.cpp");
    assert_eq!(entries[0].function, "ns::Device::run()");
    assert_eq!(entries[0].local_bytes, 48);
    assert_eq!(entries[2].qualifier, "dynamic");
}

#[test]
fn compiler_stack_reports_are_matched_without_invented_call_edges() {
    let a = analyze(ELF);
    let report = analyze_stack(
        &a,
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../fixtures/cortex-m-main.su"
        ),
    )
    .unwrap();
    assert!(!report.entries.is_empty());
    assert!(report
        .entries
        .iter()
        .any(|e| !e.symbol_candidates.is_empty()));
    assert!(!report.call_graph.complete);
    assert!(report.call_graph.edges.is_empty());
    assert!(!report.call_graph.unresolved.is_empty());
}

#[test]
fn absent_stack_reports_are_explicit() {
    let report = analyze_stack(
        &analyze(ELF),
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures/src"),
    )
    .unwrap();
    assert!(report.entries.is_empty());
    assert!(report.warnings.iter().any(|w| w.contains("No stack usage")));
}
