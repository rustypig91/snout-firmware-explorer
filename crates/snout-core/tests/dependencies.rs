use snout_core::{
    analyze_bytes,
    dependencies::{from_map, import_map, units},
    Analysis,
};

fn fixture() -> Analysis {
    analyze_bytes(
        include_bytes!("../../../fixtures/build/gcc/cortex-m.elf"),
        "firmware.elf",
        &Default::default(),
    )
    .unwrap()
}

// GNU ld's padded cross-reference format. Object paths intentionally have spaces,
// Windows separators, and an archive member; source identity comes only from symbols.
const MAP: &str = "Cross Reference Table\n\nSymbol                                            File\nReset_Handler                                     C:\\build dir\\main.o\n                                                  C:\\build dir\\main.o\ndiagnose                                          lib/diagnostics.a(diag.o)\n                                                  C:\\build dir\\main.o\n                                                  C:\\build dir\\main.o\ninitialized                                       C:\\build dir\\main.o\n                                                  lib/diagnostics.a(diag.o)\nexternal_missing                                  lib/unknown.o\n                                                  C:\\build dir\\main.o\n";

#[test]
fn exact_symbol_definitions_connect_units_and_preserve_reference_evidence() {
    let analysis = fixture();
    let graph = from_map(&analysis, MAP, "firmware.map").unwrap();
    let main = graph
        .nodes
        .iter()
        .find(|n| n.label.ends_with("/main.c"))
        .unwrap();
    let diag = graph
        .nodes
        .iter()
        .find(|n| n.label.ends_with("/diag.c"))
        .unwrap();
    assert_eq!(main.objects, ["C:/build dir/main.o"]);
    assert_eq!(diag.objects, ["lib/diagnostics.a(diag.o)"]);
    assert_eq!(
        graph
            .edges
            .iter()
            .find(|e| e.from == main.id && e.to == diag.id)
            .unwrap()
            .symbols,
        ["diagnose"]
    );
    assert_eq!(
        graph
            .edges
            .iter()
            .find(|e| e.from == diag.id && e.to == main.id)
            .unwrap()
            .symbols,
        ["initialized"]
    );
    assert!(!graph.edges.iter().any(|e| e.from == e.to));
    let unknown = graph
        .nodes
        .iter()
        .find(|n| n.label == "lib/unknown.o")
        .unwrap();
    assert!(unknown.usage.is_none());
    assert!(graph
        .edges
        .iter()
        .any(|e| e.from == main.id && e.to == unknown.id));
    assert_eq!(graph.map_path.as_deref(), Some("firmware.map"));
    let baseline = units(&analysis);
    let usage = |nodes: &[snout_core::dependencies::DependencyNode]| {
        nodes
            .iter()
            .filter_map(|n| n.usage)
            .fold((0, 0), |sum, u| (sum.0 + u.flash, sum.1 + u.ram))
    };
    assert_eq!(usage(&graph.nodes), usage(&baseline.nodes));
    assert_eq!(
        serde_json::to_value(&graph).unwrap(),
        serde_json::to_value(from_map(&analysis, MAP, "firmware.map").unwrap()).unwrap()
    );
}

#[test]
fn header_source_locations_do_not_create_compilation_units() {
    let mut analysis = fixture();
    let expected = units(&analysis).nodes.len();
    for symbol in &mut analysis.symbols {
        symbol.source_file = Some("include/shared.h".into());
    }
    let graph = units(&analysis);
    assert_eq!(graph.nodes.len(), expected);
    assert!(!graph.nodes.iter().any(|n| n.label.ends_with("shared.h")));
}

#[test]
fn ambiguous_duplicate_definitions_and_mixed_object_owners_are_not_guessed() {
    let mut analysis = fixture();
    let mut duplicate = analysis
        .symbols
        .iter()
        .find(|s| s.name == "diagnose")
        .unwrap()
        .clone();
    duplicate.dwarf_compilation_unit = Some("different/diag.c".into());
    analysis.symbols.push(duplicate);
    let graph = from_map(&analysis, MAP, "ambiguous.map").unwrap();
    assert!(graph
        .nodes
        .iter()
        .find(|n| n.label == "lib/diagnostics.a(diag.o)")
        .unwrap()
        .usage
        .is_none());
    let graph = from_map(
        &fixture(),
        &MAP.replace("lib/diagnostics.a(diag.o)", "C:\\build dir\\main.o"),
        "lto.map",
    )
    .unwrap();
    assert!(graph
        .nodes
        .iter()
        .find(|n| n.label == "C:/build dir/main.o")
        .unwrap()
        .usage
        .is_none());
}

