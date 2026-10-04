use firmware_analysis_core::{analyze_bytes, stack::analyze_stack_files, AnalysisOptions};
use std::{fs, path::PathBuf};

struct ReportFile(PathBuf);

impl Drop for ReportFile {
    fn drop(&mut self) {
        fs::remove_file(&self.0).unwrap();
    }
}

#[test]
fn parent_relative_sources_keep_stack_candidates_without_matching_other_absolute_sources() {
    let analysis = analyze_bytes(
        include_bytes!("../../../fixtures/cortex-m.elf"),
        "fixture",
        &AnalysisOptions::default(),
    )
    .unwrap();
    let file = ReportFile(std::env::temp_dir().join(format!(
        "snout-stack-{}-{}.su",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    )));
    fs::write(
        &file.0,
        "../fixtures/src/main.c:16:10:cpp_function\t16\tstatic\n\
         /other/fixtures/src/main.c:16:10:cpp_function\t16\tstatic\n",
    )
    .unwrap();
    let report = analyze_stack_files(&analysis, vec![file.0.clone()]).unwrap();
    assert_eq!(report.entries.len(), 2);
    let relative = report
        .entries
        .iter()
        .find(|entry| entry.source_file.starts_with(".."))
        .unwrap();
    assert_eq!(
        relative.symbol_candidates,
        ["_Z12cpp_functionj @ 0x800001d"]
    );
    let conflicting = report
        .entries
        .iter()
        .find(|entry| entry.source_file.starts_with('/'))
        .unwrap();
    assert!(conflicting.symbol_candidates.is_empty());
}
