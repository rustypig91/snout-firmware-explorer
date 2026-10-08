use firmware_analysis_core::build::{
    analyze_build_firmware, parse_map_regions, scan_folder, ArtifactKind,
};
use std::{fs, path::PathBuf};

const MAP: &str = "Memory Configuration\n\nName             Origin             Length             Attributes\nFLASH            0x08000000         0x00040000         xr\nRAM              0x20000000         0x00010000         xrw\n*default*        0x00000000         0xffffffff\n\nLinker script and memory map\n";

#[test]
fn committed_fixtures_import_matching_map_capacities() {
    let dir = Temp::new();
    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/build");
    for name in ["cortex-m", "cortex-m-grown", "cortex-m-stripped"] {
        for extension in ["elf", "map"] {
            let file = format!("{name}.{extension}");
            fs::copy(fixtures.join(&file), dir.0.join(&file)).unwrap();
        }
    }
    let build = scan_folder(&dir.0).unwrap();
    for name in ["cortex-m", "cortex-m-grown", "cortex-m-stripped"] {
        let firmware = build.root.join(format!("{name}.elf"));
        assert_eq!(
            build.matching_map(&firmware),
            Some(build.root.join(format!("{name}.map")).as_path())
        );
        let report = analyze_build_firmware(&build, &firmware, None).unwrap();
        let regions: Vec<_> = report
            .options
            .regions
            .iter()
            .map(|region| (region.name.as_str(), region.start, region.size))
            .collect();
        assert_eq!(
            regions,
            vec![
                ("FLASH", 0x08000000, 256 * 1024),
                ("RAM", 0x20000000, 64 * 1024)
            ]
        );
    }
}

struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "snout-build-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn scans_nested_artifacts_and_uses_map_capacities() {
    let dir = Temp::new();
    fs::create_dir_all(dir.0.join("objects")).unwrap();
    fs::write(
        dir.0.join("app.elf"),
        include_bytes!("../../../fixtures/build/cortex-m.elf"),
    )
    .unwrap();
    fs::write(dir.0.join("app.map"), MAP).unwrap();
    fs::write(
        dir.0.join("objects/main.SU"),
        "main.c:1:1:main\t8\tstatic\n",
    )
    .unwrap();
    fs::write(dir.0.join("memory.ld"), "MEMORY {}").unwrap();
    fs::write(dir.0.join("memory.lds"), "MEMORY {}").unwrap();
    fs::write(
        dir.0.join("layout.json"),
        b"{\"regions\":[{\"name\":\"FLASH\",\"start\":134217728,\"size\":262144,\"kind\":\"Flash\"}]}",
    )
    .unwrap();
    fs::write(dir.0.join("other.json"), "{}").unwrap();
    fs::write(dir.0.join("fake.out"), "not ELF").unwrap();
    let build = scan_folder(&dir.0).unwrap();
    assert_eq!(build.artifacts.len(), 3);
    assert!(!build.artifacts.iter().any(|a| {
        matches!(
            a.path.extension().and_then(|e| e.to_str()),
            Some("ld" | "lds")
        )
    }));
    assert!(build
        .artifacts
        .iter()
        .any(|a| a.kind == ArtifactKind::StackUsage));
    let path = build.root.join("app.elf");
    let report = analyze_build_firmware(&build, &path, None).unwrap();
    assert_eq!(report.options.regions.len(), 2);
    assert_eq!(report.options.regions[0].size, 262144);
    assert_eq!(report.options.regions[1].size, 65536);
    assert!(report.warnings.iter().any(|w| w.contains("app.map")));
    let override_options = Default::default();
    assert!(
        analyze_build_firmware(&build, &path, Some(&override_options))
            .unwrap()
            .options
            .regions
            .is_empty()
    );
    assert!(scan_folder(path).is_err());
}

#[test]
fn ambiguous_maps_are_not_automatically_applied() {
    let dir = Temp::new();
    for sub in ["one", "two"] {
        fs::create_dir_all(dir.0.join(sub)).unwrap();
        fs::write(dir.0.join(sub).join("app.map"), MAP).unwrap();
    }
    let build = scan_folder(&dir.0).unwrap();
    assert!(build.matching_map(&build.root.join("app.elf")).is_none());
    assert_eq!(
        build.matching_map(&build.root.join("one/app.elf")),
        Some(build.root.join("one/app.map").as_path())
    );
}