#[test]
fn stripped_elf_keeps_object_graph_without_source_or_size_claims() {
    let analysis = analyze_bytes(
        include_bytes!("../../../fixtures/build/gcc/cortex-m-stripped.elf"),
        "stripped.elf",
        &Default::default(),
    )
    .unwrap();
    assert!(units(&analysis).nodes.is_empty());
    let graph = from_map(&analysis, MAP, "stripped.map").unwrap();
    assert_eq!(graph.nodes.len(), 3);
    assert_eq!(graph.edges.len(), 3);
    assert!(graph.nodes.iter().all(|n| n.usage.is_none()));
}

#[test]
fn malformed_tables_and_missing_reports_do_not_invent_edges() {
    let mut analysis = fixture();
    for text in [
        "no table",
        "Cross Reference Table\n",
        "Cross Reference Table\nSymbol File\n  orphan.o",
        "Cross Reference Table\nSymbol File\nmissing_file",
        "Cross Reference Table\nUnknown Header",
    ] {
        assert!(from_map(&analysis, text, "bad.map").is_err());
        import_map(&mut analysis, text, "bad.map");
        assert!(analysis.dependencies.edges.is_empty());
        assert!(analysis.dependencies.map_path.is_none());
        assert!(!analysis.dependencies.nodes.is_empty());
        assert_eq!(
            analysis
                .warnings
                .iter()
                .filter(|w| w.starts_with("Dependency graph: "))
                .count(),
            1
        );
    }
    // Additive report field stays backwards compatible with existing JSON envelopes.
    let mut json = serde_json::to_value(&analysis).unwrap();
    json.as_object_mut().unwrap().remove("dependencies");
    let restored: Analysis = serde_json::from_value(json).unwrap();
    assert!(restored.dependencies.nodes.is_empty());

    // A valid, empty table is different from missing cross-reference metadata.
    import_map(
        &mut analysis,
        "Cross Reference Table\nSymbol File\n",
        "valid.map",
    );
    assert_eq!(analysis.dependencies.map_path.as_deref(), Some("valid.map"));
    assert!(analysis.dependencies.edges.is_empty());
    assert!(!analysis
        .warnings
        .iter()
        .any(|w| w.starts_with("Dependency graph: ")));
}

#[test]
fn long_symbols_and_same_basename_objects_keep_distinct_identities() {
    let symbol = "a_very_long_raw_mangled_symbol_name_that_exceeds_the_linker_column_width";
    let text =
        format!("Cross Reference Table\nSymbol File\n{symbol} first/main.o\n  second/main.o\n");
    let graph = from_map(&fixture(), &text, "long.map").unwrap();
    assert!(graph.nodes.iter().any(|n| n.id == "object:first/main.o"));
    assert!(graph.nodes.iter().any(|n| n.id == "object:second/main.o"));
    assert_eq!(graph.edges[0].symbols, [symbol]);
}

#[test]
fn weak_aliases_use_exact_function_ranges_and_reject_conflicting_owners() {
    let mut analysis = fixture();
    let symbol = analysis
        .symbols
        .iter()
        .find(|s| s.name == "weak_callback")
        .unwrap()
        .clone();
    let text = "Cross Reference Table\nSymbol File\ncallback_alias  main.o\nReset_Handler  main.o\ndiagnose  diag.o\n  main.o\n";
    let graph = from_map(&analysis, text, "alias.map").unwrap();
    let main = graph
        .nodes
        .iter()
        .find(|n| n.label.ends_with("/main.c"))
        .unwrap();
    assert_eq!(main.objects, ["main.o"]);
    let expected_flash: u64 = analysis
        .symbols
        .iter()
        .filter(|s| {
            s.source_file
                .as_ref()
                .is_some_and(|file| file.ends_with("main.c"))
        })
        .map(|s| s.usage.flash)
        .sum();
    assert_eq!(main.usage.unwrap().flash, expected_flash);
    let mut conflicting = symbol;
    conflicting.name = "folded_function".into();
    conflicting.dwarf_compilation_unit = Some("other/unit.c".into());
    analysis.symbols.push(conflicting);
    let graph = from_map(&analysis, text, "ambiguous-alias.map").unwrap();
    assert!(graph
        .nodes
        .iter()
        .find(|n| n.label == "main.o")
        .unwrap()
        .usage
        .is_none());
}

