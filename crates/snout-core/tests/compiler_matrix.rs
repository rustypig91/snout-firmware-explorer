//! Real compiler output, including TI's marked continuation rows for custom names.
use snout_core::{
    build::{
        analyze_build_firmware, map_match_issues, scan_folder, Artifact, ArtifactKind, BuildFolder,
    },
    compare::compare,
    map::{detect_map_format, parse_map_sections, MapFormat},
};
use std::{fs, path::PathBuf};

fn fixture_dir(toolchain: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/build")
        .join(toolchain)
}

#[test]
fn real_compiler_matrix_matches_sections_and_preserves_memory_semantics() {
    for (toolchain, format) in [
        ("gcc", MapFormat::GnuLd),
        ("llvm", MapFormat::LlvmLld),
        ("ti-cgt", MapFormat::TexasCgt),
        ("ti-clang", MapFormat::TexasCgt),
    ] {
        let build = scan_folder(fixture_dir(toolchain)).unwrap();
        let mut reports = Vec::new();
        for image in ["cortex-m", "cortex-m-grown", "cortex-m-stripped"] {
            let elf = build.root.join(format!("{image}.elf"));
            let map = build.root.join(format!("{image}.map"));
            let text = fs::read_to_string(&map).unwrap();
            assert_eq!(detect_map_format(&text), format);
            let issues = map_match_issues(&fs::read(&elf).unwrap(), &text);
            assert!(issues.is_empty(), "{toolchain}/{image}: {issues:?}");
            assert_eq!(build.matching_map(&elf), Some(map.as_path()));
            let report = analyze_build_firmware(&build, &elf, None).unwrap();
            let custom = report
                .sections
                .iter()
                .find(|s| {
                    s.name
                        == if toolchain == "gcc" {
                            ".ram_code"
                        } else {
                            ".sensor_calibration_code"
                        }
                })
                .unwrap();
            assert!(custom.size > 0);
            assert!(custom.address >= 0x20000000 && custom.address < 0x20010000);
            let load = custom.load_address.unwrap();
            assert!((0x08000000..0x08040000).contains(&load));
            if format == MapFormat::LlvmLld {
                assert!(report.options.regions.is_empty());
            } else {
                assert_eq!(report.options.regions.len(), 2);
                assert_eq!(report.options.regions[0].size, 256 * 1024);
                assert_eq!(report.options.regions[1].size, 64 * 1024);
            }
            if image == "cortex-m-stripped" {
                assert!(report.symbols.is_empty(), "{toolchain}");
                assert!(!report.metadata.has_dwarf, "{toolchain}");
            } else {
                assert!(
                    report.symbols.iter().any(|s| s.name
                        == if toolchain == "gcc" {
                            "ram_function"
                        } else {
                            "sensor_calibration_bias"
                        }),
                    "{toolchain}"
                );
            }
            reports.push(report);
        }
        assert_eq!(
            compare(&reports[0], &reports[1]).ram_delta,
            32,
            "{toolchain}"
        );
        assert_eq!(
            compare(&reports[0], &reports[2]).flash_delta,
            0,
            "{toolchain}"
        );
        assert_eq!(
            compare(&reports[0], &reports[2]).ram_delta,
            0,
            "{toolchain}"
        );

        // Filename-independent selection with only one candidate map; the
        // baseline/stripped maps in the full scan intentionally share a layout.
        let renamed = BuildFolder {
            root: build.root.clone(),
            artifacts: vec![Artifact {
                path: build.root.join("cortex-m-stripped.map"),
                kind: ArtifactKind::Map,
            }],
            warnings: vec![],
        };
        let elf = build.root.join("cortex-m.elf");
        assert_eq!(
            renamed.matching_map(&elf),
            Some(renamed.artifacts[0].path.as_path()),
            "{toolchain}"
        );
    }
}

#[test]
fn ti_marked_wrapped_rows_keep_load_run_placements_and_stop_at_module_summary() {
    for toolchain in ["ti-cgt", "ti-clang"] {
        let text = fs::read_to_string(fixture_dir(toolchain).join("cortex-m.map")).unwrap();
        assert!(text
            .lines()
            .zip(text.lines().skip(1))
            .any(|(name, placement)| {
                name == ".sensor_calibration_code" && placement.starts_with('*')
            }));
        assert!(text.contains("MODULE SUMMARY"));
        // Exercise both checkout styles, regardless of the host's line endings.
        let lf = text.replace("\r\n", "\n");
        let crlf = lf.replace('\n', "\r\n");
        for text in [&lf, &crlf] {
            let rows = parse_map_sections(text).unwrap().unwrap();
            let custom = rows
                .iter()
                .find(|row| row.name == ".sensor_calibration_code")
                .unwrap();
            assert_ne!(Some(custom.address), custom.load_address);
            assert!(!rows.iter().any(|row| row.name == "MODULE"));
            let malformed = text.replacen("*          0", "*          invalid", 1);
            assert!(parse_map_sections(&malformed).is_err());
        }
    }
}
