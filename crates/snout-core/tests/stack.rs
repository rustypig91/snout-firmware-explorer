use snout_core::{analyze_bytes, stack::analyze_stack_files, AnalysisOptions};
use std::{fs, path::PathBuf};

struct ReportFile(PathBuf);

impl Drop for ReportFile {
    fn drop(&mut self) {
        fs::remove_file(&self.0).unwrap();
    }
}

#[test]
fn reusable_index_preserves_duplicates_source_fallback_and_overload_ambiguity() {
    use snout_core::stack::StackAnalyzer;
    let mut analysis = analyze_bytes(
        include_bytes!("../../../fixtures/build/gcc/cortex-m.elf"),
        "fixture",
        &Default::default(),
    )
    .unwrap();
    let template = analysis
        .symbols
        .iter()
        .find(|s| s.kind == "Function")
        .unwrap()
        .clone();
    analysis.symbols = [
        ("plain", "plain", "/project/src/a.c", 10),
        ("plain", "plain", "/other/src/a.c", 10),
        ("_ZoverInt", "over(int)", "/project/src/a.c", 20),
        ("_ZoverFloat", "over(float)", "/project/src/a.c", 30),
    ]
    .into_iter()
    .enumerate()
    .map(|(i, (name, demangled, source, line))| {
        let mut symbol = template.clone();
        symbol.name = name.into();
        symbol.demangled_name = demangled.into();
        symbol.source_file = Some(source.into());
        symbol.source_line = Some(line);
        symbol.address = 100 + i as u64 * 4;
        symbol
    })
    .collect();
    let file = ReportFile(
        std::env::temp_dir().join(format!("snout-stack-index-{}.su", std::process::id())),
    );
    fs::write(
        &file.0,
        concat!(
            "src/a.c:10:1:plain\t32\tstatic\n",
            "/project/src/a.c:10:1:plain\t32\tstatic\n",
            "src/a.c:20:1:renamed\t32\tstatic\n",
            "src/a.c:99:1:over\t32\tstatic\n",
            "/unrelated/a.c:10:1:plain\t32\tstatic\n",
        ),
    )
    .unwrap();
    let index = StackAnalyzer::new(&analysis);
    for _ in 0..2 {
        let report = index.analyze_files(vec![file.0.clone()]).unwrap();
        let candidates: Vec<_> = report
            .entries
            .iter()
            .map(|e| e.symbol_candidates.clone())
            .collect();
        assert_eq!(
            candidates,
            vec![
                vec!["plain @ 0x64", "plain @ 0x68"],
                vec!["plain @ 0x64"],
                vec!["_ZoverInt @ 0x6c"],
                vec!["_ZoverInt @ 0x6c", "_ZoverFloat @ 0x70"],
                vec![],
            ]
        );
    }
}

#[test]
fn parent_relative_sources_keep_stack_candidates_without_matching_other_absolute_sources() {
    let analysis = analyze_bytes(
        include_bytes!("../../../fixtures/build/gcc/cortex-m.elf"),
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

#[test]
#[ignore = "manual startup performance measurement"]
fn large_stack_analysis_latency() {
    let mut analysis = analyze_bytes(
        include_bytes!("../../../fixtures/build/gcc/cortex-m.elf"),
        "fixture",
        &Default::default(),
    )
    .unwrap();
    let template = analysis
        .symbols
        .iter()
        .find(|s| s.kind == "Function")
        .unwrap()
        .clone();
    analysis.symbols = (0..2000)
        .map(|i| {
            let mut s = template.clone();
            s.name = format!("function_{i}");
            s.demangled_name = s.name.clone();
            s.source_file = Some(format!("/project/src/unit_{i}.c"));
            s.source_line = Some(1);
            s
        })
        .collect();
    let file = ReportFile(
        std::env::temp_dir().join(format!("snout-stack-perf-{}.su", std::process::id())),
    );
    let text: String = (0..2000)
        .map(|i| format!("src/unit_{i}.c:1:1:function_{i}\t32\tstatic\n"))
        .collect();
    fs::write(&file.0, text).unwrap();
    let start = std::time::Instant::now();
    let report = analyze_stack_files(&analysis, vec![file.0.clone()]).unwrap();
    println!(
        "2,000 functions and stack entries: {:.3} ms",
        start.elapsed().as_secs_f64() * 1000.0
    );
    assert_eq!(report.entries.len(), 2000);
    assert!(report
        .entries
        .iter()
        .all(|e| e.symbol_candidates.len() == 1));
}