#[test]
fn unsupported_or_invalid_maps_do_not_block_firmware() {
    assert!(parse_map_regions("Some other linker format").is_err());
    assert!(parse_map_regions(&MAP.replace("0x20000000", "0x08000000")).is_err());
    assert!(parse_map_regions(&MAP.replace("0x00010000", "0xffffffffffffffff")).is_err());
    assert!(parse_map_regions(&MAP.replace("0x00010000", "bad-value")).is_err());
    let dir = Temp::new();
    fs::write(
        dir.0.join("app.elf"),
        include_bytes!("../../../fixtures/build/cortex-m.elf"),
    )
    .unwrap();
    fs::write(dir.0.join("app.map"), "unsupported map").unwrap();
    let build = scan_folder(&dir.0).unwrap();
    let report = analyze_build_firmware(&build, &build.root.join("app.elf"), None).unwrap();
    assert!(report.options.regions.is_empty());
    assert!(report
        .warnings
        .iter()
        .any(|w| w.contains("capacity remains unknown")));
}

#[test]
fn matching_cross_references_load_even_with_explicit_memory_layout() {
    let dir = Temp::new();
    fs::write(
        dir.0.join("app.elf"),
        include_bytes!("../../../fixtures/build/cortex-m.elf"),
    )
    .unwrap();
    fs::write(dir.0.join("app.map"), format!("{MAP}\nCross Reference Table\nSymbol File\nReset_Handler  main.o\ndiagnose  diag.o\n  main.o\n")).unwrap();
    let build = scan_folder(&dir.0).unwrap();
    for options in [None, Some(Default::default())] {
        let analysis = analyze_build_firmware(
            &build,
            build.root.join("app.elf").as_path(),
            options.as_ref(),
        )
        .unwrap();
        assert_eq!(analysis.dependencies.edges.len(), 1);
        assert_eq!(analysis.dependencies.edges[0].symbols, ["diagnose"]);
        assert!(analysis
            .dependencies
            .map_path
            .as_ref()
            .unwrap()
            .ends_with("app.map"));
    }
    // A matching filename does not make an unsupported map into dependency evidence.
    fs::write(dir.0.join("app.map"), MAP).unwrap();
    let analysis = analyze_build_firmware(&build, &build.root.join("app.elf"), None).unwrap();
    assert!(analysis.dependencies.edges.is_empty());
    assert!(analysis.dependencies.map_path.is_none());
    assert!(!analysis.dependencies.nodes.is_empty());
}

