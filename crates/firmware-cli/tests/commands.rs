use std::process::{Command, Output, Stdio};
fn fixture(name: &str) -> String {
    format!("{}/../../fixtures/{name}", env!("CARGO_MANIFEST_DIR"))
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
        assert_eq!(json["totals"]["flash"], 260);
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
        &fixture("cortex-m-main.su"),
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
    let output = run(&["analyze", &fixture("src/main.c")]);
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
        "{}/../../examples/cortex-m-memory.json",
        env!("CARGO_MANIFEST_DIR")
    );
    let output = run(&["analyze", &fixture("cortex-m.elf"), "--config", &config]);
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("260 B"));
    assert!(text.contains("232 B"));
}