#[test]
fn long_symbols_preserve_defining_paths_with_repeated_spaces() {
    let symbol = "a_very_long_raw_mangled_symbol_name_that_exceeds_the_linker_column_width";
    let object = "build  directory/main.o";
    let text = format!("Cross Reference Table\nSymbol File\n{symbol} {object}\n  other.o\n");
    let graph = from_map(&fixture(), &text, "spaces.map").unwrap();
    assert!(graph
        .nodes
        .iter()
        .any(|n| n.id == format!("object:{object}")));
    assert_eq!(graph.edges[0].to, format!("object:{object}"));
    assert_eq!(graph.edges[0].symbols, [symbol]);
}

#[test]
fn local_symbols_cannot_establish_cross_reference_object_ownership() {
    let analysis = fixture();
    let local = analysis
        .symbols
        .iter()
        .find(|s| s.name == "private_state")
        .unwrap();
    assert!(local.local);
    let text = "Cross Reference Table\nSymbol File\nprivate_state external.o\n  caller.o\n";
    let graph = from_map(&analysis, text, "locals.map").unwrap();
    assert!(graph
        .nodes
        .iter()
        .any(|n| n.id == "object:external.o" && n.usage.is_none()));
    assert_eq!(graph.edges[0].to, "object:external.o");
}

#[test]
fn local_name_collision_does_not_make_global_definition_ambiguous() {
    let mut analysis = fixture();
    let mut local = analysis
        .symbols
        .iter()
        .find(|s| s.name == "diagnose")
        .unwrap()
        .clone();
    assert!(!local.local);
    local.local = true;
    local.dwarf_compilation_unit = Some("other/unit.c".into());
    analysis.symbols.push(local);
    let graph = from_map(&analysis, MAP, "locals.map").unwrap();
    let diag = graph
        .nodes
        .iter()
        .find(|n| n.label.ends_with("/diag.c"))
        .unwrap();
    assert_eq!(diag.objects, ["lib/diagnostics.a(diag.o)"]);
}

#[test]
fn undefined_or_discarded_symbols_cannot_invent_source_unit_dependencies() {
    // GNU ld emits exactly this row shape for an unresolved weak symbol used
    // by both objects: the first file is a caller, not a defining object.
    let text = "Cross Reference Table\nSymbol File\nReset_Handler main.o\ndiagnose diag.o\n  main.o\nmissing_weak diag.o\n  main.o\n  third.o\n";
    let graph = from_map(&fixture(), text, "weak.map").unwrap();
    assert_eq!(graph.edges.len(), 1);
    assert_eq!(graph.edges[0].symbols, ["diagnose"]);
    assert!(graph.nodes.iter().any(|n| n.id == "object:third.o"));
    assert!(graph
        .notes
        .iter()
        .any(|n| n.contains("1 symbols without a global ELF definition")));
}

#[test]
fn missing_cross_reference_hints_follow_detected_map_format() {
    let mut analysis = fixture();
    for (text, label, supports_flags) in [
        ("Memory Configuration\n", "GNU ld", true),
        (
            "             VMA              LMA     Size Align Out     In      Symbol\n",
            "LLVM lld",
            true,
        ),
        ("MEMORY CONFIGURATION\n", "TI", false),
        ("unrecognized map", "Unrecognized", false),
    ] {
        import_map(&mut analysis, text, "app.map");
        let graph = &analysis.dependencies;
        assert!(graph.connection_hint.as_ref().unwrap().contains(label));
        assert_eq!(graph.cross_reference_flags.is_some(), supports_flags);
        if !supports_flags {
            assert!(!graph.notes.iter().any(|n| n.contains("--cref")));
            assert!(!analysis.warnings.iter().any(|n| n.contains("--cref")));
        }
    }
    import_map(&mut analysis, MAP, "valid.map");
    assert!(analysis.dependencies.connection_hint.is_none());
    assert!(analysis.dependencies.cross_reference_flags.is_none());
    let graph = units(&fixture());
    assert!(graph.connection_hint.is_none());
    assert!(graph.cross_reference_flags.is_none());
}