#[test]
fn committed_fixtures_have_six_units_and_real_cross_dependencies() {
    let build = scan_folder(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/build"))
        .unwrap();
    for name in ["cortex-m", "cortex-m-grown"] {
        let analysis =
            analyze_build_firmware(&build, &build.root.join(format!("{name}.elf")), None).unwrap();
        let graph = &analysis.dependencies;
        assert_eq!(graph.nodes.len(), 6);
        assert!(graph.nodes.iter().all(|n| n.usage.is_some()));
        assert_eq!(graph.edges.len(), 16);
        for (from, to, symbol) in [
            ("main.c", "diag.c", "diagnose"),
            ("main.c", "telemetry.c", "telemetry_collect"),
            ("diag.c", "telemetry.c", "telemetry_scale"),
            ("telemetry.c", "diag.c", "diagnose"),
            ("telemetry.c", "main.c", "ram_function"),
            ("main.c", "sensor.c", "sensor_sample"),
            ("main.c", "config.c", "config_get"),
            ("main.c", "transport.c", "transport_flush"),
            ("sensor.c", "config.c", "config_get"),
            ("diag.c", "config.c", "config_alarm_threshold"),
            ("diag.c", "transport.c", "transport_pending"),
            ("telemetry.c", "config.c", "config_checksum_seed"),
            ("telemetry.c", "transport.c", "transport_enqueue"),
            ("transport.c", "config.c", "config_baudrate"),
            ("transport.c", "telemetry.c", "telemetry_checksum"),
            ("transport.c", "diag.c", "diagnostics_record_fault"),
        ] {
            let from = &graph
                .nodes
                .iter()
                .find(|n| n.label.ends_with(from))
                .unwrap()
                .id;
            let to = &graph
                .nodes
                .iter()
                .find(|n| n.label.ends_with(to))
                .unwrap()
                .id;
            let edge = graph
                .edges
                .iter()
                .find(|e| &e.from == from && &e.to == to)
                .unwrap();
            assert!(edge.symbols.iter().any(|name| name == symbol));
        }
        let reports = build
            .root
            .join(format!("CMakeFiles/{name}-objects.dir/src"));
        let stack = firmware_analysis_core::stack::analyze_stack(&analysis, &reports).unwrap();
        assert_eq!(stack.entries.len(), 24);
        assert!(stack
            .entries
            .iter()
            .all(|entry| entry.symbol_candidates.len() == 1));
        assert!(stack.entries.iter().any(
            |entry| entry.function == "telemetry_collect" && entry.symbol_candidates.len() == 1
        ));
    }
    let stripped =
        analyze_build_firmware(&build, &build.root.join("cortex-m-stripped.elf"), None).unwrap();
    assert_eq!(stripped.dependencies.nodes.len(), 6);
    assert_eq!(stripped.dependencies.edges.len(), 16);
    assert!(stripped
        .dependencies
        .nodes
        .iter()
        .all(|n| n.usage.is_none()));
}

const TI_MAP: &str = "ARM Linker PC v20.2.7.LTS\nOUTPUT FILE NAME: <app.out>\nMEMORY CONFIGURATION\nname origin length used unused attr fill\n---------------------- -------- -------- -------- -------- ---- --------\nPAGE 0:\nFLASH 08000000 00040000 00000100 0003ff00 RWIX\nPAGE 1:\nSRAM 20000000 00010000 00000020 0000ffe0 RWIX\nEMPTY 30000000 00000000 00000000 00000000 RWIX\nSECTION ALLOCATION MAP\nnot a memory region\n";

#[test]
fn detects_formats_from_contents_including_bom_and_crlf() {
    use firmware_analysis_core::build::{detect_map_format, MapFormat};
    assert_eq!(detect_map_format(MAP), MapFormat::GnuLd);
    assert_eq!(detect_map_format(TI_MAP), MapFormat::TexasCgt);
    assert_eq!(detect_map_format("unknown.map"), MapFormat::Unknown);
    assert_eq!(
        detect_map_format(&format!("{MAP}{TI_MAP}")),
        MapFormat::Unknown
    );
    let text = format!("\u{feff}{}", TI_MAP.replace('\n', "\r\n"));
    assert_eq!(detect_map_format(&text), MapFormat::TexasCgt);
    assert_eq!(parse_map_regions(&text).unwrap().regions.len(), 2);
    assert!(parse_map_regions(&format!("\u{feff}{MAP}")).is_ok());
}

#[test]
fn ti_regions_use_capacity_and_names_before_permissions() {
    use firmware_analysis_core::MemoryKind;
    let options = parse_map_regions(TI_MAP).unwrap();
    assert_eq!(options.regions.len(), 2);
    assert_eq!(options.regions[0].start, 0x08000000);
    assert_eq!(options.regions[0].size, 0x40000);
    assert_eq!(options.regions[0].kind, MemoryKind::Flash);
    assert_eq!(options.regions[1].size, 0x10000);
    assert_eq!(options.regions[1].kind, MemoryKind::Ram);
}

#[test]
fn ti_legacy_table_and_hex_variants() {
    let options = parse_map_regions("MEMORY CONFIGURATION\nname origin length attributes fill\n-------- -------- -------- ---------- --------\nEEPROM 0X08000000 0x400 RWIX ffffffff\nRAM 20000000 0000ABCD RW\nGLOBAL SYMBOLS\nignored\n").unwrap();
    assert_eq!(options.regions[0].size, 0x400);
    assert_eq!(
        options.regions[0].kind,
        firmware_analysis_core::MemoryKind::Flash
    );
    assert_eq!(options.regions[1].size, 0xabcd);
}

#[test]
fn ti_target_detection_does_not_use_the_output_filename() {
    let text = TI_MAP.replace("<app.out>", "<C:/C2000/ARM/app.out>");
    assert_eq!(
        parse_map_regions(&text).unwrap(),
        parse_map_regions(TI_MAP).unwrap()
    );
    for target in [
        "TMS320C2800",
        "TMS320C2000 COFF",
        "TMS320C54x",
        "TMS320C55x",
        "C2000",
    ] {
        let text = TI_MAP.replace("ARM Linker", &format!("{target} Linker"));
        assert!(parse_map_regions(&text)
            .unwrap_err()
            .to_string()
            .contains("word-addressed"));
    }
}

#[test]
fn ti_invalid_and_unrepresentable_layouts_are_rejected() {
    for text in [
        TI_MAP.replace("20000000", "08000000"),
        TI_MAP.replace("00010000", "ffffffffffffffff"),
        TI_MAP.replace("00040000", "invalid"),
        TI_MAP.replace("00000100", "invalid"),
        TI_MAP.replace("PAGE 1:", "PAGE bad:"),
        TI_MAP.replace("ARM Linker", "TMS320C2800 Linker"),
        "MEMORY CONFIGURATION\nname origin length attr\n".into(),
        "MEMORY CONFIGURATION\nname origin length used attr\nFLASH 0 100 0 RWIX\n".into(),
        "MEMORY CONFIGURATION\nFLASH 08000000 00040000 RWIX\n".into(),
    ] {
        assert!(parse_map_regions(&text).is_err(), "Accepted: {text}");
    }
}

#[test]
fn ti_automatic_import_preserves_elf_and_dwarf_information() {
    let dir = Temp::new();
    let firmware = dir.0.join("app.out");
    fs::write(
        &firmware,
        include_bytes!("../../../fixtures/build/cortex-m.elf"),
    )
    .unwrap();
    fs::write(dir.0.join("app.map"), TI_MAP).unwrap();
    let baseline = firmware_analysis_core::analyze_path(&firmware, &Default::default()).unwrap();
    assert!(baseline
        .symbols
        .iter()
        .any(|s| s.dwarf_compilation_unit.is_some()));
    let build = scan_folder(&dir.0).unwrap();
    let imported = analyze_build_firmware(&build, &firmware, None).unwrap();
    assert_eq!(imported.options.regions.len(), 2);
    assert!(imported
        .warnings
        .iter()
        .any(|w| w.contains("Texas Instruments CGT")));
    let attribution = |a: &firmware_analysis_core::Analysis| {
        a.symbols
            .iter()
            .map(|s| {
                (
                    s.name.clone(),
                    s.address,
                    s.size,
                    s.compilation_unit.clone(),
                    s.dwarf_compilation_unit.clone(),
                )
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(attribution(&baseline), attribution(&imported));
}

#[test]
fn section_evidence_selects_a_renamed_map_over_a_stale_same_name_map() {
    let dir = Temp::new();
    fs::write(
        dir.0.join("app.elf"),
        include_bytes!("../../../fixtures/build/cortex-m.elf"),
    )
    .unwrap();
    fs::write(
        dir.0.join("app.map"),
        include_str!("../../../fixtures/build/cortex-m-grown.map"),
    )
    .unwrap();
    fs::create_dir(dir.0.join("reports")).unwrap();
    fs::write(
        dir.0.join("reports/linker-output.map"),
        include_str!("../../../fixtures/build/cortex-m.map"),
    )
    .unwrap();
    let build = scan_folder(&dir.0).unwrap();
    let elf = build.root.join("app.elf");
    let map = build.root.join("reports/linker-output.map");
    assert_eq!(build.matching_map(&elf), Some(map.as_path()));
    let report = analyze_build_firmware(&build, &elf, None).unwrap();
    assert_eq!(report.options.regions.len(), 2);
    assert_eq!(report.dependencies.edges.len(), 16);
    assert!(report
        .warnings
        .iter()
        .any(|note| note.contains("matched by ELF section")));
}

#[test]
fn stale_named_map_is_rejected_and_changes_are_reread() {
    let dir = Temp::new();
    fs::write(
        dir.0.join("app.elf"),
        include_bytes!("../../../fixtures/build/cortex-m.elf"),
    )
    .unwrap();
    let map = dir.0.join("app.map");
    fs::write(
        &map,
        include_str!("../../../fixtures/build/cortex-m-grown.map"),
    )
    .unwrap();
    let build = scan_folder(&dir.0).unwrap();
    let elf = build.root.join("app.elf");
    assert!(build.matching_map(&elf).is_none());
    assert!(analyze_build_firmware(&build, &elf, None)
        .unwrap()
        .options
        .regions
        .is_empty());
    fs::write(&map, include_str!("../../../fixtures/build/cortex-m.map")).unwrap();
    assert_eq!(
        build.matching_map(&elf),
        Some(build.root.join("app.map").as_path())
    );
}

#[test]
fn identical_unnamed_content_matches_remain_ambiguous() {
    let dir = Temp::new();
    fs::write(
        dir.0.join("app.elf"),
        include_bytes!("../../../fixtures/build/cortex-m.elf"),
    )
    .unwrap();
    for name in ["one.map", "two.map"] {
        fs::write(
            dir.0.join(name),
            include_str!("../../../fixtures/build/cortex-m.map"),
        )
        .unwrap();
    }
    let build = scan_folder(&dir.0).unwrap();
    assert!(build.matching_map(&build.root.join("app.elf")).is_none());
    // Filename evidence can disambiguate maps with identical section layouts.
    fs::rename(dir.0.join("one.map"), dir.0.join("app.map")).unwrap();
    let build = scan_folder(&dir.0).unwrap();
    assert_eq!(
        build.matching_map(&build.root.join("app.elf")),
        Some(build.root.join("app.map").as_path())
    );
}

#[test]
fn region_coverage_and_partial_sections_do_not_identify_a_map() {
    let dir = Temp::new();
    fs::write(
        dir.0.join("app.elf"),
        include_bytes!("../../../fixtures/build/cortex-m.elf"),
    )
    .unwrap();
    fs::write(dir.0.join("unrelated.map"), MAP).unwrap();
    fs::write(
        dir.0.join("app.map"),
        format!("{MAP}\n.text 0x08000000 0x540\n"),
    )
    .unwrap();
    let build = scan_folder(&dir.0).unwrap();
    assert!(build.matching_map(&build.root.join("app.elf")).is_none());
}

#[test]
fn gnu_wrapped_output_names_and_input_sections_are_distinguished() {
    let dir = Temp::new();
    fs::write(
        dir.0.join("app.elf"),
        include_bytes!("../../../fixtures/build/cortex-m.elf"),
    )
    .unwrap();
    let text = include_str!("../../../fixtures/build/cortex-m.map").replace(
        ".text           0x08000000      0x540",
        ".text\n                0x08000000      0x540",
    );
    fs::write(
        dir.0.join("renamed.map"),
        format!("\u{feff}{}", text.replace('\n', "\r\n")),
    )
    .unwrap();
    let build = scan_folder(&dir.0).unwrap();
    assert_eq!(
        build.matching_map(&build.root.join("app.elf")),
        Some(build.root.join("renamed.map").as_path())
    );
}

#[test]
fn lld_section_content_can_select_dependency_map_without_capacities() {
    let dir = Temp::new();
    fs::write(
        dir.0.join("app.elf"),
        include_bytes!("../../../fixtures/build/cortex-m.elf"),
    )
    .unwrap();
    let mut text = matching_section_map(firmware_analysis_core::map::MapFormat::LlvmLld);
    text.push_str(
        "Cross Reference Table\nSymbol File\nReset_Handler main.o\ndiagnose diag.o\n main.o\n",
    );
    fs::write(dir.0.join("linker.map"), text).unwrap();
    let build = scan_folder(&dir.0).unwrap();
    let firmware = build.root.join("app.elf");
    assert_eq!(
        build.matching_map(&firmware),
        Some(build.root.join("linker.map").as_path())
    );
    let report = analyze_build_firmware(&build, &firmware, None).unwrap();
    assert!(report.options.regions.is_empty());
    assert_eq!(report.dependencies.edges.len(), 1);
}

#[test]
fn common_sections_parse_committed_gnu_map() {
    let rows = firmware_analysis_core::map::parse_map_sections(include_str!(
        "../../../fixtures/build/cortex-m.map"
    ))
    .unwrap()
    .unwrap();
    assert!(rows
        .iter()
        .any(|row| row.name == ".text" && row.size == 0x540));
}

fn matching_section_map(format: firmware_analysis_core::map::MapFormat) -> String {
    use firmware_analysis_core::map::{parse_map_sections, MapFormat};
    let sections = parse_map_sections(include_str!("../../../fixtures/build/cortex-m.map"))
        .unwrap()
        .unwrap();
    let mut text = match format {
        MapFormat::GnuLd => MAP.to_owned(),
        MapFormat::TexasCgt => TI_MAP.replace("not a memory region\n", "output attributes/\nsection page origin length input sections\n-------- ---- ---------- ---------- ----------------\n"),
        MapFormat::LlvmLld => "VMA LMA Size Align Out In Symbol\n".into(),
        _ => unreachable!(),
    };
    for section in sections
        .into_iter()
        .filter(|section| section.address != 0 && section.size != 0)
    {
        match format {
            MapFormat::GnuLd => {
                text.push_str(&format!(
                    "{} 0x{:x} 0x{:x}",
                    section.name, section.address, section.size
                ));
                if let Some(load) = section.load_address {
                    text.push_str(&format!(" load address 0x{load:x}"));
                }
                text.push('\n');
            }
            MapFormat::TexasCgt => {
                let origin = section.load_address.unwrap_or(section.address);
                // Exercise wrapped names and ignore nested inputs.
                text.push_str(&format!(
                    "{}\n 0 {:08x} {:08x}",
                    section.name, origin, section.size
                ));
                if section.load_address.is_some() {
                    text.push_str(&format!(" RUN ADDR = {:08x}", section.address));
                }
                text.push_str(&format!(
                    "\n {:08x} {:08x} main.obj ({})\n",
                    origin, section.size, section.name
                ));
            }
            MapFormat::LlvmLld => text.push_str(&format!(
                "{:x} {:x} {:x} 4 {}\n",
                section.address,
                section.load_address.unwrap_or(section.address),
                section.size,
                section.name
            )),
            _ => unreachable!(),
        }
    }
    text
}

#[test]
fn all_formats_expose_common_runtime_section_evidence() {
    use firmware_analysis_core::map::{parse_map_sections, MapFormat};
    let mut expected = None;
    for format in [MapFormat::GnuLd, MapFormat::TexasCgt, MapFormat::LlvmLld] {
        let text = matching_section_map(format);
        let sections = parse_map_sections(&format!("\u{feff}{}", text.replace('\n', "\r\n")))
            .unwrap()
            .unwrap();
        let values: Vec<_> = sections
            .iter()
            .map(|section| (&section.name, section.address, section.size))
            .collect();
        let values = serde_json::to_value(values).unwrap();
        if let Some(expected) = &expected {
            assert_eq!(&values, expected, "{format:?}");
        } else {
            expected = Some(values);
        }
        if format == MapFormat::TexasCgt {
            let copied = sections
                .iter()
                .find(|section| section.name == ".ram_code")
                .unwrap();
            assert_eq!(copied.address, 0x20000000);
            assert_eq!(copied.load_address, Some(0x08000598));
        }
    }
}

#[test]
fn generic_matching_selects_rejects_and_disambiguates_each_format() {
    use firmware_analysis_core::map::MapFormat;
    for format in [MapFormat::GnuLd, MapFormat::TexasCgt, MapFormat::LlvmLld] {
        let dir = Temp::new();
        let elf = dir.0.join("app.elf");
        fs::write(
            &elf,
            include_bytes!("../../../fixtures/build/cortex-m-stripped.elf"),
        )
        .unwrap();
        let map = dir.0.join("renamed.map");
        let text = matching_section_map(format);
        fs::write(&map, &text).unwrap();
        let build = scan_folder(&dir.0).unwrap();
        assert_eq!(
            build.matching_map(&elf),
            Some(build.root.join("renamed.map").as_path()),
            "{format:?}"
        );
        fs::write(dir.0.join("duplicate.map"), &text).unwrap();
        let build = scan_folder(&dir.0).unwrap();
        assert!(build.matching_map(&elf).is_none(), "{format:?}");
        fs::remove_file(dir.0.join("duplicate.map")).unwrap();
        fs::rename(&map, dir.0.join("app.map")).unwrap();
        // The same generic matcher rejects stale layouts even with matching names.
        fs::write(
            &elf,
            include_bytes!("../../../fixtures/build/cortex-m-grown.elf"),
        )
        .unwrap();
        let build = scan_folder(&dir.0).unwrap();
        assert!(build.matching_map(&elf).is_none(), "{format:?}");
    }
}

#[test]
fn malformed_ti_section_rows_and_unsupported_targets_are_not_filename_fallbacks() {
    use firmware_analysis_core::map::{parse_map_sections, MapFormat};
    let text = matching_section_map(MapFormat::TexasCgt);
    for text in [
        text.replace("0 08000000 00000540", "0 BAD_ADDRESS 00000540"),
        text.replace("RUN ADDR = 20000000", "RUN ADDR = INVALID"),
        text.replace("ARM Linker", "TMS320C2800 Linker"),
        text.replace("0 08000000 00000540", "0 ffffffffffffffff 00000540"),
    ] {
        assert!(parse_map_sections(&text).is_err());
        let dir = Temp::new();
        fs::write(
            dir.0.join("app.elf"),
            include_bytes!("../../../fixtures/build/cortex-m.elf"),
        )
        .unwrap();
        fs::write(dir.0.join("app.map"), text).unwrap();
        let build = scan_folder(&dir.0).unwrap();
        assert!(build.matching_map(&build.root.join("app.elf")).is_none());
    }
    assert!(parse_map_sections(TI_MAP).unwrap().is_none());
    assert!(parse_map_sections("unknown format").unwrap().is_none());
    let empty = TI_MAP.replace(
        "not a memory region\n",
        "section page origin length input sections\n",
    );
    assert_eq!(parse_map_sections(&empty).unwrap(), Some(vec![]));
}

#[test]
fn ti_common_sections_preserve_run_addresses_and_custom_names() {
    use firmware_analysis_core::map::parse_map_sections;
    let text = TI_MAP.replace("not a memory region\n", "output attributes/\nsection page origin length input sections\n-------- ---- ---------- ---------- ----------------\n.text 0 08000000 00000138\n 08000000 000000a0 ctrl.obj (.text)\nabc 0 20000000 00000020 UNINITIALIZED\nram_code\n 0 08000138 0000001c RUN ADDR = 20000020\n 08000138 0000001c main.obj (.text)\nGLOBAL SYMBOLS\n08000000 Reset_Handler\n");
    let rows = parse_map_sections(&text).unwrap().unwrap();
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0].name, ".text");
    assert_eq!(rows[0].address, 0x08000000);
    assert_eq!(rows[0].size, 0x138);
    assert_eq!(rows[1].name, "abc");
    assert_eq!(rows[2].address, 0x20000020);
    assert_eq!(rows[2].load_address, Some(0x08000138));
}

#[test]
fn conflicting_load_addresses_reject_maps_with_matching_runtime_sections() {
    use firmware_analysis_core::map::MapFormat;
    for format in [MapFormat::GnuLd, MapFormat::TexasCgt, MapFormat::LlvmLld] {
        let dir = Temp::new();
        let elf = dir.0.join("app.elf");
        fs::write(&elf, include_bytes!("../../../fixtures/build/cortex-m.elf")).unwrap();
        let map = dir.0.join("app.map");
        let text = matching_section_map(format);
        fs::write(&map, &text).unwrap();
        let build = scan_folder(&dir.0).unwrap();
        assert_eq!(
            build.matching_map(&elf),
            Some(build.root.join("app.map").as_path()),
            "{format:?}"
        );
        let stale = match format {
            MapFormat::GnuLd => text.replace("load address 0x8000598", "load address 0x8001598"),
            MapFormat::TexasCgt => text.replace("0 08000598", "0 08001598"),
            MapFormat::LlvmLld => text.replace("20000000 8000598", "20000000 8001598"),
            _ => unreachable!(),
        };
        assert_ne!(text, stale, "{format:?}");
        fs::write(&map, stale).unwrap();
        assert!(build.matching_map(&elf).is_none(), "{format:?}");
        let report = analyze_build_firmware(&build, &elf, None).unwrap();
        assert!(report.options.regions.is_empty());
        assert!(report.dependencies.map_path.is_none());
    }
}

#[test]
fn implicit_load_addresses_reject_stale_maps_for_copied_sections() {
    use firmware_analysis_core::map::MapFormat;
    for format in [MapFormat::GnuLd, MapFormat::TexasCgt] {
        let dir = Temp::new();
        let elf = dir.0.join("app.elf");
        fs::write(&elf, include_bytes!("../../../fixtures/build/cortex-m.elf")).unwrap();
        let map = dir.0.join("app.map");
        let text = matching_section_map(format);
        // The stale map places .ram_code directly in RAM, with no separate Flash
        // load placement. Its runtime name/address/size still match the ELF.
        let stale = match format {
            MapFormat::GnuLd => text.replace(" load address 0x8000598", ""),
            MapFormat::TexasCgt => text.replace(
                "0 08000598 0000001c RUN ADDR = 20000000",
                "0 20000000 0000001c",
            ),
            _ => unreachable!(),
        };
        assert_ne!(text, stale, "{format:?}");
        fs::write(&map, stale).unwrap();
        let build = scan_folder(&dir.0).unwrap();
        assert!(build.matching_map(&elf).is_none(), "{format:?}");
        let report = analyze_build_firmware(&build, &elf, None).unwrap();
        assert!(report.options.regions.is_empty());
        assert!(report.dependencies.map_path.is_none());
    }
}
