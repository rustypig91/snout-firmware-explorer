use firmware_analysis_core::build::{
    analyze_build_firmware, parse_map_regions, scan_folder, ArtifactKind,
};
use std::{fs, path::PathBuf};

const MAP: &str = "Memory Configuration\n\nName             Origin             Length             Attributes\nFLASH            0x08000000         0x00040000         xr\nRAM              0x20000000         0x00010000         xrw\n*default*        0x00000000         0xffffffff\n\nLinker script and memory map\n";
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
    fs::write(
        dir.0.join("layout.json"),
        include_bytes!("../../../examples/cortex-m-memory.json"),
    )
    .unwrap();
    fs::write(dir.0.join("other.json"), "{}").unwrap();
    fs::write(dir.0.join("fake.out"), "not ELF").unwrap();
    let build = scan_folder(&dir.0).unwrap();
    assert_eq!(build.artifacts.len(), 5);
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
