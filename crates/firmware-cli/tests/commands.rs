use std::process::{Command, Output, Stdio};
fn fixture(name: &str) -> String {
    format!("{}/../../fixtures/build/{name}", env!("CARGO_MANIFEST_DIR"))
}
fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_firmware-explorer"))
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn analysis_commands_emit_versioned_json() {
    for command in ["analyze", "files", "symbols"] {
        let output = run(&[command, &fixture("cortex-m.elf"), "--format", "json"]);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(json["schema_version"], 1);
        assert_eq!(json["totals"]["flash"], 1484);
        assert!(json["warnings"].is_array());
    }
}
#[test]
fn diff_reports_signed_growth() {
    let output = run(&[
        "diff",
        &fixture("cortex-m.elf"),
        &fixture("cortex-m-grown.elf"),
        "--format",
        "json",
    ]);
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["ram_delta"], 32);
    assert_eq!(json["flash_delta"], 0);
}
#[test]
fn stack_command_exposes_uncertainty() {
    let output = run(&[
        "stack",
        &fixture("cortex-m.elf"),
        "--stack-usage",
        &fixture("CMakeFiles/cortex-m-objects.dir/src/main.c.su"),
        "--format",
        "json",
    ]);
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["call_graph"]["complete"], false);
    assert!(!json["entries"].as_array().unwrap().is_empty());
}
#[test]
fn bad_input_exits_with_an_actionable_error() {
    let output = run(&["analyze", &fixture("../src/main.c")]);
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(error.contains("Invalid ELF"));
    assert!(error.contains("ELF file"));
}

#[test]
fn closed_output_pipes_exit_successfully_for_text_and_json() {
    for format in ["text", "json"] {
        let mut child = Command::new(env!("CARGO_BIN_EXE_firmware-explorer"))
            .args(["symbols", &fixture("cortex-m.elf"), "--format", format])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        drop(child.stdout.take());
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{format}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.stderr.is_empty());
    }
}
#[test]
fn configured_layout_and_text_output_work() {
    let config = format!(
        "{}/../../fixtures/build/cortex-m.map",
        env!("CARGO_MANIFEST_DIR")
    );
    let output = run(&["analyze", &fixture("cortex-m.elf"), "--map", &config]);
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("1.45 KiB"));
    assert!(text.contains("356 B"));
}

#[test]
fn explicit_map_matches_build_folder_graph_for_all_report_commands() {
    let build = firmware_analysis_core::build::scan_folder(fixture("")).unwrap();
    let expected = firmware_analysis_core::build::analyze_build_firmware(
        &build,
        &build.root.join("cortex-m.elf"),
        None,
    )
    .unwrap();
    for command in ["analyze", "files", "symbols"] {
        let output = run(&[
            command,
            &fixture("cortex-m.elf"),
            "--map",
            &fixture("cortex-m.map"),
            "--format",
            "json",
        ]);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let report: firmware_analysis_core::Analysis =
            serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(report.dependencies.nodes.len(), 6);
        assert_eq!(report.dependencies.edges.len(), 16);
        assert_eq!(report.options, expected.options);
        assert_eq!(
            serde_json::to_value(&report.dependencies.nodes).unwrap(),
            serde_json::to_value(&expected.dependencies.nodes).unwrap()
        );
        assert_eq!(
            serde_json::to_value(&report.dependencies.edges).unwrap(),
            serde_json::to_value(&expected.dependencies.edges).unwrap()
        );
        assert_eq!(
            report.dependencies.map_path.as_deref(),
            Some(fixture("cortex-m.map").as_str())
        );
        assert_eq!(report.dependencies.notes, expected.dependencies.notes);
    }
}

#[test]
fn elf_only_does_not_automatically_import_a_sibling_map() {
    let output = run(&["analyze", &fixture("cortex-m.elf"), "--format", "json"]);
    let report: firmware_analysis_core::Analysis = serde_json::from_slice(&output.stdout).unwrap();
    assert!(report.options.regions.is_empty());
    assert!(report.dependencies.edges.is_empty());
    assert!(report.dependencies.map_path.is_none());
}

#[test]
fn llvm_map_imports_connections_with_unknown_capacity() {
    let output = run(&[
        "analyze",
        &fixture("../maps/llvm-lld.elf"),
        "--map",
        &fixture("../maps/llvm-lld.map"),
        "--format",
        "json",
    ]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: firmware_analysis_core::Analysis = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report.dependencies.edges.len(), 16);
    assert!(report.options.regions.is_empty());
    assert!(report
        .warnings
        .iter()
        .any(|note| note.contains("capacity remains unknown")));
}

#[test]
fn invalid_maps_fail_without_emitting_a_partial_report() {
    for (map, diagnostic) in [
        ("missing.map", "Cannot read linker map"),
        ("../src/main.c", "Unknown or ambiguous linker map format"),
    ] {
        let output = run(&["analyze", &fixture("cortex-m.elf"), "--map", &fixture(map)]);
        assert_eq!(output.status.code(), Some(1));
        assert!(output.stdout.is_empty());
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(error.contains(diagnostic), "{error}");
        assert!(error.contains(&fixture(map)));
    }
    let output = run(&[
        "analyze",
        &fixture("cortex-m.elf"),
        "--config",
        "layout.json",
    ]);
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn comparison_accepts_independent_maps_and_help_documents_them() {
    let output = run(&[
        "diff",
        &fixture("cortex-m.elf"),
        &fixture("cortex-m-grown.elf"),
        "--old-map",
        &fixture("cortex-m.map"),
        "--new-map",
        &fixture("cortex-m-grown.map"),
        "--format",
        "json",
    ]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["ram_delta"], 32);
    for command in ["analyze", "files", "symbols", "stack", "diff"] {
        let output = run(&[command, "--help"]);
        assert!(output.status.success());
        let help = String::from_utf8(output.stdout).unwrap();
        assert!(help.contains(if command == "diff" {
            "--old-map"
        } else {
            "--map"
        }));
        assert!(!help.contains("--config"));
    }
}

#[test]
fn capacity_only_and_malformed_cross_reference_maps_have_defined_behavior() {
    let path = std::env::temp_dir().join(format!("snout-cli-map-{}.map", std::process::id()));
    let original = std::fs::read_to_string(fixture("cortex-m.map")).unwrap();
    let regions_only = original.split("Cross Reference Table").next().unwrap();
    std::fs::write(&path, regions_only).unwrap();
    let output = run(&[
        "analyze",
        &fixture("cortex-m.elf"),
        "--map",
        path.to_str().unwrap(),
        "--format",
        "json",
    ]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: firmware_analysis_core::Analysis = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report.options.regions.len(), 2);
    assert!(report.dependencies.edges.is_empty());
    assert_eq!(report.dependencies.map_path.as_deref(), path.to_str());
    assert!(report
        .dependencies
        .notes
        .iter()
        .any(|note| note.contains("--cref")));
    std::fs::write(
        &path,
        format!("{regions_only}\nCross Reference Table\nBroken Header\n"),
    )
    .unwrap();
    let output = run(&[
        "analyze",
        &fixture("cortex-m.elf"),
        "--map",
        path.to_str().unwrap(),
    ]);
    std::fs::remove_file(&path).unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(error.contains("Unsupported cross-reference header"));
    assert!(error.contains("--cref"));
}
