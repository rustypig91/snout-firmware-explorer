use firmware_analysis_core::{
    analyze_bytes,
    build::{analyze_build_firmware, scan_folder},
    dependencies::{from_map, import_map},
    map::{detect_map_format, parse_lld_sections, parse_map_regions, MapFormat},
};
use std::path::PathBuf;

const MAP: &str = include_str!("../../../fixtures/maps/llvm-lld.map");
const ELF: &[u8] = include_bytes!("../../../fixtures/maps/llvm-lld.elf");

#[test]
fn real_lld_map_sections_match_linked_elf_without_inventing_capacity() {
    assert_eq!(detect_map_format(MAP), MapFormat::LlvmLld);
    let sections = parse_lld_sections(MAP).unwrap();
    let analysis = analyze_bytes(ELF, "llvm-lld.elf", &Default::default()).unwrap();
    for section in &sections {
        let elf = analysis
            .sections
            .iter()
            .find(|s| s.name == section.name)
            .unwrap();
        assert_eq!(
            (section.vma, section.size, section.alignment),
            (elf.address, elf.size, elf.alignment)
        );
        if elf.allocated && elf.load_size > 0 {
            assert_eq!(Some(section.lma), elf.load_address);
        }
    }
    assert!(sections.iter().any(|s| s.name == ".data" && s.vma != s.lma));
    assert!(parse_map_regions(MAP)
        .unwrap_err()
        .to_string()
        .contains("not physical memory capacities"));
}

#[test]
fn lld_detection_handles_bom_crlf_and_spacing_but_rejects_other_dialects() {
    let windows = format!("\u{feff}{}", MAP.replace('\n', "\r\n"));
    assert_eq!(detect_map_format(&windows), MapFormat::LlvmLld);
    assert_eq!(
        parse_lld_sections(&windows).unwrap(),
        parse_lld_sections(MAP).unwrap()
    );
    assert_eq!(
        detect_map_format("VMA\tLMA Size Align Out In Symbol"),
        MapFormat::LlvmLld
    );
    for text in [
        "Address Size Align Out In Symbol",
        "Address Size FileOff Out In Symbol",
        "# Address Size File Name",
        "VMA LMA Size Align Out In",
        "VMA LMA Size Align Out In Symbol\nMemory Configuration",
    ] {
        assert_eq!(detect_map_format(text), MapFormat::Unknown);
        assert!(parse_lld_sections(text).is_err());
    }
}

#[test]
fn lld_64_bit_sections_ignore_nested_rows_and_script_assignments() {
    let text = "             VMA              LMA     Size Align Out     In      Symbol\n\
                    100000000        200000000       40    16 custom_section\n\
                    100000000        200000000       40    16         C:\\build dir\\archive.a(member.o):(.text)\n\
                    100000000        200000000       20     1                 function(int, char*)\n\
                    100000020        200000020        4     1         LONG ( 0x1234 )\n\
                    100000040        200000040        0     1 __end = .\n\
                    300000000        200000040        0     4 .empty\n";
    let rows = parse_lld_sections(text).unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].name, "custom_section");
    assert_eq!(
        (rows[0].vma, rows[0].lma, rows[0].size, rows[0].alignment),
        (0x100000000, 0x200000000, 0x40, 16)
    );
    assert_eq!(rows[1].size, 0);
}

#[test]
fn lld_backward_location_counter_is_not_an_allocation_range() {
    // lld 21 emits the unsigned delta when a script moves dot backward.
    let text = "VMA LMA Size Align Out In Symbol\n\
        20000040 20000040 14 16 .text\n\
        20000054 20000054 ffffffffe7ffffac 1 . = 0x08000000\n\
        8000000 8000000 4 4 .data\n";
    let sections = parse_lld_sections(text).unwrap();
    assert_eq!(sections.len(), 2);
    assert_eq!(sections[0].name, ".text");
    assert_eq!(sections[1].name, ".data");
    assert_eq!(sections[1].vma, 0x08000000);
}

#[test]
fn malformed_lld_placement_returns_errors_without_panicking() {
    for row in [
        "",
        "0 0",
        "xyz 0 10 1 .text",
        "0 0 10 bad .text",
        "ffffffffffffffff 0 1 1 .text",
        "0 ffffffffffffffff 1 1 .text",
        "0 0 10 1",
        "0 0 10 1   ",
    ] {
        let text = format!("VMA LMA Size Align Out In Symbol\n{row}\n");
        assert!(parse_lld_sections(&text).is_err(), "Accepted {row}");
    }
}

#[test]
fn lld_cref_preserves_dwarf_and_elf_information() {
    let mut analysis = analyze_bytes(ELF, "llvm-lld.elf", &Default::default()).unwrap();
    let symbols = serde_json::to_value(&analysis.symbols).unwrap();
    let files = serde_json::to_value(&analysis.files).unwrap();
    // Misleading object names cannot replace source ownership from DWARF.
    let map = MAP.replace("diag.c.obj", "wrong-source.c.obj");
    import_map(&mut analysis, &map, "llvm-lld.map");
    assert_eq!(serde_json::to_value(&analysis.symbols).unwrap(), symbols);
    assert_eq!(serde_json::to_value(&analysis.files).unwrap(), files);
    let graph = &analysis.dependencies;
    assert_eq!(graph.edges.len(), 16);
    assert_eq!(graph.nodes.len(), 6);
    let diag = graph
        .nodes
        .iter()
        .find(|n| n.label.ends_with("diag.c"))
        .unwrap();
    assert!(diag.id.starts_with("dwarf:"));
    assert!(diag
        .objects
        .iter()
        .any(|o| o.ends_with("wrong-source.c.obj")));
    assert!(graph.notes.iter().any(|n| n.contains("LLVM lld")));
    let no_cref = MAP.split("Cross Reference Table").next().unwrap();
    assert!(!parse_lld_sections(no_cref).unwrap().is_empty());
    assert!(from_map(&analysis, no_cref, "no-cref.map")
        .unwrap_err()
        .contains("--cref"));
}

#[test]
fn matching_lld_map_loads_dependencies_with_or_without_explicit_capacity() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/maps");
    let build = scan_folder(root).unwrap();
    let layout =
        serde_json::from_str(include_str!("../../../examples/cortex-m-memory.json")).unwrap();
    for options in [None, Some(&layout)] {
        let analysis =
            analyze_build_firmware(&build, &build.root.join("llvm-lld.elf"), options).unwrap();
        assert_eq!(analysis.dependencies.edges.len(), 16);
        if options.is_none() {
            assert!(analysis.options.regions.is_empty());
            assert!(analysis
                .warnings
                .iter()
                .any(|w| w.contains("capacity remains unknown") && w.contains("LLVM lld")));
        } else {
            assert_eq!(analysis.options, layout);
        }
    }
}
