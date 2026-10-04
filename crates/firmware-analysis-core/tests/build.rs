use firmware_analysis_core::build::{
    analyze_build_firmware, parse_map_regions, scan_folder, ArtifactKind,
};
use std::{fs, path::PathBuf};

const MAP: &str = "Memory Configuration\n\nName             Origin             Length             Attributes\nFLASH            0x08000000         0x00040000         xr\nRAM              0x20000000         0x00010000         xrw\n*default*        0x00000000         0xffffffff\n\nLinker script and memory map\n";

#[test]
fn committed_fixtures_import_matching_map_capacities() {
    let dir = Temp::new();
    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures");
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
        include_bytes!("../../../fixtures/cortex-m.elf"),
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
        include_bytes!("../../../examples/cortex-m-memory.json"),
    )
    .unwrap();
    fs::write(dir.0.join("other.json"), "{}").unwrap();
    fs::write(dir.0.join("fake.out"), "not ELF").unwrap();
    let build = scan_folder(&dir.0).unwrap();
    assert_eq!(build.artifacts.len(), 4);
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
        include_bytes!("../../../fixtures/cortex-m.elf"),
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
        include_bytes!("../../../fixtures/cortex-m.elf"),
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
    let build =
        scan_folder(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures")).unwrap();
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
        let stack = firmware_analysis_core::stack::analyze_stack(&analysis, &build.root).unwrap();
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
