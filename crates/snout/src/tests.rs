use super::*;
use snout_core::stack::analyze_stack;

fn finish_job(app: &mut Explorer) {
    let result = app
        .receiver
        .take()
        .unwrap()
        .recv_timeout(Duration::from_secs(5))
        .unwrap();
    let (sender, receiver) = mpsc::channel();
    sender.send(result).unwrap();
    app.receiver = Some(receiver);
    app.poll();
}

#[test]
fn interface_text_respects_minimum_size_except_zoomable_graph_labels() {
    let ctx = egui::Context::default();
    ctx.style_mut(|style| {
        style.text_styles.insert(
            egui::TextStyle::Name("tiny".into()),
            egui::FontId::proportional(6.0),
        );
    });
    shell::configure_style(&ctx);
    assert!(ctx
        .style()
        .text_styles
        .values()
        .all(|font| font.size >= shell::MIN_TEXT_SIZE));
    assert_eq!(ctx.style().text_styles[&egui::TextStyle::Small].size, 12.0);
    assert_eq!(ctx.style().text_styles[&egui::TextStyle::Body].size, 14.0);
    let analysis = snout_core::analyze_bytes(
        include_bytes!("../../../fixtures/build/gcc/cortex-m.elf"),
        "test.elf",
        &Default::default(),
    )
    .unwrap();
    let mut app = Explorer {
        analysis: Some(Arc::new(analysis)),
        ..Default::default()
    };
    for view in View::ALL {
        app.change_view(view);
        let frame = |app: &mut Explorer| {
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1400.0, 1000.0),
                    )),
                    ..Default::default()
                },
                |ctx| app.show(ctx),
            )
        };
        let output = if view == View::Dependencies {
            dependencies::settle_graph(&mut app, frame)
        } else {
            frame(&mut app)
        };
        for shape in &output.shapes {
            if let egui::Shape::Text(text) = &shape.shape {
                // Diagram labels scale with zoom; surrounding UI stays readable.
                if view == View::Dependencies
                    && (text.galley.text().contains("\nFlash ")
                        || text.galley.text().contains("\nRAM "))
                {
                    continue;
                }

                assert!(
                    text.galley
                        .job
                        .sections
                        .iter()
                        .all(|section| section.format.font_id.size >= shell::MIN_TEXT_SIZE),
                    "{}: {}",
                    view.label(),
                    text.galley.text()
                );
            }
        }
    }
}

#[test]
fn changing_layout_reloads_stack_reports_against_the_current_elf() {
    for use_map in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().canonicalize().unwrap();
        let elf = root.join("app.elf");
        let report = root.join("app.su");
        let layout = root.join(if use_map { "app.map" } else { "layout.map" });
        std::fs::write(
            &elf,
            include_bytes!("../../../fixtures/build/gcc/cortex-m.elf"),
        )
        .unwrap();
        std::fs::write(&report, "diag.c:22:36:diagnose\t24\tstatic\n").unwrap();
        std::fs::write(
            &layout,
            if use_map {
                "Memory Configuration\nName Origin Length Attributes\nFLASH 0x08000000 0x10000 xr\nRAM 0x20000000 0x10000 xrw\nLinker script and memory map\n"
            } else {
                include_str!("../../../fixtures/build/gcc/cortex-m.map")
            },
        )
        .unwrap();
        let mut app = Explorer::default();
        app.scan_build(root);
        finish_job(&mut app);
        app.open(elf.clone());
        finish_job(&mut app);
        app.select_stack_reports(vec![report.clone()]);
        finish_job(&mut app);
        assert_eq!(
            app.stack.as_ref().unwrap().entries[0]
                .symbol_candidates
                .len(),
            1
        );

        // A build has changed on disk before the user applies a memory layout.
        std::fs::write(
            &elf,
            include_bytes!("../../../fixtures/build/gcc/cortex-m-stripped.elf"),
        )
        .unwrap();
        std::fs::write(&report, "diag.c:22:36:diagnose\t96\tstatic\n").unwrap();
        if use_map {
            app.apply_map(layout.clone());
        } else {
            app.configure(Some(layout.clone()));
        }
        finish_map_job(&mut app);
        assert!(app.analysis.as_ref().unwrap().symbols.is_empty());
        let stack = app.stack.as_ref().unwrap();
        assert!(stack.entries[0].symbol_candidates.is_empty());
        assert_eq!(stack.entries[0].local_bytes, 96);
        assert_eq!(app.saved_stack_reports(&elf), Some(vec![report.clone()]));

        // Failed report reads must not retain matches from the previous ELF snapshot.
        std::fs::write(&report, [0xff]).unwrap();
        app.configure(None);
        finish_job(&mut app);
        assert!(app.stack.is_none());
        assert!(app
            .analysis
            .as_ref()
            .unwrap()
            .warnings
            .iter()
            .any(|w| w.contains("Stack reports could not be loaded")));
        assert_eq!(app.saved_stack_reports(&elf), Some(vec![report]));
    }
}

#[test]
fn changing_layout_rescans_stack_reports_after_a_rebuild() {
    for use_map in [false, true] {
        for remove_old in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path().canonicalize().unwrap();
            let elf = root.join("app.elf");
            let reports = root.join("reports");
            std::fs::create_dir(&reports).unwrap();
            let old_report = reports.join("old.su");
            let new_report = reports.join("new.su");
            let layout = root.join(if use_map { "app.map" } else { "layout.map" });
            std::fs::write(
                &elf,
                include_bytes!("../../../fixtures/build/gcc/cortex-m.elf"),
            )
            .unwrap();
            std::fs::write(&old_report, "diag.c:22:36:diagnose\t24\tstatic\n").unwrap();
            std::fs::write(
                &layout,
                if use_map {
                    "Memory Configuration\nName Origin Length Attributes\nFLASH 0x08000000 0x10000 xr\nRAM 0x20000000 0x10000 xrw\nLinker script and memory map\n"
                } else {
                    include_str!("../../../fixtures/build/gcc/cortex-m.map")
                },
            )
            .unwrap();
            let mut app = Explorer::default();
            app.scan_build(root);
            finish_job(&mut app);
            app.open(elf.clone());
            finish_job(&mut app);
            let selection = workspace::StackSelection {
                paths: vec![reports.clone()],
                ..Default::default()
            };
            app.select_stack_selection(selection.clone());
            finish_job(&mut app);
            assert_eq!(app.stack.as_ref().unwrap().entries.len(), 1);

            std::fs::write(&new_report, "sensor.c:9:1:sensor_init\t96\tstatic\n").unwrap();
            if remove_old {
                std::fs::remove_file(&old_report).unwrap();
            }
            if use_map {
                app.apply_map(layout);
            } else {
                app.configure(Some(layout));
            }
            finish_map_job(&mut app);
            let stack = app
                .stack
                .as_ref()
                .expect("deleted reports must not block reload");
            assert_eq!(stack.entries.len(), if remove_old { 1 } else { 2 });
            assert!(stack.entries.iter().any(|entry| entry.local_bytes == 96));
            assert_eq!(app.saved_stack_selection(&elf), Some(selection));
            let artifacts = &app.build.as_ref().unwrap().artifacts;
            assert!(artifacts.iter().any(|artifact| artifact.path == new_report));
            assert_eq!(
                artifacts.iter().any(|artifact| artifact.path == old_report),
                !remove_old
            );
        }
    }
}

#[test]
fn symbol_navigation_clears_filters_even_when_already_in_symbols() {
    let mut app = Explorer {
        view: View::Symbols,
        search: "no-match".into(),
        kind_filter: "Label".into(),
        details: Some(("old".into(), "detail".into())),
        ..Default::default()
    };
    app.show_file_symbols("src/main.c".into());
    assert_eq!(app.selected_file.as_deref(), Some("src/main.c"));
    assert!(app.search.is_empty());
    assert_eq!(app.kind_filter, "All");
    assert!(app.details.is_none());
}

#[test]
fn same_named_symbols_at_different_addresses_expand_independently() {
    let mut analysis = analyze_path(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/build/gcc/cortex-m.elf"),
        &Default::default(),
    )
    .unwrap();
    let mut symbols: Vec<_> = analysis
        .symbols
        .iter()
        .filter(|symbol| symbol.kind == "Global")
        .take(2)
        .cloned()
        .collect();
    assert_eq!(symbols.len(), 2);
    assert_ne!(symbols[0].address, symbols[1].address);
    for symbol in &mut symbols {
        symbol.name = "shared_local".into();
        symbol.demangled_name = "shared_local".into();
        symbol.weak = false;
        symbol.source_file = None;
        symbol.source_line = None;
        symbol.compilation_unit = None;
        symbol.attribution = "Unknown owner".into();
    }
    symbols.sort_by_key(|symbol| symbol.address);
    let addresses: Vec<_> = symbols.iter().map(|symbol| symbol.address).collect();
    analysis.symbols = symbols;
    let mut app = Explorer {
        analysis: Some(Arc::new(analysis)),
        view: View::Symbols,
        sort_column: 6,
        descending: false,
        ..Default::default()
    };
    let ctx = egui::Context::default();
    for address in addresses {
        let _ = ctx.run(
            egui::RawInput {
                events: [true, false]
                    .map(|pressed| egui::Event::Key {
                        key: egui::Key::ArrowDown,
                        physical_key: None,
                        pressed,
                        repeat: false,
                        modifiers: egui::Modifiers::NONE,
                    })
                    .into(),
                ..Default::default()
            },
            |ctx| app.show(ctx),
        );
        assert_eq!(app.visible_rows, 2);
        assert_eq!(app.details.as_ref().unwrap().0, "shared_local");
        assert!(app
            .details
            .as_ref()
            .unwrap()
            .1
            .contains(&format!("Address: {address:#010x}")));
    }
}

#[test]
fn refresh_reloads_uppercase_maps_and_selected_maps_and_keeps_comparison() {
    let folder = tempfile::tempdir().unwrap();
    let elf = folder.path().join("app.elf");
    std::fs::write(
        &elf,
        include_bytes!("../../../fixtures/build/gcc/cortex-m.elf"),
    )
    .unwrap();
    let map = folder.path().join("manual.MAP");
    std::fs::write(
        &map,
        include_bytes!("../../../fixtures/build/gcc/cortex-m.map"),
    )
    .unwrap();
    let layout = folder.path().join("memory.MAP");
    std::fs::write(
        &layout,
        include_bytes!("../../../fixtures/build/gcc/cortex-m.map"),
    )
    .unwrap();
    let mut app = Explorer {
        preferences_file: Some(folder.path().join("workspace.json")),
        ..Explorer::default()
    };
    app.scan_build(folder.path().to_owned());
    finish_job(&mut app);
    app.open(elf.clone());
    finish_job(&mut app);
    for source in [&map, &layout] {
        if source == &map {
            app.apply_map(source.clone());
        } else {
            app.configure(Some(source.clone()));
        }
        finish_map_job(&mut app);
        let name = source.file_name().unwrap().to_str().unwrap().to_owned();
        app.take_snapshot(&name).unwrap();
        app.select_snapshot(Some(name)).unwrap();
        if source == &map {
            std::fs::write(
                source,
                include_str!("../../../fixtures/build/gcc/cortex-m-grown.map").replacen(
                    "0x00040000",
                    "0x00080000",
                    1,
                ),
            )
            .unwrap();
        } else {
            std::fs::write(
                source,
                include_str!("../../../fixtures/build/gcc/cortex-m.map").replacen(
                    "0x00040000",
                    "0x00080000",
                    1,
                ),
            )
            .unwrap();
        }
        let previous = app.options.clone();
        std::fs::write(
            &elf,
            include_bytes!("../../../fixtures/build/gcc/cortex-m-grown.elf"),
        )
        .unwrap();
        app.refresh();
        finish_job(&mut app);
        assert!(app.error.is_none(), "{:?}", app.error);
        assert_ne!(app.options, previous);
        assert!(app.comparison.is_some());
        assert!(app.snapshot_analysis().is_some());
        if source == &map {
            assert_eq!(app.comparison.as_ref().unwrap().ram_delta, 32);
            assert!(app
                .comparison
                .as_ref()
                .unwrap()
                .sections
                .iter()
                .any(|s| s.identity == ".bss"));
        }
    }
}

#[test]
fn stripped_firmware_shows_unresolved_uppercase_stack_reports() {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("report.SU");
    std::fs::write(
        &path,
        include_bytes!("../../../fixtures/build/gcc/CMakeFiles/cortex-m-objects.dir/src/main.c.su"),
    )
    .unwrap();
    let analysis = analyze_path(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/build/gcc/cortex-m-stripped.elf"),
        &Default::default(),
    )
    .unwrap();
    let report = analyze_stack(&analysis, folder.path()).unwrap();
    assert_eq!(report.entries.len(), 7);
    let mut app = Explorer {
        analysis: Some(Arc::new(analysis)),
        stack: Some(report),
        ..Default::default()
    };
    let ctx = egui::Context::default();
    let _ = ctx.run(egui::RawInput::default(), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| app.stack_view(ui));
    });
    assert_eq!(app.visible_rows, 7);
}

#[test]
fn dropping_an_elf_opens_its_folder_and_selects_it() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/build/gcc/cortex-m-grown.elf")
        .canonicalize()
        .unwrap();
    let mut app = Explorer::default();
    let ctx = egui::Context::default();
    let _ = ctx.run(
        egui::RawInput {
            dropped_files: vec![egui::DroppedFile {
                path: Some(path.clone()),
                ..Default::default()
            }],
            ..Default::default()
        },
        |ctx| app.show(ctx),
    );
    finish_job(&mut app);
    finish_job(&mut app);
    assert!(app.error.is_none());
    assert_eq!(PathBuf::from(&app.analysis.as_ref().unwrap().path), path);
}

#[test]
fn active_map_follows_successful_map_selection() {
    let mut app = Explorer::default();
    let folder = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/build/gcc");
    app.scan_build(folder);
    finish_job(&mut app);
    let root = app.build.as_ref().unwrap().root.clone();
    let map = root.join("cortex-m.map");
    let other_map = root.join("cortex-m-grown.map");
    assert!(!app.map_in_use(&map));
    app.open(root.join("cortex-m.elf"));
    finish_job(&mut app);
    assert!(app.map_in_use(&map));
    assert!(!app.map_in_use(&other_map));
    assert!(app.map_in_use(&map));
    assert!(!app.map_in_use(&other_map));
    app.apply_map(other_map.clone());
    finish_map_job(&mut app);
    assert!(!app.map_in_use(&map));
    assert!(app.map_in_use(&other_map));
    app.refresh();
    finish_job(&mut app);
    assert!(app.map_in_use(&other_map));
    app.apply_map(root.join("missing.map"));
    finish_map_job(&mut app);
    assert!(app.error.is_some());
    assert!(app.map_in_use(&other_map));
    let mut restored = Explorer::default();
    restored.apply_preferences(&app.preference_value());
    finish_job(&mut restored);
    finish_job(&mut restored);
    assert!(restored.map_in_use(&other_map));
    app.configure(Some(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/build/gcc/cortex-m.map"),
    ));
    finish_map_job(&mut app);
    assert!(!app.map_in_use(&map));
    assert!(!app.map_in_use(&other_map));
    app.discover_layout();
    finish_job(&mut app);
    assert!(app.map_in_use(&map));
    app.open(root.join("cortex-m-grown.elf"));
    finish_job(&mut app);
    assert!(!app.map_in_use(&map));
    assert!(app.map_in_use(&other_map));
    let empty = tempfile::tempdir().unwrap();
    app.scan_build(empty.path().to_owned());
    finish_job(&mut app);
    assert!(!app.map_in_use(&other_map));
}

#[test]
fn memory_regions_links_to_build_files_and_no_map_choice_survives_restart() {
    let mut app = Explorer::default();
    app.scan_build(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/build/gcc"));
    finish_job(&mut app);
    let root = app.build.as_ref().unwrap().root.clone();
    app.open(root.join("cortex-m.elf"));
    finish_job(&mut app);
    app.change_view(View::BuildFiles);
    let ctx = egui::Context::default();
    let frame = |app: &mut Explorer, events| {
        ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1280.0, 820.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| app.show(ctx),
        )
    };
    let text_pos = |output: &egui::FullOutput, label: &str| {
        output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == label => {
                    Some(text.pos + text.galley.size() * 0.5)
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("Missing text: {label}"))
    };
    let click = |app: &mut Explorer, pos| {
        for pressed in [true, false] {
            frame(
                app,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
    };
    frame(&mut app, vec![]);
    let output = frame(&mut app, vec![]);
    let load = text_pos(&output, "Load map file...");
    let detect = text_pos(&output, "Autodetect map file");
    for name in [
        "cortex-m-grown.map",
        "cortex-m-stripped.map",
        "cortex-m.map",
    ] {
        let pos = text_pos(&output, name);
        assert!(
            pos.y < load.y && pos.y < detect.y,
            "map actions must follow the list"
        );
    }
    click(&mut app, text_pos(&output, "No map selected"));
    finish_job(&mut app);
    assert!(app.options.regions.is_empty());
    assert!(!app.map_in_use(&root.join("cortex-m.map")));
    let mut restored = Explorer::default();
    restored.apply_preferences(&app.preference_value());
    finish_job(&mut restored);
    finish_job(&mut restored);
    assert!(restored.options.regions.is_empty());
    restored.change_view(View::MemoryMap);
    frame(&mut restored, vec![]);
    let output = frame(&mut restored, vec![]);
    text_pos(&output, "No map selected");
    for shape in &output.shapes {
        if let egui::Shape::Text(text) = &shape.shape {
            assert!(![
                "Use ELF inference",
                "Load memory regions...",
                "Discover layout from matching map",
                "Restore map memory regions"
            ]
            .contains(&text.galley.text()));
        }
    }
    click(&mut restored, text_pos(&output, "Open Build files"));
    assert!(restored.view == View::BuildFiles);
    let output = frame(&mut restored, vec![]);
    let label = text_pos(&output, "cortex-m-grown.map");
    let radio = output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Circle(circle)
                if circle.center.x < label.x && (circle.center.y - label.y).abs() < 2.0 =>
            {
                Some(circle.center)
            }
            _ => None,
        })
        .unwrap();
    click(&mut restored, radio);
    finish_map_job(&mut restored);
    assert!(restored.map_in_use(&root.join("cortex-m-grown.map")));
    assert!(!restored.options.regions.is_empty());
    let output = frame(&mut restored, vec![]);
    click(&mut restored, text_pos(&output, "Autodetect map file"));
    finish_job(&mut restored);
    assert!(restored.error.is_none());
    assert!(restored.view == View::BuildFiles);
    assert!(restored.map_in_use(&root.join("cortex-m.map")));
    assert!(restored.layout_override.is_none());
}

#[test]
fn unsupported_matching_map_is_not_marked_in_use() {
    let folder = tempfile::tempdir().unwrap();
    std::fs::write(
        folder.path().join("app.elf"),
        include_bytes!("../../../fixtures/build/gcc/cortex-m.elf"),
    )
    .unwrap();
    std::fs::write(folder.path().join("app.map"), "unsupported map").unwrap();
    let mut app = Explorer::default();
    app.scan_build(folder.path().to_owned());
    finish_job(&mut app);
    let root = app.build.as_ref().unwrap().root.clone();
    app.open(root.join("app.elf"));
    finish_job(&mut app);
    assert!(app.error.is_none());
    assert!(!app.map_in_use(&root.join("app.map")));
}

#[test]
fn map_rediscovery_commits_layout_only_after_successful_analysis() {
    let mut app = Explorer::default();
    app.scan_build(PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/build/gcc"
    )));
    finish_job(&mut app);
    let path = app.build.as_ref().unwrap().root.join("cortex-m.elf");
    app.open(path.clone());
    finish_job(&mut app);
    app.configure(Some(PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/build/gcc/cortex-m.map"
    ))));
    finish_job(&mut app);
    assert!(app.error.is_none());
    let expected = serde_json::to_value(&app.options).unwrap();
    // Simulate firmware becoming unavailable during a rebuild.
    Arc::make_mut(app.analysis.as_mut().unwrap()).path =
        path.with_extension("missing").display().to_string();
    app.discover_layout();
    finish_job(&mut app);
    assert!(app.error.is_some());
    assert_eq!(
        serde_json::to_value(&app.layout_override).unwrap(),
        expected
    );
    assert_eq!(serde_json::to_value(&app.options).unwrap(), expected);
    Arc::make_mut(app.analysis.as_mut().unwrap()).path = path.display().to_string();
    app.discover_layout();
    finish_job(&mut app);
    assert!(app.error.is_none());
    assert!(app.layout_override.is_none());
    assert_eq!(
        serde_json::to_value(&app.options).unwrap(),
        serde_json::to_value(&app.analysis.as_ref().unwrap().options).unwrap()
    );
}

#[test]
fn folder_reset_preserves_saved_choices_and_report_when_reanalysis_fails() {
    let folder = tempfile::tempdir().unwrap();
    for name in ["app.elf", "other.elf"] {
        std::fs::write(
            folder.path().join(name),
            include_bytes!("../../../fixtures/build/gcc/cortex-m.elf"),
        )
        .unwrap();
    }
    std::fs::write(
        folder.path().join("app.map"),
        include_bytes!("../../../fixtures/build/gcc/cortex-m.map"),
    )
    .unwrap();
    std::fs::write(
        folder.path().join("manual.map"),
        include_bytes!("../../../fixtures/build/gcc/cortex-m-grown.map"),
    )
    .unwrap();
    let mut app = Explorer::default();
    app.scan_build(folder.path().to_owned());
    finish_job(&mut app);
    let root = app.build.as_ref().unwrap().root.clone();
    for name in ["other.elf", "app.elf"] {
        app.open(root.join(name));
        finish_job(&mut app);
        app.apply_map(root.join("manual.map"));
        finish_map_job(&mut app);
    }
    let preferences = app.preference_value();
    let analysis = app.analysis.clone().unwrap();
    std::fs::write(root.join("app.elf"), b"incomplete rebuild").unwrap();
    app.reset_build_settings();
    finish_job(&mut app);
    assert!(app.error.is_some());
    assert!(Arc::ptr_eq(&analysis, app.analysis.as_ref().unwrap()));
    assert_eq!(app.preference_value(), preferences);
    assert_eq!(app.build_settings[&root].layouts.len(), 2);

    std::fs::write(
        root.join("app.elf"),
        include_bytes!("../../../fixtures/build/gcc/cortex-m.elf"),
    )
    .unwrap();
    app.reset_build_settings();
    finish_job(&mut app);
    assert!(app.error.is_none());
    assert!(app.build_settings[&root].layouts.is_empty());
    assert!(app.map_in_use(&root.join("app.map")));
}

#[test]
fn folder_workflow_selects_firmware_and_loads_stack_automatically() {
    let mut app = Explorer::default();
    app.scan_build(PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/build/gcc"
    )));
    finish_job(&mut app);
    assert!(app.error.is_none());
    assert!(
        app.analysis.is_none(),
        "The user chooses which image to view"
    );
    let build = app.build.clone().unwrap();
    assert_eq!(
        build
            .artifacts
            .iter()
            .filter(|a| a.kind == snout_core::build::ArtifactKind::Firmware)
            .count(),
        3
    );
    app.open(build.root.join("cortex-m.elf"));
    finish_job(&mut app);
    assert!(app.analysis.is_some());
    assert!(!app.stack.as_ref().unwrap().entries.is_empty());
    assert!(app
        .stack
        .as_ref()
        .unwrap()
        .entries
        .iter()
        .all(|entry| entry.report_file.contains("cortex-m-objects.dir")));
    assert!(app
        .stack
        .as_ref()
        .unwrap()
        .warnings
        .iter()
        .any(|w| w.contains("selected automatically")));
    let previous = app.analysis.as_ref().unwrap().path.clone();
    app.open(build.root.join("missing.elf"));
    finish_job(&mut app);
    assert!(app.error.is_some());
    assert_eq!(app.analysis.as_ref().unwrap().path, previous);
    app.open(build.root.join("cortex-m-grown.elf"));
    finish_job(&mut app);
    assert!(app.error.is_none());
    assert!(app
        .analysis
        .as_ref()
        .unwrap()
        .path
        .ends_with("cortex-m-grown.elf"));
    let ctx = egui::Context::default();
    let output = ctx.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 820.0),
            )),
            ..Default::default()
        },
        |ctx| app.show(ctx),
    );
    assert!(output.viewport_output[&egui::ViewportId::ROOT]
        .commands
        .iter()
        .any(|command| matches!(command, egui::ViewportCommand::Title(title) if title.ends_with("gcc - Rusty's Snout - Firmware Explorer"))));
    assert!(output
        .shapes
        .iter()
        .any(|s| matches!(&s.shape, egui::Shape::Text(t) if t.galley.text() == "Menu")));
    app.scan_build(build.root.join("cortex-m.elf"));
    finish_job(&mut app);
    assert!(app.error.as_ref().unwrap().contains("folder"));
    assert!(
        app.analysis.is_some(),
        "A failed scan preserves the selected report"
    );
}

#[test]
fn arrow_keys_navigate_visible_rows_and_respect_text_focus() {
    let analysis = snout_core::analyze_bytes(
        include_bytes!("../../../fixtures/build/gcc/cortex-m.elf"),
        "fixture.elf",
        &Default::default(),
    )
    .unwrap();
    let mut names: Vec<_> = analysis.sections.iter().map(|s| s.name.clone()).collect();
    names.sort();
    let ctx = egui::Context::default();
    shell::configure_style(&ctx);
    let mut app = Explorer {
        analysis: Some(Arc::new(analysis)),
        view: View::Sections,
        sort_column: 0,
        descending: false,
        ..Default::default()
    };
    let frame = |app: &mut Explorer, key: Option<egui::Key>| {
        let events = key
            .into_iter()
            .flat_map(|key| {
                [true, false].map(|pressed| egui::Event::Key {
                    key,
                    physical_key: None,
                    pressed,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                })
            })
            .collect();
        ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(900.0, 600.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| app.show(ctx),
        )
    };
    frame(&mut app, None);
    for key in [egui::Key::ArrowUp, egui::Key::ArrowDown] {
        app.details = None;
        frame(&mut app, Some(key));
        assert_eq!(app.details.as_ref().unwrap().0, names[0]);
    }
    frame(&mut app, Some(egui::Key::ArrowDown));
    assert_eq!(app.details.as_ref().unwrap().0, names[1]);
    frame(&mut app, Some(egui::Key::ArrowUp));
    frame(&mut app, Some(egui::Key::ArrowUp));
    assert_eq!(app.details.as_ref().unwrap().0, names[0]);
    for _ in 0..names.len() + 1 {
        frame(&mut app, Some(egui::Key::ArrowDown));
    }
    assert_eq!(app.details.as_ref().unwrap().0, *names.last().unwrap());
    let output = frame(&mut app, None);
    assert!(
        output.shapes.iter().any(|s| match &s.shape {
            egui::Shape::Text(t) =>
                t.galley.text() == names.last().unwrap()
                    && s.clip_rect.contains(t.pos + t.galley.size() * 0.5),
            _ => false,
        }),
        "Keyboard selection must scroll into view"
    );
    app.search = ".text".into();
    frame(&mut app, Some(egui::Key::ArrowDown));
    assert_eq!(app.details.as_ref().unwrap().0, ".text");
    app.search = "no matching rows".into();
    app.details = None;
    frame(&mut app, Some(egui::Key::ArrowUp));
    assert!(app.details.is_none());
    app.search.clear();
    // A focused text editor owns arrow keys; table navigation must not consume them.
    let id = egui::Id::new("focused_editor");
    egui::TextEdit::store_state(&ctx, id, egui::text_edit::TextEditState::default());
    ctx.memory_mut(|m| m.request_focus(id));
    let analysis = app.analysis.clone().unwrap();
    let _ = ctx.run(
        egui::RawInput {
            events: vec![egui::Event::Key {
                key: egui::Key::ArrowDown,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.add(egui::TextEdit::singleline(&mut String::new()).id(id));
                app.sections(ui, &analysis);
            });
        },
    );
    assert!(app.details.is_none());
}

#[test]
fn all_data_views_render_headlessly() {
    let analysis = snout_core::analyze_bytes(
        include_bytes!("../../../fixtures/build/gcc/cortex-m.elf"),
        "fixture.elf",
        &Default::default(),
    )
    .unwrap();
    let ctx = egui::Context::default();
    let mut app = Explorer {
        analysis: Some(Arc::new(analysis.clone())),
        comparison: Some(compare(&analysis, &analysis)),
        ..Default::default()
    };
    app.stack = Some(
        analyze_stack(
            &analysis,
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../fixtures/build/gcc/CMakeFiles/cortex-m-objects.dir/src/main.c.su"
            ),
        )
        .unwrap(),
    );
    for view in View::ALL {
        app.view = view;
        for search in ["", "no such symbol"] {
            app.search = search.into();
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1100.0, 700.0),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| match view {
                        View::Overview => app.overview(ui, &analysis),
                        View::Files => app.files(ui, &analysis),
                        View::Symbols => app.symbols(ui, &analysis),
                        View::Sections => app.sections(ui, &analysis),
                        View::MemoryMap => app.memory_map(ui, &analysis),
                        View::Dependencies => app.dependency_view(ui, &analysis),
                        View::Stack => app.stack_view(ui),
                        View::Memory => app.memory_view(ui, &Arc::new(analysis.clone())),
                        View::BuildFiles => app.build_files(ui),
                        View::Baselines => app.baselines_view(ui),
                    });
                },
            );
            assert!(!output.shapes.is_empty());
        }
    }
}

#[test]
fn compact_shell_renders_all_views_with_optional_panes() {
    let analysis = snout_core::analyze_bytes(
        include_bytes!("../../../fixtures/build/gcc/cortex-m.elf"),
        "fixture.elf",
        &Default::default(),
    )
    .unwrap();
    let ctx = egui::Context::default();
    shell::configure_style(&ctx);
    let mut app = Explorer {
        analysis: Some(Arc::new(analysis)),
        ..Default::default()
    };
    for size in [egui::vec2(900.0, 600.0), egui::vec2(1280.0, 820.0)] {
        for view in View::ALL {
            app.change_view(view);
            for expanded in [false, true] {
                app.tree = expanded;
                app.show_notes = expanded;
                app.details = Some((
                    "Selection".into(),
                    "Address: 0x08000000\nSource: main.c:1".into(),
                ));
                let output = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                        ..Default::default()
                    },
                    |ctx| app.show(ctx),
                );
                assert!(!output.shapes.is_empty());
            }
        }
    }
}

#[test]
fn tabs_row_selection_and_escape_work_in_the_shell() {
    let ctx = egui::Context::default();
    shell::configure_style(&ctx);
    let analysis = snout_core::analyze_bytes(
        include_bytes!("../../../fixtures/build/gcc/cortex-m.elf"),
        "fixture.elf",
        &Default::default(),
    )
    .unwrap();
    let mut app = Explorer {
        analysis: Some(Arc::new(analysis)),
        ..Default::default()
    };
    fn frame(
        ctx: &egui::Context,
        app: &mut Explorer,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1280.0, 820.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| app.show(ctx),
        )
    }
    fn text_position(output: &egui::FullOutput, text: &str) -> egui::Pos2 {
        output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(t) if t.galley.text() == text => {
                    Some(t.pos + t.galley.size() * 0.5)
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("Visible text not found: {text}"))
    }
    fn click(ctx: &egui::Context, app: &mut Explorer, pos: egui::Pos2) {
        for pressed in [true, false] {
            frame(
                ctx,
                app,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
    }
    frame(&ctx, &mut app, vec![]);
    let output = frame(&ctx, &mut app, vec![]);
    click(&ctx, &mut app, text_position(&output, "Sections"));
    assert!(app.view == View::Sections);
    frame(&ctx, &mut app, vec![]);
    let output = frame(&ctx, &mut app, vec![]);
    let text_pos = text_position(&output, ".text");
    for pos in [text_pos, egui::pos2(200.0, text_pos.y)] {
        frame(&ctx, &mut app, vec![egui::Event::PointerMoved(pos)]);
        let hovered = frame(&ctx, &mut app, vec![]);
        let highlighted_cells = hovered
            .shapes
            .iter()
            .filter(|shape| match &shape.shape {
                egui::Shape::Rect(rect) => {
                    rect.fill == ctx.style().visuals.widgets.hovered.bg_fill
                        && rect.rect.top() <= text_pos.y
                        && rect.rect.bottom() >= text_pos.y
                }
                _ => false,
            })
            .count();
        assert!(
            highlighted_cells >= 7,
            "Hovering text or blank space should highlight every cell in the row"
        );
    }
    click(&ctx, &mut app, text_position(&output, ".text"));
    assert!(app.details.is_some());
    assert_eq!(app.details.as_ref().unwrap().0, ".text");
    let output = frame(&ctx, &mut app, vec![]);
    let detail_pos = text_position(&output, &app.details.as_ref().unwrap().1);
    assert!(detail_pos.y > text_position(&output, ".text").y);
    assert!(detail_pos.y < text_position(&output, ".reserved").y);
    let (text_index, detail_rect) = output
        .shapes
        .iter()
        .enumerate()
        .find_map(|(index, shape)| match &shape.shape {
            egui::Shape::Text(t) if t.galley.text() == app.details.as_ref().unwrap().1 => {
                Some((index, egui::Rect::from_min_size(t.pos, t.galley.size())))
            }
            _ => None,
        })
        .unwrap();
    let dividers: Vec<_> = output
        .shapes
        .iter()
        .enumerate()
        .filter_map(|(index, shape)| match &shape.shape {
            egui::Shape::LineSegment { points, .. }
                if points[0].x == points[1].x
                    && points[0].y.min(points[1].y) < detail_rect.top()
                    && points[0].y.max(points[1].y) > detail_rect.bottom()
                    && points[0].x > detail_rect.left()
                    && points[0].x < detail_rect.right() =>
            {
                Some((index, egui::pos2(points[0].x, detail_rect.center().y)))
            }
            _ => None,
        })
        .collect();
    assert!(
        !dividers.is_empty(),
        "Test detail text should span a column boundary"
    );
    for (divider_index, crossing) in dividers {
        assert!(divider_index < text_index);
        assert!(
            output.shapes[divider_index + 1..text_index]
                .iter()
                .any(|shape| match &shape.shape {
                    egui::Shape::Rect(rect) =>
                        rect.rect.contains(crossing)
                            && shape.clip_rect.contains(crossing)
                            && rect.fill.a() == 255,
                    _ => false,
                }),
            "The detail background must cover column dividers before drawing text"
        );
    }
    let detail_origin = output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(t) if t.galley.text() == app.details.as_ref().unwrap().1 => {
                Some(t.pos)
            }
            _ => None,
        })
        .unwrap();
    let start = detail_origin + egui::vec2(1.0, 6.0);
    let end = start + egui::vec2(24.0, 0.0);
    frame(
        &ctx,
        &mut app,
        vec![
            egui::Event::PointerMoved(start),
            egui::Event::PointerButton {
                pos: start,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
        ],
    );
    frame(&ctx, &mut app, vec![egui::Event::PointerMoved(end)]);
    let selected = frame(
        &ctx,
        &mut app,
        vec![egui::Event::PointerButton {
            pos: end,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        }],
    );
    assert!(
        app.details.is_some(),
        "Selecting detail text must not collapse the row"
    );
    assert!(
        selected.shapes.iter().any(|shape| match &shape.shape {
            egui::Shape::Text(t) => t.galley.rows.iter().any(|r| r
                .visuals
                .mesh
                .vertices
                .iter()
                .any(|v| v.color == views::TEXT_SELECTION)),
            _ => false,
        }),
        "Selected text needs a visible selection background"
    );
    assert_ne!(views::TEXT_SELECTION, ctx.style().visuals.selection.bg_fill);
    click(&ctx, &mut app, text_position(&output, ".text"));
    assert!(
        app.details.is_none(),
        "A second click should collapse the row"
    );
    let output = frame(&ctx, &mut app, vec![]);
    click(&ctx, &mut app, text_position(&output, ".text"));
    assert!(app.details.is_some());
    frame(
        &ctx,
        &mut app,
        vec![egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }],
    );
    assert!(app.details.is_none());
}

#[test]
fn build_folder_scan_finds_adjacent_and_nested_reports() {
    let root = std::env::temp_dir().join(format!(
        "snout-stack-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(root.join("objects/nested")).unwrap();
    let root = root.canonicalize().unwrap();
    std::fs::write(
        root.join("main.su"),
        "main.c:1:1:Reset_Handler\t32\tstatic\n",
    )
    .unwrap();
    std::fs::write(
        root.join("objects/nested/diag.su"),
        "diag.c:1:1:diagnose\t48\tstatic\n",
    )
    .unwrap();
    let mut analysis = snout_core::analyze_bytes(
        include_bytes!("../../../fixtures/build/gcc/cortex-m.elf"),
        "fixture.elf",
        &Default::default(),
    )
    .unwrap();
    analysis.path = root.join("firmware.elf").to_string_lossy().into_owned();
    let mut app = Explorer {
        analysis: Some(Arc::new(analysis)),
        ..Default::default()
    };
    app.build = Some(Arc::new(snout_core::build::scan_folder(&root).unwrap()));
    app.select_stack_reports(vec![root.clone()]);
    let result = app
        .receiver
        .take()
        .unwrap()
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap()
        .result
        .unwrap();
    std::fs::remove_dir_all(&root).unwrap();
    let Loaded::SelectedStack(report, _) = result else {
        panic!("Expected stack report")
    };
    assert_eq!(report.entries.len(), 2);
    assert!(report
        .entries
        .iter()
        .any(|e| e.function == "diagnose" && e.local_bytes == 48));
    assert!(report
        .entries
        .iter()
        .any(|e| e.function == "Reset_Handler" && e.local_bytes == 32));
}
#[test]
fn configured_regions_render_bars_and_search_without_symbols() {
    let options = snout_core::map::parse_map_regions(include_str!(
        "../../../fixtures/build/gcc/cortex-m.map"
    ))
    .unwrap();
    let analysis = snout_core::analyze_bytes(
        include_bytes!("../../../fixtures/build/gcc/cortex-m.elf"),
        "fixture",
        &options,
    )
    .unwrap();
    let ctx = egui::Context::default();
    let mut app = Explorer {
        view: View::MemoryMap,
        ..Default::default()
    };
    for region in 0..options.regions.len() {
        app.selected_region = Some(region); // Legacy selection must not open a symbol browser.
        for search in ["", "no-such-region-symbol"] {
            app.search = search.into();
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1100.0, 700.0),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| app.memory_map(ui, &analysis));
                },
            );
            assert!(!output.shapes.is_empty());
            if search.is_empty() {
                assert_eq!(app.visible_rows, options.regions.len());
                let texts: Vec<_> = output
                    .shapes
                    .iter()
                    .filter_map(|shape| match &shape.shape {
                        egui::Shape::Text(t) => Some(t.galley.text()),
                        _ => None,
                    })
                    .collect();
                for region in &options.regions {
                    assert!(texts.iter().any(|text| text.contains(&region.name)));
                }
                assert!(!texts
                    .iter()
                    .any(|text| text.contains("Symbols in") || *text == "Symbol"));
                assert_eq!(
                    output
                        .shapes
                        .iter()
                        .filter(|shape| matches!(&shape.shape,
                            egui::Shape::Rect(r) if r.fill == egui::Color32::from_rgb(45, 60, 79)
                        ))
                        .count(),
                    options.regions.len()
                );
            } else {
                assert_eq!(app.visible_rows, 0);
            }
        }
    }
}

#[test]
fn header_reload_is_right_aligned_clickable_and_replaced_while_loading() {
    let mut app = Explorer::default();
    let ctx = egui::Context::default();
    shell::configure_style(&ctx);
    let frame = |app: &mut Explorer, width, events| {
        ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(width, 800.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| app.show(ctx),
        )
    };
    // Locate the drawn reload arc; the icon-only control has no visible text.
    let reload_center = |output: &egui::FullOutput| {
        output.shapes.iter().find_map(|shape| match &shape.shape {
            egui::Shape::Path(path) if path.points.len() == 33 && !path.closed => {
                let angle = std::f32::consts::PI * 0.2;
                Some(path.points[0] - egui::vec2(angle.cos(), angle.sin()) * 8.0)
            }
            _ => None,
        })
    };
    let click = |app: &mut Explorer, pos| {
        for pressed in [true, false] {
            frame(
                app,
                1280.0,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
    };
    frame(&mut app, 1280.0, vec![]);
    let output = frame(&mut app, 1280.0, vec![]);
    click(&mut app, reload_center(&output).unwrap());
    assert!(
        app.receiver.is_none(),
        "Refresh is disabled without a build folder"
    );

    app.scan_build(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/build/gcc"));
    finish_job(&mut app);
    app.open(app.build.as_ref().unwrap().root.join("cortex-m.elf"));
    finish_job(&mut app);
    for width in [900.0, 1280.0] {
        frame(&mut app, width, vec![]);
        let output = frame(&mut app, width, vec![]);
        let center = reload_center(&output).expect("Idle header must have a reload icon");
        assert!(
            (center.x - (width - 32.0)).abs() < 1.0,
            "Reload must stay at the top right at {width}: {center:?}"
        );
        assert!(center.y < 140.0);
    }
    let output = frame(&mut app, 1280.0, vec![]);
    click(&mut app, reload_center(&output).unwrap());
    assert!(
        app.receiver.is_some(),
        "Clicking reload must start a refresh"
    );
    let output = frame(&mut app, 1280.0, vec![]);
    assert!(
        reload_center(&output).is_none(),
        "Loading must replace the reload button"
    );
    assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,
        egui::Shape::Text(text) if text.galley.text() == "Analyzing..."
    )));
    finish_job(&mut app);
    let output = frame(&mut app, 1280.0, vec![]);
    assert!(reload_center(&output).is_some());
}

#[test]
fn changed_firmware_indicator_survives_failed_reload_and_clears_on_success() {
    let directory = tempfile::tempdir().unwrap();
    let elf = directory.path().join("zephyr.elf");
    let fixture =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/build/gcc/cortex-m.elf");
    std::fs::copy(&fixture, &elf).unwrap();
    let mut app = Explorer::default();
    app.scan_build(directory.path().to_owned());
    finish_job(&mut app);
    app.open(elf.clone());
    finish_job(&mut app);
    assert!(!app.firmware_watch.changed());
    // Change after analysis, before starting the monitor, to exercise its
    // immediate first check without waiting for the periodic interval.
    std::fs::write(&elf, b"incomplete linker output").unwrap();
    let ctx = egui::Context::default();
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while !app.firmware_watch.changed() {
        app.firmware_watch.poll(&ctx);
        assert!(
            std::time::Instant::now() < deadline,
            "Worker must flag the changed ELF"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    let report = app.analysis.clone().unwrap();
    app.refresh();
    finish_job(&mut app);
    assert!(app.error.is_some());
    assert!(Arc::ptr_eq(&report, app.analysis.as_ref().unwrap()));
    assert!(app.firmware_watch.changed());
    let output = ctx.run(egui::RawInput::default(), |ctx| app.show(ctx));
    assert!(
        output.shapes.iter().any(|shape| matches!(&shape.shape,
            egui::Shape::Circle(circle) if circle.radius == 2.5
                && circle.fill == egui::Color32::from_rgb(245, 184, 75)
        )),
        "Changed firmware must show the amber reload badge"
    );
    std::fs::copy(fixture, &elf).unwrap();
    app.refresh();
    finish_job(&mut app);
    assert!(app.error.is_none());
    assert!(!app.firmware_watch.changed());
    // Switching away from a report must stop tracking that ELF.
    let empty_folder = tempfile::tempdir().unwrap();
    app.scan_build(empty_folder.path().to_owned());
    finish_job(&mut app);
    assert!(app.analysis.is_none());
    assert!(!app.firmware_watch.changed());
}

#[test]
fn refresh_restores_selection_and_layout_and_preserves_report_on_failure() {
    let mut app = Explorer::default();
    app.scan_build(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/build/gcc"));
    finish_job(&mut app);
    let path = app.build.as_ref().unwrap().root.join("cortex-m.elf");
    app.open(path.clone());
    finish_job(&mut app);
    app.configure(Some(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/build/gcc/cortex-m.map"),
    ));
    finish_job(&mut app);
    app.view = View::MemoryMap;
    let options = app.options.clone();
    let source = app.layout_source.clone();
    app.refresh();
    finish_job(&mut app);
    assert!(app.error.is_none());
    assert_eq!(app.options, options);
    assert_eq!(app.layout_source, source);
    assert!(app.view == View::MemoryMap);
    assert_eq!(
        std::path::Path::new(&app.analysis.as_ref().unwrap().path),
        path
    );
    Arc::make_mut(app.analysis.as_mut().unwrap()).path =
        path.with_extension("missing").display().to_string();
    let previous = app.analysis.clone().unwrap();
    app.refresh();
    finish_job(&mut app);
    assert!(app.error.is_some());
    assert!(Arc::ptr_eq(app.analysis.as_ref().unwrap(), &previous));
    assert_eq!(app.options, options);
}

#[test]
fn startup_folder_restores_saved_elf_unless_another_is_explicitly_selected() {
    let folder = tempfile::tempdir().unwrap();
    // Match the canonical paths produced by startup parsing and build discovery.
    let root = folder.path().canonicalize().unwrap();
    let first = root.join("first.elf");
    let second = root.join("second.elf");
    for path in [&first, &second] {
        std::fs::write(
            path,
            include_bytes!("../../../fixtures/build/gcc/cortex-m.elf"),
        )
        .unwrap();
    }
    let value = serde_json::json!({
        "version": 1, "folder": root,
        "build_settings": {root.to_string_lossy(): {"firmware": first, "layouts": {}}},
    });
    for explicit in [None, Some(second.clone())] {
        let mut app = Explorer::default();
        app.apply_preferences_with_workspace(&value, false);
        assert!(app.receiver.is_none());
        app.open_startup(&startup::Startup {
            folder: Some(root.clone()),
            elf: explicit.clone(),
            ..Default::default()
        });
        finish_job(&mut app);
        finish_job(&mut app);
        assert!(app.error.is_none());
        assert_eq!(
            PathBuf::from(&app.analysis.as_ref().unwrap().path),
            explicit.unwrap_or(first.clone())
        );
    }
}

#[test]
fn explicit_startup_selection_restores_its_saved_layout_and_reloads_the_source() {
    let folder = tempfile::tempdir().unwrap();
    // Match the canonical paths produced by startup parsing and build discovery.
    let root = folder.path().canonicalize().unwrap();
    let first = root.join("first.elf");
    let second = root.join("second.elf");
    let map = root.join("manual.map");
    for path in [&first, &second] {
        std::fs::write(
            path,
            include_bytes!("../../../fixtures/build/gcc/cortex-m.elf"),
        )
        .unwrap();
    }
    std::fs::write(
        &map,
        include_bytes!("../../../fixtures/build/gcc/cortex-m.map"),
    )
    .unwrap();
    let mut original = Explorer::default();
    original.scan_build(root.clone());
    finish_job(&mut original);
    original.open(second.clone());
    finish_job(&mut original);
    original.apply_map(map.clone());
    finish_map_job(&mut original);
    original.open(first);
    finish_job(&mut original);

    let updated_map = include_str!("../../../fixtures/build/gcc/cortex-m.map").replacen(
        "0x00040000",
        "0x00080000",
        1,
    );
    std::fs::write(&map, &updated_map).unwrap();
    let expected = snout_core::build::parse_map_regions(&updated_map).unwrap();
    let mut restored = Explorer::default();
    restored.apply_preferences_with_workspace(&original.preference_value(), false);
    restored.open_startup(&startup::Startup {
        folder: Some(root.clone()),
        elf: Some(second.clone()),
        ..Default::default()
    });
    finish_job(&mut restored);
    finish_job(&mut restored);
    assert!(restored.error.is_none(), "{:?}", restored.error);
    assert_eq!(
        PathBuf::from(&restored.analysis.as_ref().unwrap().path),
        second
    );
    assert!(restored.map_in_use(&map));
    assert_eq!(restored.options, expected);
    assert_eq!(restored.saved_layout(&second).unwrap().options, expected);
}

#[test]
fn startup_without_a_path_restores_last_folder_even_without_saved_firmware() {
    let folder = tempfile::tempdir().unwrap();
    // Match the canonical paths produced by startup parsing and build discovery.
    let root = folder.path().canonicalize().unwrap();
    let mut app = Explorer::default();
    app.apply_preferences(&serde_json::json!({"version": 1, "folder": root}));
    app.open_startup(&startup::Startup::default());
    finish_job(&mut app);
    assert_eq!(app.build.as_ref().unwrap().root, root);
    assert!(app.analysis.is_none());
    assert!(app.receiver.is_none());
    assert!(app.error.is_none());
}

#[test]
fn reopening_same_folder_restores_firmware_and_layout() {
    let mut app = Explorer::default();
    app.scan_build(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/build/gcc"));
    finish_job(&mut app);
    let folder = app.build.as_ref().unwrap().root.clone();
    let elf = folder.join("cortex-m.elf");
    app.open(elf.clone());
    finish_job(&mut app);
    app.configure(Some(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/build/gcc/cortex-m.map"),
    ));
    finish_job(&mut app);
    let layout = app.options.clone();
    // A folder with no selected firmware must not reuse another folder's ELF.
    let other = tempfile::tempdir().unwrap();
    app.scan_build(other.path().to_owned());
    finish_job(&mut app);
    assert!(app.analysis.is_none());
    assert!(app.receiver.is_none());
    app.scan_build(folder.clone());
    finish_job(&mut app);
    finish_job(&mut app);
    assert_eq!(PathBuf::from(&app.analysis.as_ref().unwrap().path), elf);
    assert_eq!(app.options, layout);
    // Reopening the currently selected folder also keeps the selected firmware.
    app.scan_build(folder);
    finish_job(&mut app);
    finish_job(&mut app);
    assert_eq!(PathBuf::from(&app.analysis.as_ref().unwrap().path), elf);
    assert!(app.error.is_none());
}

#[test]
fn reopening_folder_with_missing_saved_elf_leaves_firmware_unselected() {
    let folder = tempfile::tempdir().unwrap();
    // Match the canonical paths produced by startup parsing and build discovery.
    let root = folder.path().canonicalize().unwrap();
    let elf = root.join("firmware.elf");
    std::fs::write(
        &elf,
        include_bytes!("../../../fixtures/build/gcc/cortex-m.elf"),
    )
    .unwrap();
    let value = serde_json::json!({"version": 1, "folder": root,
        "build_settings": {root.to_string_lossy(): {"firmware": elf, "layouts": {}}},
    });
    let mut app = Explorer::default();
    app.apply_preferences_with_workspace(&value, false);
    std::fs::remove_file(elf).unwrap();
    app.open_startup(&startup::Startup {
        folder: Some(root.clone()),
        ..Default::default()
    });
    finish_job(&mut app);
    assert!(app.analysis.is_none());
    assert!(app.receiver.is_none());
    assert!(app.error.is_none());
    assert_eq!(app.build.as_ref().unwrap().root, root);
}

#[test]
fn preferences_restore_selected_firmware_layout_and_view() {
    let mut original = Explorer::default();
    original.scan_build(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/build/gcc"));
    finish_job(&mut original);
    original.open(original.build.as_ref().unwrap().root.join("cortex-m.elf"));
    finish_job(&mut original);
    original.configure(Some(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/build/gcc/cortex-m.map"),
    ));
    finish_job(&mut original);
    original.view = View::Symbols;
    original.tree = true;
    original.overview_metric = overview::Metric::Ram;
    let value = original.preference_value();
    let mut restored = Explorer::default();
    restored.apply_preferences(&value);
    finish_job(&mut restored);
    finish_job(&mut restored);
    assert!(restored.error.is_none());
    assert!(restored.view == View::Symbols);
    assert!(!restored.tree);
    assert!(restored.overview_metric == overview::Metric::Flash);
    assert_eq!(restored.options, original.options);
    assert_eq!(restored.layout_source, original.layout_source);
    assert_eq!(
        restored.analysis.as_ref().unwrap().path,
        original.analysis.as_ref().unwrap().path
    );
}

#[test]
fn notes_and_cached_rankings_follow_the_report_and_view() {
    let mut app = Explorer::default();
    app.scan_build(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/build/gcc"));
    finish_job(&mut app);
    app.open(app.build.as_ref().unwrap().root.join("cortex-m.elf"));
    finish_job(&mut app);
    let a = app.analysis.clone().unwrap();
    let base = a.warnings.len();
    let stack = app.stack.as_ref().unwrap().warnings.len();
    assert_eq!(app.visible_notes().len(), base + stack);
    app.view = View::Files;
    assert_eq!(app.visible_notes().len(), base);
    app.view = View::Stack;
    assert_eq!(app.visible_notes().len(), base + stack);
    app.ensure_region_cache(&a);
    for (slot, metric) in [overview::Metric::Flash, overview::Metric::Ram]
        .into_iter()
        .enumerate()
    {
        assert!(app.top_symbols[slot]
            .windows(2)
            .all(|w| metric.value(a.symbols[w[0]].usage) >= metric.value(a.symbols[w[1]].usage)));
    }
    app.configure(Some(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/build/gcc/cortex-m.map"),
    ));
    finish_job(&mut app);
    let a = app.analysis.clone().unwrap();
    app.ensure_region_cache(&a);
    assert_eq!(app.region_cache.len(), a.options.regions.len());
    assert!(!app.region_cache.is_empty());
    for (region, usage) in a.options.regions.iter().zip(&app.region_cache) {
        assert_eq!(
            usage.used,
            snout_core::regions::region_usage(&a, region).used
        );
    }
}

#[test]
fn startup_elf_is_selected_after_scan_with_map_and_stack_reports() {
    let folder = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/build/gcc");
    let startup = startup::parse([
        folder.into_os_string(),
        "--elf".into(),
        "cortex-m.elf".into(),
    ])
    .unwrap()
    .unwrap();
    let mut app = Explorer::default();
    app.open_startup(&startup);
    finish_job(&mut app);
    assert!(app.analysis.is_none());
    assert!(app.receiver.is_some());
    finish_job(&mut app);
    assert!(app.error.is_none());
    assert_eq!(
        PathBuf::from(&app.analysis.as_ref().unwrap().path),
        startup.elf.unwrap()
    );
    assert_eq!(app.options.regions.len(), 2);
    assert!(app.stack.is_some());
}

#[test]
fn update_preferences_round_trip_without_an_open_workspace() {
    let mut app = Explorer::default();
    app.updates.check_on_startup = false;
    app.updates.skipped_version = Some("v0.2.0".into());
    let mut restored = Explorer::default();
    restored.apply_preferences(&app.preference_value());
    assert!(!restored.updates.check_on_startup);
    assert_eq!(restored.updates.skipped_version.as_deref(), Some("v0.2.0"));
    restored.apply_preferences(&serde_json::json!({"version": 1}));
    assert!(restored.updates.check_on_startup);
    assert!(restored.updates.skipped_version.is_none());
}

#[test]
fn local_frame_bars_compare_visible_memory_without_covering_the_numbers() {
    let analysis = snout_core::analyze_bytes(
        include_bytes!("../../../fixtures/build/gcc/cortex-m.elf"),
        "fixture.elf",
        &Default::default(),
    )
    .unwrap();
    let mut report = analyze_stack(
        &analysis,
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../fixtures/build/gcc/CMakeFiles/cortex-m-objects.dir/src/main.c.su"
        ),
    )
    .unwrap();
    report.entries.truncate(3);
    for (entry, (name, bytes)) in report.entries.iter_mut().zip([
        ("largest_frame", 64),
        ("half_frame", 32),
        ("zero_frame", 0),
    ]) {
        entry.function = name.into();
        entry.local_bytes = bytes;
    }
    let mut app = Explorer {
        view: View::Stack,
        analysis: Some(Arc::new(analysis)),
        stack: Some(report),
        ..Default::default()
    };
    let ctx = egui::Context::default();
    shell::configure_style(&ctx);
    let frame = |app: &mut Explorer| {
        ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1000.0, 600.0),
                )),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| app.stack_view(ui));
            },
        )
    };
    let bars = |output: &egui::FullOutput| {
        output
            .shapes
            .iter()
            .enumerate()
            .filter_map(|(i, shape)| match &shape.shape {
                egui::Shape::Rect(rect) if rect.fill == views::MEMORY_BAR => Some((i, rect.rect)),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    let output = frame(&mut app);
    let rects = bars(&output);
    assert_eq!(rects.len(), 2);
    let mut widths = vec![];
    for value in ["64 B", "32 B"] {
        let (text_index, text) = output
            .shapes
            .iter()
            .enumerate()
            .find_map(|(i, shape)| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == value => Some((i, text)),
                _ => None,
            })
            .unwrap();
        let (bar_index, bar) = rects
            .iter()
            .find(|(_, bar)| bar.y_range().contains(text.pos.y))
            .unwrap();
        assert!(
            bar_index < &text_index,
            "Paint the gray bar behind the byte count"
        );
        assert_eq!(bar.height(), 23.0);
        widths.push(bar.width());
    }
    assert!((widths[1] / widths[0] - 0.5).abs() < 0.001);
    app.search = "half_frame".into();
    let filtered = bars(&frame(&mut app));
    assert_eq!(filtered.len(), 1);
    assert!((filtered[0].1.width() - widths[0]).abs() < 0.1);
    app.search = "zero_frame".into();
    assert!(bars(&frame(&mut app)).is_empty());
}

#[test]
fn memory_bars_scale_size_flash_and_ram_independently_in_each_table() {
    let mut analysis = snout_core::analyze_bytes(
        include_bytes!("../../../fixtures/build/gcc/cortex-m.elf"),
        "fixture.elf",
        &Default::default(),
    )
    .unwrap();
    analysis.sections.truncate(2);
    analysis.symbols.truncate(2);
    analysis.files.truncate(2);
    for (i, (size, flash, ram)) in [(64, 8, 256), (32, 16, 128)].into_iter().enumerate() {
        let name = if i == 0 { "first_item" } else { "second_item" };
        let usage = snout_core::Usage { flash, ram };
        analysis.sections[i].name = name.into();
        analysis.sections[i].size = size;
        analysis.sections[i].usage = usage;
        analysis.symbols[i].demangled_name = name.into();
        analysis.symbols[i].size = size;
        analysis.symbols[i].usage = usage;
        analysis.files[i].path = format!("{name}.c");
        analysis.files[i].usage = usage;
    }
    let bar_rects = |output: &egui::FullOutput| {
        output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Rect(rect) if rect.fill == views::MEMORY_BAR => Some(rect.rect),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    let bar_for = |output: &egui::FullOutput, value: &str| {
        let (pos, clip) = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == value => {
                    Some((text.pos, shape.clip_rect))
                }
                _ => None,
            })
            .unwrap();
        *bar_rects(output)
            .iter()
            .find(|bar| clip.contains_rect(**bar) && bar.y_range().contains(pos.y))
            .unwrap()
    };
    for view in [View::Sections, View::Symbols, View::Files] {
        let mut app = Explorer {
            view,
            ..Default::default()
        };
        let ctx = egui::Context::default();
        shell::configure_style(&ctx);
        let frame = |app: &mut Explorer| {
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1200.0, 600.0),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| match view {
                        View::Sections => app.sections(ui, &analysis),
                        View::Symbols => app.symbols(ui, &analysis),
                        View::Files => app.files(ui, &analysis),
                        _ => unreachable!(),
                    });
                },
            )
        };
        let output = frame(&mut app);
        let columns = if view == View::Files { 2 } else { 3 };
        assert_eq!(bar_rects(&output).len(), columns * 2);
        let mut comparisons = vec![("16 B", "8 B"), ("256 B", "128 B")];
        if view != View::Files {
            comparisons.push(("64 B", "32 B"));
        }
        for (largest, half) in comparisons {
            let largest = bar_for(&output, largest);
            let half = bar_for(&output, half);
            assert!((largest.left() - half.left()).abs() < 0.01);
            assert!((half.width() / largest.width() - 0.5).abs() < 0.001);
        }
        app.search = "second_item".into();
        let filtered = frame(&mut app);
        assert_eq!(bar_rects(&filtered).len(), columns);
        assert!(
            (bar_for(&filtered, "128 B").width() - bar_for(&output, "256 B").width()).abs() < 0.01
        );
    }
}

#[test]
fn stack_view_scopes_rows_to_selected_elf_and_keeps_unresolved_available() {
    let analysis = snout_core::analyze_bytes(
        include_bytes!("../../../fixtures/build/gcc/cortex-m.elf"),
        "fixture.elf",
        &Default::default(),
    )
    .unwrap();
    let mut report = analyze_stack(
        &analysis,
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../fixtures/build/gcc/CMakeFiles/cortex-m-objects.dir/src/main.c.su"
        ),
    )
    .unwrap();
    assert_eq!(report.entries.len(), 7);
    assert!(report
        .entries
        .iter()
        .all(|e| e.symbol_candidates.len() == 1));
    let mut unrelated = report.entries[0].clone();
    unrelated.function = "other_target".into();
    unrelated.symbol_candidates.clear();
    report.entries.push(unrelated);
    let mut app = Explorer {
        analysis: Some(Arc::new(analysis)),
        stack: Some(report),
        ..Default::default()
    };
    let ctx = egui::Context::default();
    for (show_unresolved, expected) in [(false, 7), (true, 8)] {
        app.stack_show_unresolved = show_unresolved;
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| app.stack_view(ui));
        });
        assert_eq!(app.visible_rows, expected);
    }
}

#[test]
fn map_choices_survive_elf_folder_switching_restart_and_folder_reset() {
    let mut app = Explorer::default();
    app.scan_build(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/build/gcc"));
    finish_job(&mut app);
    let root = app.build.as_ref().unwrap().root.clone();
    let elf = root.join("cortex-m.elf");
    let other_elf = root.join("cortex-m-grown.elf");
    let map = root.join("cortex-m.map");
    let other_map = root.join("cortex-m-grown.map");
    app.open(elf.clone());
    finish_job(&mut app);
    app.apply_map(other_map.clone());
    finish_map_job(&mut app);
    app.open(other_elf.clone());
    finish_job(&mut app);
    app.apply_map(map.clone());
    finish_map_job(&mut app);
    app.open(elf.clone());
    finish_job(&mut app);
    assert!(app.map_in_use(&other_map));

    let folder = tempfile::tempdir().unwrap();
    std::fs::write(
        folder.path().join("app.elf"),
        include_bytes!("../../../fixtures/build/gcc/cortex-m.elf"),
    )
    .unwrap();
    std::fs::write(
        folder.path().join("manual.map"),
        include_bytes!("../../../fixtures/build/gcc/cortex-m.map"),
    )
    .unwrap();
    app.scan_build(folder.path().to_owned());
    finish_job(&mut app);
    let second_root = app.build.as_ref().unwrap().root.clone();
    let second_elf = second_root.join("app.elf");
    let second_map = second_root.join("manual.map");
    app.open(second_elf.clone());
    finish_job(&mut app);
    app.apply_map(second_map.clone());
    finish_map_job(&mut app);

    let value =
        serde_json::from_slice(&serde_json::to_vec(&app.preference_value()).unwrap()).unwrap();
    let mut restored = Explorer::default();
    restored.apply_preferences(&value);
    finish_job(&mut restored);
    finish_job(&mut restored);
    assert!(restored.map_in_use(&second_map));
    // Refresh reads the chosen map again after a rebuild, rather than using stale capacities.
    std::fs::write(
        &second_map,
        include_bytes!("../../../fixtures/build/gcc/cortex-m-grown.map"),
    )
    .unwrap();
    restored.refresh();
    finish_job(&mut restored);
    let expected = snout_core::build::parse_map_regions(include_str!(
        "../../../fixtures/build/gcc/cortex-m-grown.map"
    ))
    .unwrap();
    assert_eq!(
        serde_json::to_value(&restored.options).unwrap(),
        serde_json::to_value(expected).unwrap()
    );

    restored.scan_build(root.clone());
    finish_job(&mut restored);
    finish_job(&mut restored);
    assert!(restored.map_in_use(&other_map));
    restored.open(other_elf.clone());
    finish_job(&mut restored);
    assert!(restored.map_in_use(&map));
    restored.reset_build_settings();
    finish_job(&mut restored);
    assert!(restored.map_in_use(&other_map));
    restored.open(elf.clone());
    finish_job(&mut restored);
    assert!(restored.map_in_use(&map));
    // Reset affects only the current folder.
    restored.scan_build(second_root);
    finish_job(&mut restored);
    finish_job(&mut restored);
    assert!(restored.map_in_use(&second_map));
}

#[test]
fn selected_map_supplies_dependencies_across_refresh_restart_and_failure() {
    let folder = tempfile::tempdir().unwrap();
    let root = folder.path().canonicalize().unwrap();
    let firmware = root.join("app.elf");
    std::fs::write(
        &firmware,
        include_bytes!("../../../fixtures/build/gcc/cortex-m.elf"),
    )
    .unwrap();
    let map = root.join("manual.map");
    let regions = "Memory Configuration\nName Origin Length Attributes\nFLASH 0x08000000 0x40000 xr\nRAM 0x20000000 0x10000 xrw\nLinker script and memory map\n";
    let table =
        "Cross Reference Table\nSymbol File\nReset_Handler main.o\ndiagnose diag.o\n main.o\n";
    std::fs::write(&map, format!("{regions}{table}")).unwrap();
    let mut app = Explorer::default();
    app.scan_build(root.clone());
    finish_job(&mut app);
    app.open(firmware.clone());
    finish_job(&mut app);
    app.change_view(View::BuildFiles);
    app.apply_map(map.clone());
    finish_map_job(&mut app);
    assert!(app.view == View::BuildFiles);
    assert_eq!(app.analysis.as_ref().unwrap().dependencies.edges.len(), 1);
    assert!(app.map_in_use(&map));
    let mut prefs = app.preference_value();
    // Older preferences may contain an independently selected dependency map.
    // The current map remains the single input when those preferences are read.
    prefs["build_settings"][root.to_str().unwrap()]["dependency_maps"] =
        serde_json::json!({firmware.to_str().unwrap(): root.join("obsolete.map")});
    app.apply_map(root.join("missing.map"));
    finish_map_job(&mut app);
    assert!(app.error.is_some());
    assert!(app.map_in_use(&map));
    std::fs::write(
        &map,
        format!("{regions}{table}initialized main.o\n diag.o\n"),
    )
    .unwrap();
    app.refresh();
    finish_job(&mut app);
    assert_eq!(app.analysis.as_ref().unwrap().dependencies.edges.len(), 2);
    let mut restored = Explorer::default();
    restored.apply_preferences(&prefs);
    finish_job(&mut restored);
    finish_job(&mut restored);
    assert!(restored.map_in_use(&map));
    assert_eq!(
        restored.analysis.as_ref().unwrap().dependencies.edges.len(),
        2
    );
    let last_report = restored.analysis.clone().unwrap();
    std::fs::remove_file(&map).unwrap();
    restored.refresh();
    finish_job(&mut restored);
    assert!(restored.error.is_some());
    assert!(Arc::ptr_eq(
        &last_report,
        restored.analysis.as_ref().unwrap()
    ));
    restored.configure(None);
    finish_job(&mut restored);
    assert!(restored.options.regions.is_empty());
    assert!(restored
        .analysis
        .as_ref()
        .unwrap()
        .dependencies
        .edges
        .is_empty());
    assert!(restored
        .analysis
        .as_ref()
        .unwrap()
        .dependencies
        .map_path
        .is_none());
    restored.refresh();
    finish_job(&mut restored);
    assert!(restored
        .analysis
        .as_ref()
        .unwrap()
        .dependencies
        .edges
        .is_empty());
    std::fs::write(&map, format!("{regions}{table}")).unwrap();
    restored.apply_map(map.clone());
    finish_map_job(&mut restored);
    assert_eq!(
        restored.analysis.as_ref().unwrap().dependencies.edges.len(),
        1
    );
    // A map without cross references replaces the previous map's graph too.
    std::fs::write(&map, regions).unwrap();
    restored.apply_map(map.clone());
    finish_map_job(&mut restored);
    assert!(restored
        .analysis
        .as_ref()
        .unwrap()
        .dependencies
        .edges
        .is_empty());
    assert!(restored.map_in_use(&map));
}

#[test]
fn committed_fixture_renders_six_boxes_and_sixteen_dependency_arrowheads() {
    let mut app = Explorer::default();
    app.scan_build(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/build/gcc"));
    finish_job(&mut app);
    app.open(app.build.as_ref().unwrap().root.join("cortex-m.elf"));
    finish_job(&mut app);
    assert_eq!(app.analysis.as_ref().unwrap().dependencies.edges.len(), 16);
    app.change_view(View::Dependencies);
    let ctx = egui::Context::default();
    shell::configure_style(&ctx);
    let output = dependencies::settle_graph(&mut app, |app| {
        ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1280.0, 900.0),
                )),
                ..Default::default()
            },
            |ctx| app.show(ctx),
        )
    });
    let boxes: Vec<_> = output
        .shapes
        .iter()
        .filter_map(|shape| match &shape.shape {
            egui::Shape::Rect(card) if card.fill == egui::Color32::from_rgb(40, 100, 140) => {
                assert!(shape.clip_rect.contains_rect(card.rect));
                Some(card.rect)
            }
            _ => None,
        })
        .collect();
    let mut labels = 0;
    let mut main_area = None;
    let mut config_area = None;
    for shape in &output.shapes {
        if let egui::Shape::Text(text) = &shape.shape {
            if text.galley.text().contains("\nFlash ") {
                let text_rect = egui::Rect::from_min_size(text.pos, text.galley.size());
                let card = boxes
                    .iter()
                    .find(|card| card.contains_rect(text_rect))
                    .unwrap();
                assert!(text.galley.job.sections[0].format.font_id.size > 0.0);
                if text.galley.text().starts_with("src/main.c\n") {
                    main_area = Some(card.area());
                }
                if text.galley.text().starts_with("src/config.c\n") {
                    config_area = Some(card.area());
                }
                labels += 1;
            }
        }
    }
    let arrow_color = ctx.style().visuals.text_color().gamma_multiply(0.3);
    let arrows = output.shapes.iter().filter(|shape| matches!(&shape.shape,
        egui::Shape::Path(path) if path.closed && path.points.len() == 3 && path.fill == arrow_color
    )).count();
    assert_eq!(boxes.len(), 6);
    assert_eq!(labels, 6);
    assert_eq!(arrows, 16);
    // Memory area spans zero bytes at the label-sized floor through 25× area.
    let bytes: Vec<_> = app
        .analysis
        .as_ref()
        .unwrap()
        .dependencies
        .nodes
        .iter()
        .filter_map(|node| node.usage.map(|usage| usage.flash))
        .collect();
    let largest = *bytes.iter().max().unwrap() as f32;
    let expected_ratio = (1.0 + 24.0 * 346.0 / largest) / (1.0 + 24.0 * 144.0 / largest);
    assert!((main_area.unwrap() / config_area.unwrap() - expected_ratio).abs() < 0.01);
}

#[test]
fn stack_selection_is_build_independent_persisted_per_elf_and_refreshed() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let elf = root.join("app.elf");
    let other = root.join("other.elf");
    let bytes = include_bytes!("../../../fixtures/build/gcc/cortex-m.elf");
    std::fs::write(&elf, bytes).unwrap();
    std::fs::write(&other, bytes).unwrap();
    let first = root.join("arbitrary-one.su");
    let second = root.join("arbitrary-two.su");
    std::fs::write(&first, "diag.c:22:36:diagnose\t56\tstatic\n").unwrap();
    std::fs::write(&second, "diag.c:22:36:diagnose\t80\tstatic\n").unwrap();
    let mut app = Explorer::default();
    app.scan_build(root.clone());
    finish_job(&mut app);
    app.open(elf.clone());
    finish_job(&mut app);
    assert!(app.stack.as_ref().unwrap().entries.is_empty());
    app.select_stack_reports(vec![first.clone()]);
    finish_job(&mut app);
    assert_eq!(app.stack.as_ref().unwrap().entries.len(), 1);
    assert_eq!(app.stack.as_ref().unwrap().entries[0].local_bytes, 56);
    app.refresh();
    finish_job(&mut app);
    assert_eq!(app.stack.as_ref().unwrap().entries[0].local_bytes, 56);
    let preferences = app.preference_value();
    let mut restored = Explorer::default();
    restored.apply_preferences_with_workspace(&preferences, false);
    restored.build = app.build.clone();
    assert_eq!(restored.saved_stack_reports(&elf), Some(vec![first]));
    app.open(other);
    finish_job(&mut app);
    assert!(app.stack.as_ref().unwrap().entries.is_empty());
    app.open(elf);
    finish_job(&mut app);
    assert_eq!(app.stack.as_ref().unwrap().entries[0].local_bytes, 56);
    app.select_stack_reports(vec![]);
    finish_job(&mut app);
    app.refresh();
    finish_job(&mut app);
    assert!(app.stack.as_ref().unwrap().entries.is_empty());
}

#[test]
fn selected_report_folder_remains_recursive_and_discovers_new_reports_on_refresh() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let elf = root.join("app.elf");
    std::fs::write(
        &elf,
        include_bytes!("../../../fixtures/build/gcc/cortex-m.elf"),
    )
    .unwrap();
    let reports = root.join("reports");
    std::fs::create_dir_all(reports.join("nested")).unwrap();
    std::fs::write(
        reports.join("diag.su"),
        "diag.c:22:36:diagnose\t56\tstatic\n",
    )
    .unwrap();
    let mut app = Explorer::default();
    app.scan_build(root.clone());
    finish_job(&mut app);
    app.open(elf.clone());
    finish_job(&mut app);
    // The folder itself is saved, rather than a snapshot of its files.
    app.select_stack_reports(vec![reports.clone()]);
    finish_job(&mut app);
    assert_eq!(app.saved_stack_reports(&elf), Some(vec![reports.clone()]));
    assert_eq!(app.stack.as_ref().unwrap().entries.len(), 1);
    std::fs::write(
        reports.join("nested/new.su"),
        "sensor.c:9:1:sensor_init\t24\tstatic\n",
    )
    .unwrap();
    std::fs::write(
        root.join("unrelated.su"),
        "diag.c:22:36:diagnose\t80\tstatic\n",
    )
    .unwrap();
    app.refresh();
    finish_job(&mut app);
    assert_eq!(app.stack.as_ref().unwrap().entries.len(), 2);
    // Overlapping folder/file selections load each report only once.
    app.select_stack_reports(vec![reports.clone(), reports.join("diag.su")]);
    finish_job(&mut app);
    assert_eq!(app.stack.as_ref().unwrap().entries.len(), 2);
    let preferences = app.preference_value();
    let mut restored = Explorer::default();
    restored.apply_preferences_with_workspace(&preferences, false);
    restored.build = app.build.clone();
    restored.open(elf);
    finish_job(&mut restored);
    assert_eq!(restored.stack.as_ref().unwrap().entries.len(), 2);
}

#[test]
fn sidebar_scrolls_past_long_supporting_lists_to_the_next_firmware() {
    use snout_core::build::{Artifact, ArtifactKind, BuildFolder};
    let root = std::env::temp_dir().join("snout-continuous-sidebar/build");
    let elf = root.join("app.elf");
    let analysis = snout_core::analyze_bytes(
        include_bytes!("../../../fixtures/build/gcc/cortex-m.elf"),
        &elf.to_string_lossy(),
        &Default::default(),
    )
    .unwrap();
    let mut artifacts = vec![Artifact {
        path: elf,
        kind: ArtifactKind::Firmware,
    }];
    for (extension, kind) in [("map", ArtifactKind::Map), ("su", ArtifactKind::StackUsage)] {
        artifacts.extend((0..2000).map(|i| Artifact {
            path: root.join(format!("file_{i:04}.{extension}")),
            kind,
        }));
    }
    artifacts.push(Artifact {
        path: root.join("second.elf"),
        kind: ArtifactKind::Firmware,
    });
    let mut app = Explorer {
        build: Some(Arc::new(BuildFolder {
            root,
            artifacts,
            warnings: vec![],
        })),
        analysis: Some(Arc::new(analysis)),
        ..Default::default()
    };
    let ctx = egui::Context::default();
    ctx.style_mut(|style| style.animation_time = 0.0);
    let mut frame = |scroll| {
        ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1000.0, 500.0),
                )),
                events: if scroll {
                    vec![
                        egui::Event::PointerMoved(egui::pos2(100.0, 250.0)),
                        egui::Event::MouseWheel {
                            unit: egui::MouseWheelUnit::Point,
                            delta: egui::vec2(0.0, -1_000_000.0),
                            modifiers: egui::Modifiers::NONE,
                        },
                    ]
                } else {
                    vec![]
                },
                ..Default::default()
            },
            |ctx| app.build_browser(ctx),
        )
    };
    let labels = |output: &egui::FullOutput| {
        output
            .shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Text(t) => Some(t.galley.text().to_owned()),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    frame(false);
    let first = labels(&frame(false));
    assert!(first.iter().any(|s| s == "file_0000.map"));
    assert!(
        !first.iter().any(|s| s.starts_with("Stack usage files")),
        "the map list must occupy its full height rather than a nested viewport"
    );
    assert!(
        first.len() < 100,
        "offscreen supporting rows should be skipped"
    );
    let mut last = Vec::new();
    for _ in 0..20 {
        last = labels(&frame(true));
    }
    assert!(
        last.iter().any(|s| s == "Stack usage files (2000)"),
        "collapsed stack group must be reachable: {last:?}"
    );
    assert!(
        last.iter().any(|s| s == "second.elf"),
        "scrolling over supporting files must reach the next firmware: {last:?}"
    );
    assert!(last.len() < 100);
}

#[test]
fn sidebar_large_firmware_list_only_builds_visible_rows() {
    use snout_core::build::{Artifact, ArtifactKind, BuildFolder};
    let root = std::env::temp_dir().join("snout-firmware-list/build");
    let mut app = Explorer {
        build: Some(Arc::new(BuildFolder {
            root: root.clone(),
            artifacts: (0..2000)
                .map(|i| Artifact {
                    path: root.join(format!("firmware_{i:04}.elf")),
                    kind: ArtifactKind::Firmware,
                })
                .collect(),
            warnings: vec![],
        })),
        ..Default::default()
    };
    let ctx = egui::Context::default();
    ctx.style_mut(|style| style.animation_time = 0.0);
    let frame = |app: &mut Explorer, scroll| {
        ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1000.0, 500.0),
                )),
                events: if scroll {
                    vec![
                        egui::Event::PointerMoved(egui::pos2(100.0, 250.0)),
                        egui::Event::MouseWheel {
                            unit: egui::MouseWheelUnit::Point,
                            delta: egui::vec2(0.0, -100_000.0),
                            modifiers: egui::Modifiers::NONE,
                        },
                    ]
                } else {
                    vec![]
                },
                ..Default::default()
            },
            |ctx| app.build_browser(ctx),
        )
    };
    frame(&mut app, false);
    // Each constructed collapsing header stores state, even if its paint is clipped.
    assert!(
        ctx.memory(|memory| memory.data.len()) < 200,
        "offscreen firmware widgets should not be constructed"
    );
    // A background job must not disable virtualization for every firmware row.
    let (_sender, receiver) = mpsc::channel();
    app.receiver = Some(receiver);
    frame(&mut app, false);
    assert!(
        ctx.memory(|memory| memory.data.len()) < 200,
        "offscreen collapsed firmware widgets should not be constructed while loading"
    );
    // Recover the first header's persistent ID through the same scoped UI.
    // An expanded row must still be found after it scrolls out of view.
    let mut first_header = None;
    let _ = ctx.run(Default::default(), |ctx| {
        egui::SidePanel::left("build_artifacts").show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.push_id(root.join("firmware_0000.elf"), |ui| {
                    first_header = Some(ui.make_persistent_id("firmware_files"));
                });
            });
        });
    });
    let first_header = first_header.unwrap();
    let mut state = egui::collapsing_header::CollapsingState::load(&ctx, first_header)
        .expect("the first firmware header should have stored its state");
    state.set_open(true);
    state.store(&ctx);
    frame(&mut app, false);
    let mut output = frame(&mut app, true);
    for _ in 0..10 {
        output = frame(&mut app, true);
    }
    assert!(
        output.shapes.iter().any(|shape| matches!(
            &shape.shape,
            egui::Shape::Text(t) if t.galley.text() == "firmware_1999.elf"
        )),
        "the last firmware must remain reachable by scrolling"
    );
    assert!(
        egui::collapsing_header::CollapsingState::load(&ctx, first_header)
            .unwrap()
            .is_open()
    );
    app.receiver = None;
    frame(&mut app, false);
    assert!(
        !egui::collapsing_header::CollapsingState::load(&ctx, first_header)
            .unwrap()
            .is_open(),
        "offscreen inactive firmware must close when the background job finishes"
    );
}

#[test]
fn sidebar_expands_firmware_loaded_outside_the_sidebar() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let elf = root.join("app.elf");
    std::fs::write(
        &elf,
        include_bytes!("../../../fixtures/build/gcc/cortex-m.elf"),
    )
    .unwrap();
    let other_elf = root.join("second.elf");
    std::fs::copy(&elf, &other_elf).unwrap();
    let mut app = Explorer::default();
    app.scan_build(root);
    finish_job(&mut app);
    let ctx = egui::Context::default();
    ctx.style_mut(|style| style.animation_time = 0.0);
    let frame = |app: &mut Explorer, events| {
        ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1000.0, 800.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| app.build_browser(ctx),
        )
    };
    // The folder is drawn before startup restoration or an external open completes.
    frame(&mut app, vec![]);
    app.open(elf.clone());
    finish_job(&mut app);
    let output = frame(&mut app, vec![]);
    // The first frame after a load must use the same widget IDs as later frames.
    // A user can immediately select another firmware after loading completes.
    let label = output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(t) if t.galley.text() == "second.elf" => {
                Some(t.pos + t.galley.size() * 0.5)
            }
            _ => None,
        })
        .unwrap();
    for pressed in [true, false] {
        frame(
            &mut app,
            vec![
                egui::Event::PointerMoved(label),
                egui::Event::PointerButton {
                    pos: label,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
    }
    assert!(
        app.receiver.is_some(),
        "firmware label must respond immediately after loading"
    );
    finish_job(&mut app);
    assert_eq!(
        app.analysis.as_ref().unwrap().path,
        other_elf.display().to_string()
    );
    app.open(elf.clone());
    finish_job(&mut app);
    frame(&mut app, vec![]);
    let output = frame(&mut app, vec![]);
    assert!(
        output.shapes.iter().any(|shape| matches!(
            &shape.shape,
            egui::Shape::Text(t) if t.galley.text() == "Map file (0)"
        )),
        "loaded firmware should expose its supporting-file controls"
    );

    // Refreshing the same firmware must preserve a deliberate collapse.
    let arrow = output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(t) if t.galley.text() == "app.elf" => {
                Some(t.pos + egui::vec2(-12.0, t.galley.size().y * 0.5))
            }
            _ => None,
        })
        .unwrap();
    for pressed in [true, false] {
        frame(
            &mut app,
            vec![
                egui::Event::PointerMoved(arrow),
                egui::Event::PointerButton {
                    pos: arrow,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
    }
    app.open(elf.clone());
    finish_job(&mut app);
    frame(&mut app, vec![]);
    let output = frame(&mut app, vec![]);
    assert!(!output.shapes.iter().any(|shape| matches!(
        &shape.shape,
        egui::Shape::Text(t) if t.galley.text() == "Map file (0)"
    )));

    // Loading while a search hides the collapsed row must expand it when revealed.
    app.open(other_elf);
    finish_job(&mut app);
    frame(&mut app, vec![]);
    app.artifact_search = "no-match".into();
    frame(&mut app, vec![]);
    app.open(elf.clone());
    finish_job(&mut app);
    frame(&mut app, vec![]);
    app.artifact_search.clear();
    frame(&mut app, vec![]);
    let output = frame(&mut app, vec![]);
    assert!(
        output.shapes.iter().any(|shape| matches!(
            &shape.shape,
            egui::Shape::Text(t) if t.galley.text() == "Map file (0)"
        )),
        "firmware loaded while filtered out should expand when the search is cleared"
    );
}

#[test]
fn build_files_without_firmware_does_not_load_supporting_files() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    std::fs::write(root.join("only.map"), "map preview contents").unwrap();
    std::fs::write(root.join("only.su"), "unit.c:1:1:func\t16\tstatic\n").unwrap();
    let mut app = Explorer::default();
    app.scan_build(root.clone());
    finish_job(&mut app);
    let ctx = egui::Context::default();
    ctx.style_mut(|style| style.animation_time = 0.0);
    let frame = |app: &mut Explorer, events| {
        ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1000.0, 800.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| app.build_browser(ctx),
        )
    };
    for name in ["only.map", "only.su"] {
        app.artifact_search = name.to_uppercase();
        frame(&mut app, vec![]);
        let output = frame(&mut app, vec![]);
        let pos = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(t) if t.galley.text() == name => {
                    Some(t.pos + t.galley.size() * 0.5)
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("Missing supporting file {name}"));
        for pressed in [true, false] {
            frame(
                &mut app,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
        assert!(app.receiver.is_none());
        assert!(app.analysis.is_none());
    }
}

#[test]
fn sidebar_checkboxes_and_map_radios_apply_choices_to_current_elf() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let elf = root.join("app.elf");
    std::fs::write(
        &elf,
        include_bytes!("../../../fixtures/build/gcc/cortex-m.elf"),
    )
    .unwrap();
    let other_elf = root.join("second.elf");
    std::fs::copy(&elf, &other_elf).unwrap();
    for name in ["app.map", "other.map"] {
        std::fs::write(
            root.join(name),
            include_bytes!("../../../fixtures/build/gcc/cortex-m.map"),
        )
        .unwrap();
    }
    std::fs::write(root.join("frame.su"), "diag.c:22:36:diagnose\t56\tstatic\n").unwrap();
    let mut app = Explorer::default();
    app.scan_build(root.clone());
    finish_job(&mut app);
    app.open(elf.clone());
    finish_job(&mut app);
    let ctx = egui::Context::default();
    ctx.style_mut(|style| style.animation_time = 0.0);
    fn frame(
        ctx: &egui::Context,
        app: &mut Explorer,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1000.0, 1800.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| app.build_browser(ctx),
        )
    }
    fn click(ctx: &egui::Context, app: &mut Explorer, pos: egui::Pos2) {
        for pressed in [true, false] {
            frame(
                ctx,
                app,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
    }
    // Allow the default-open collapsing sections to finish laying out.
    frame(&ctx, &mut app, vec![]);
    let output = frame(&ctx, &mut app, vec![]);
    let text_pos = |output: &egui::FullOutput, name: &str| {
        output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(t) if t.galley.text() == name => {
                    Some(t.pos + t.galley.size() * 0.5)
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("Missing sidebar label {name}"))
    };
    assert!(!output.shapes.iter().any(|shape| matches!(&shape.shape,
        egui::Shape::Text(t) if t.galley.text() == "frame.su")));
    click(&ctx, &mut app, text_pos(&output, "Stack usage files (1)"));
    let output = frame(&ctx, &mut app, vec![]);
    // Firmware rows are top-level, and supporting files are indented beneath them.
    let firmware = text_pos(&output, "app.elf");
    let map = text_pos(&output, "app.map");
    let report = text_pos(&output, "frame.su");
    assert!(map.y > firmware.y && report.y > map.y);
    assert!(map.x > firmware.x && report.x > firmware.x);
    click(&ctx, &mut app, text_pos(&output, "frame.su"));
    finish_job(&mut app);
    assert_eq!(app.saved_stack_reports(&elf), Some(vec![]));
    let output = frame(&ctx, &mut app, vec![]);
    click(&ctx, &mut app, text_pos(&output, "frame.su"));
    finish_job(&mut app);
    assert_eq!(
        app.saved_stack_reports(&elf),
        Some(vec![root.join("frame.su")])
    );
    let output = frame(&ctx, &mut app, vec![]);
    let label = text_pos(&output, "other.map");
    let radio = output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Circle(c)
                if c.center.x < label.x && (c.center.y - label.y).abs() < 2.0 =>
            {
                Some(c.center)
            }
            _ => None,
        })
        .expect("Map radio button");
    click(&ctx, &mut app, radio);
    finish_job(&mut app);
    assert!(app.map_in_use(&root.join("other.map")));
    assert!(!app.map_in_use(&root.join("app.map")));
    assert_eq!(
        app.saved_stack_reports(&elf),
        Some(vec![root.join("frame.su")])
    );
    app.artifact_search = "frame.su".into();
    let output = frame(&ctx, &mut app, vec![]);
    text_pos(&output, "app.elf");
    text_pos(&output, "second.elf");
    text_pos(&output, "frame.su");
    app.artifact_search.clear();
    // Switching via a top-level row exposes choices for the new ELF only.
    let output = frame(&ctx, &mut app, vec![]);
    click(&ctx, &mut app, text_pos(&output, "second.elf"));
    finish_job(&mut app);
    frame(&ctx, &mut app, vec![]);
    let output = frame(&ctx, &mut app, vec![]);
    assert_eq!(
        app.analysis.as_ref().unwrap().path,
        other_elf.display().to_string()
    );
    assert!(!output.shapes.iter().any(|shape| matches!(&shape.shape,
        egui::Shape::Text(t) if t.galley.text() == "frame.su")));
    click(&ctx, &mut app, text_pos(&output, "Stack usage files (1)"));
    let output = frame(&ctx, &mut app, vec![]);
    click(&ctx, &mut app, text_pos(&output, "frame.su"));
    finish_job(&mut app);
    assert_eq!(app.saved_stack_reports(&other_elf), Some(vec![]));
    assert_eq!(
        app.saved_stack_reports(&elf),
        Some(vec![root.join("frame.su")])
    );
    assert_eq!(
        app.build_settings[&root].layouts[&elf].source,
        root.join("other.map").display().to_string()
    );
    // Expanding an inactive firmware with its arrow also loads its saved choices.
    let output = frame(&ctx, &mut app, vec![]);
    let arrow = output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(t) if t.galley.text() == "app.elf" => {
                Some(t.pos + egui::vec2(-12.0, t.galley.size().y * 0.5))
            }
            _ => None,
        })
        .unwrap();
    click(&ctx, &mut app, arrow);
    finish_job(&mut app);
    frame(&ctx, &mut app, vec![]);
    let output = frame(&ctx, &mut app, vec![]);
    text_pos(&output, "frame.su");
    assert_eq!(
        app.analysis.as_ref().unwrap().path,
        elf.display().to_string()
    );
    assert!(app.map_in_use(&root.join("other.map")));
    assert_eq!(
        app.saved_stack_reports(&elf),
        Some(vec![root.join("frame.su")])
    );
    // A filename selects its map directly and keeps the firmware/report choices.
    click(&ctx, &mut app, text_pos(&output, "app.map"));
    finish_job(&mut app);
    assert!(app.map_in_use(&root.join("app.map")));
    assert_eq!(
        app.saved_stack_reports(&elf),
        Some(vec![root.join("frame.su")])
    );
}

#[test]
fn stack_folder_rules_save_immediately_and_survive_disk_restart_and_file_changes() {
    for whole_folder in [true, false] {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let elf = root.join("app.elf");
        std::fs::write(
            &elf,
            include_bytes!("../../../fixtures/build/gcc/cortex-m.elf"),
        )
        .unwrap();
        let reports = root.join("reports");
        std::fs::create_dir(&reports).unwrap();
        let first = reports.join("a.su");
        let unchecked = reports.join("b.su");
        std::fs::write(&first, "diag.c:22:36:diagnose\t56\tstatic\n").unwrap();
        std::fs::write(&unchecked, "sensor.c:9:1:sensor_init\t24\tstatic\n").unwrap();
        let preferences_file = root.join("config/workspace.json");
        let mut app = Explorer {
            preferences_file: Some(preferences_file.clone()),
            ..Default::default()
        };
        app.scan_build(root.clone());
        finish_job(&mut app);
        app.open(elf.clone());
        finish_job(&mut app);
        let paths = vec![first.clone(), unchecked.clone()];
        let mut selection = workspace::StackSelection::default();
        selection.set(if whole_folder { &reports } else { &first }, true, &paths);
        app.select_stack_selection(selection.clone());
        // Saved before the analysis finishes, without relying on on_exit.
        let saved: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&preferences_file).unwrap()).unwrap();
        let settings: std::collections::BTreeMap<PathBuf, BuildSettings> =
            serde_json::from_value(saved["build_settings"].clone()).unwrap();
        assert_eq!(settings[&root].stack_reports[&elf], selection);
        finish_job(&mut app);
        drop(app);
        std::fs::remove_file(&first).unwrap();
        let new = reports.join("new.su");
        std::fs::write(&new, "diag.c:22:36:diagnose\t80\tstatic\n").unwrap();
        let mut restored = Explorer {
            preferences_file: Some(preferences_file.clone()),
            ..Default::default()
        };
        restored.restore_preferences(true);
        finish_job(&mut restored);
        finish_job(&mut restored);
        assert!(restored.error.is_none());
        assert_eq!(
            restored.saved_stack_selection(&elf),
            Some(selection.clone())
        );
        assert!(restored.current_stack_selection().contains(&new));
        assert_eq!(
            restored.current_stack_selection().contains(&unchecked),
            whole_folder
        );
        let entries = &restored.stack.as_ref().unwrap().entries;
        assert!(entries
            .iter()
            .any(|entry| std::path::Path::new(&entry.report_file) == new));
        assert!(!entries
            .iter()
            .any(|entry| std::path::Path::new(&entry.report_file) == first));
        assert_eq!(entries.len(), if whole_folder { 2 } else { 1 });
        // Losing every report does not lose the saved directory intent.
        std::fs::remove_file(&new).unwrap();
        std::fs::remove_file(&unchecked).unwrap();
        restored.refresh();
        finish_job(&mut restored);
        assert!(restored.stack.as_ref().unwrap().entries.is_empty());
        assert_eq!(restored.saved_stack_selection(&elf), Some(selection));
        std::fs::write(&new, "diag.c:22:36:diagnose\t96\tstatic\n").unwrap();
        restored.refresh();
        finish_job(&mut restored);
        assert_eq!(restored.stack.as_ref().unwrap().entries[0].local_bytes, 96);
        let mut selection = restored.current_stack_selection();
        selection.set(&reports, false, std::slice::from_ref(&new));
        restored.select_stack_selection(selection);
        finish_job(&mut restored);
        drop(restored);
        let later = reports.join("later.su");
        std::fs::write(&later, "sensor.c:9:1:sensor_init\t32\tstatic\n").unwrap();
        let mut restored = Explorer {
            preferences_file: Some(preferences_file),
            ..Default::default()
        };
        restored.restore_preferences(true);
        finish_job(&mut restored);
        finish_job(&mut restored);
        assert!(!restored.current_stack_selection().contains(&later));
        assert!(restored.stack.as_ref().unwrap().entries.is_empty());
    }
}

#[test]
fn automatic_stack_choices_are_saved_per_elf_and_manual_choices_take_precedence() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let first_elf = root.join("app.elf");
    let second_elf = root.join("other.elf");
    for elf in [&first_elf, &second_elf] {
        std::fs::write(
            elf,
            include_bytes!("../../../fixtures/build/gcc/cortex-m.elf"),
        )
        .unwrap();
    }
    let first_dir = root.join("CMakeFiles/app.dir/src");
    let second_dir = root.join("CMakeFiles/other.dir/src");
    for directory in [&first_dir, &second_dir] {
        std::fs::create_dir_all(directory).unwrap();
    }
    let first_report = first_dir.join("diag.su");
    let second_report = second_dir.join("diag.su");
    std::fs::write(&first_report, "diag.c:22:36:diagnose\t56\tstatic\n").unwrap();
    std::fs::write(&second_report, "diag.c:22:36:diagnose\t96\tstatic\n").unwrap();
    let preferences_file = root.join("config/workspace.json");
    let mut app = Explorer {
        preferences_file: Some(preferences_file.clone()),
        ..Default::default()
    };
    app.scan_build(root.clone());
    finish_job(&mut app);
    app.open(first_elf.clone());
    finish_job(&mut app);
    assert_eq!(app.stack.as_ref().unwrap().entries[0].local_bytes, 56);
    let guessed = app.saved_stack_selection(&first_elf).unwrap();
    assert!(guessed.contains(&first_report));
    assert!(!guessed.contains(&second_report));
    assert!(guessed.paths.contains(&first_dir));
    app.open(second_elf.clone());
    finish_job(&mut app);
    assert_eq!(app.stack.as_ref().unwrap().entries[0].local_bytes, 96);
    assert!(app
        .saved_stack_selection(&second_elf)
        .unwrap()
        .contains(&second_report));
    app.open(first_elf.clone());
    finish_job(&mut app);
    drop(app);
    let new_report = first_dir.join("nested/new.su");
    std::fs::create_dir_all(new_report.parent().unwrap()).unwrap();
    std::fs::write(&new_report, "sensor.c:9:1:sensor_init\t24\tstatic\n").unwrap();
    let mut restored = Explorer {
        preferences_file: Some(preferences_file.clone()),
        ..Default::default()
    };
    restored.restore_preferences(true);
    finish_job(&mut restored);
    finish_job(&mut restored);
    assert_eq!(restored.saved_stack_selection(&first_elf), Some(guessed));
    assert_eq!(restored.stack.as_ref().unwrap().entries.len(), 2);
    restored.select_stack_reports(vec![second_report.clone()]);
    finish_job(&mut restored);
    restored.refresh();
    finish_job(&mut restored);
    assert_eq!(restored.stack.as_ref().unwrap().entries.len(), 1);
    assert_eq!(restored.stack.as_ref().unwrap().entries[0].local_bytes, 96);
    restored.select_stack_reports(vec![]);
    finish_job(&mut restored);
    drop(restored);
    let mut restored = Explorer {
        preferences_file: Some(preferences_file),
        ..Default::default()
    };
    restored.restore_preferences(true);
    finish_job(&mut restored);
    finish_job(&mut restored);
    assert!(restored.stack.as_ref().unwrap().entries.is_empty());
    assert_eq!(restored.saved_stack_reports(&first_elf), Some(vec![]));
    restored.open(second_elf.clone());
    finish_job(&mut restored);
    assert_eq!(restored.stack.as_ref().unwrap().entries[0].local_bytes, 96);
}

#[test]
fn failed_stack_selection_does_not_display_the_previous_selection_report() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let elf = root.join("app.elf");
    std::fs::write(
        &elf,
        include_bytes!("../../../fixtures/build/gcc/cortex-m.elf"),
    )
    .unwrap();
    let valid = root.join("diag.su");
    std::fs::write(&valid, "diag.c:22:36:diagnose\t56\tstatic\n").unwrap();
    let mut app = Explorer::default();
    app.scan_build(root.clone());
    finish_job(&mut app);
    app.open(elf.clone());
    finish_job(&mut app);
    assert_eq!(app.stack.as_ref().unwrap().entries.len(), 1);
    std::fs::write(&valid, [0xff]).unwrap();
    app.select_stack_reports(vec![valid.clone()]);
    finish_job(&mut app);
    assert!(app.error.is_some());
    assert_eq!(app.saved_stack_reports(&elf), Some(vec![valid]));
    assert!(app.stack.is_none());
}

#[test]
fn llvm_map_selection_imports_dependencies_and_keeps_capacity_unknown() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/build/llvm");
    let mut app = Explorer::default();
    app.scan_build(root.clone());
    finish_job(&mut app);
    let root = app.build.as_ref().unwrap().root.clone();
    let map = root.join("cortex-m.map");
    app.open(root.join("cortex-m.elf"));
    finish_job(&mut app);
    app.configure(Some(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/build/gcc/cortex-m.map"),
    ));
    finish_map_job(&mut app);
    let analysis = app.analysis.clone().unwrap();
    app.apply_map(map.clone());
    finish_map_job(&mut app);
    assert!(app.error.is_none());
    let imported = app.analysis.as_ref().unwrap();
    assert!(imported.options.regions.is_empty());
    assert_eq!(imported.dependencies.edges.len(), 16);
    assert!(app.map_in_use(&map));
    assert_eq!(
        serde_json::to_value(&imported.symbols).unwrap(),
        serde_json::to_value(&analysis.symbols).unwrap()
    );
    app.refresh();
    finish_job(&mut app);
    assert!(app.map_in_use(&map));
    assert!(app.options.regions.is_empty());
    assert_eq!(app.analysis.as_ref().unwrap().dependencies.edges.len(), 16);
    let mut restored = Explorer::default();
    restored.apply_preferences(&app.preference_value());
    finish_job(&mut restored);
    finish_job(&mut restored);
    assert!(restored.map_in_use(&map));
    assert!(restored.options.regions.is_empty());
    assert_eq!(
        restored.analysis.as_ref().unwrap().dependencies.edges.len(),
        16
    );
}

#[test]
fn tab_controls_are_independent_and_remembered_for_the_session() {
    let mut app = Explorer::default();
    for (index, view) in View::ALL.into_iter().enumerate() {
        app.change_view(view);
        assert!(app.search.is_empty());
        assert_eq!(app.sort_column, 1);
        assert_eq!(app.descending, view != View::MemoryMap);
        assert!(!app.tree);
        assert!(app.selected_file.is_none());
        assert!(app.selected_region.is_none());
        assert_eq!(app.kind_filter, "All");
        app.search = format!("filter {index}");
        app.sort_column = index;
        app.descending = index % 2 == 0;
        app.tree = index % 2 == 1;
        app.selected_file = Some(format!("file {index}"));
        app.selected_region = Some(index);
        app.kind_filter = format!("kind {index}");
    }
    app.stack_show_unresolved = true;
    app.overview_metric = overview::Metric::Ram;
    app.contributor_ram = true;
    for (index, view) in View::ALL.into_iter().enumerate() {
        app.change_view(view);
        app.change_view(view); // Clicking the active tab leaves its settings intact.
        assert_eq!(app.search, format!("filter {index}"));
        assert_eq!(app.sort_column, index);
        assert_eq!(app.descending, index % 2 == 0);
        assert_eq!(app.tree, index % 2 == 1);
        assert_eq!(app.selected_file, Some(format!("file {index}")));
        assert_eq!(app.selected_region, Some(index));
        assert_eq!(app.kind_filter, format!("kind {index}"));
    }
    assert!(app.stack_show_unresolved);
    assert!(app.overview_metric == overview::Metric::Ram);
    assert!(app.contributor_ram);

    let mut preferences = app.preference_value();
    for key in [
        "search",
        "sort_column",
        "descending",
        "tab_options",
        "directories",
        "metric",
        "contributor_ram",
        "stack_show_unresolved",
        "comparison_group",
        "only_diffs",
    ] {
        assert!(preferences.get(key).is_none(), "{key} must remain in RAM");
    }
    // Ignore display options written by older versions, too.
    preferences["directories"] = true.into();
    preferences["metric"] = "RAM".into();
    preferences["contributor_ram"] = true.into();
    let mut restarted = Explorer::default();
    restarted.apply_preferences(&preferences);
    assert!(restarted.search.is_empty());
    assert_eq!(restarted.sort_column, 1);
    assert!(!restarted.tree);
    assert!(!restarted.contributor_ram);
    assert!(restarted.overview_metric == overview::Metric::Flash);
}

#[test]
fn replacing_data_invalidates_filters_in_inactive_tabs() {
    for replacement in ["build", "firmware", "refresh", "layout", "failure"] {
        let mut app = Explorer::default();
        let folder = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/build/gcc");
        app.scan_build(folder.clone());
        finish_job(&mut app);
        let elf = app.build.as_ref().unwrap().root.join("cortex-m.elf");
        app.open(elf.clone());
        finish_job(&mut app);
        for view in View::ALL {
            app.change_view(view);
            app.search = "session search".into();
            app.sort_column = 2;
            app.descending = false;
            app.tree = true;
            app.kind_filter = "Function".into();
            app.selected_file = Some("old/source.c".into());
            app.selected_region = Some(0);
        }
        app.stack_show_unresolved = true;
        match replacement {
            "build" => app.scan_build(folder.clone()),
            "firmware" => app.open(elf),
            "refresh" => app.refresh(),
            "layout" => app.configure(Some(folder.join("cortex-m.map"))),
            "failure" => app.open(folder.join("missing.elf")),
            _ => unreachable!(),
        }
        finish_job(&mut app);
        // A build load can schedule restoration of the selected firmware.
        if app.receiver.is_some() {
            finish_job(&mut app);
        }
        assert_eq!(app.error.is_some(), replacement == "failure");
        for view in View::ALL {
            app.change_view(view);
            assert_eq!(app.search, "session search", "{replacement}");
            assert_eq!(app.sort_column, 2, "{replacement}");
            assert!(!app.descending, "{replacement}");
            assert!(app.tree, "{replacement}");
            assert_eq!(app.kind_filter, "Function", "{replacement}");
            assert_eq!(
                app.selected_region,
                (replacement == "failure").then_some(0),
                "{replacement}"
            );
            assert_eq!(
                app.selected_file.as_deref(),
                matches!(replacement, "layout" | "failure").then_some("old/source.c"),
                "{replacement}"
            );
        }
        assert!(app.stack_show_unresolved, "{replacement}");
    }
}

#[test]
fn recent_build_folders_are_unique_bounded_and_persisted() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = Explorer {
        preferences_file: Some(directory.path().join("workspace.json")),
        ..Default::default()
    };
    let folders: Vec<_> = (0..6)
        .map(|index| {
            let folder = directory.path().join(format!("build-{index}"));
            std::fs::create_dir(&folder).unwrap();
            folder.canonicalize().unwrap()
        })
        .collect();
    for folder in &folders {
        app.scan_build(folder.clone());
        finish_job(&mut app);
    }
    assert_eq!(
        app.recent_build_folders,
        folders[1..].iter().rev().cloned().collect::<Vec<_>>()
    );
    // A different spelling of the same folder must not create a duplicate.
    app.scan_build(folders[3].join("."));
    finish_job(&mut app);
    let expected = vec![
        folders[3].clone(),
        folders[5].clone(),
        folders[4].clone(),
        folders[2].clone(),
        folders[1].clone(),
    ];
    assert_eq!(app.recent_build_folders, expected);
    let mut restored = Explorer {
        preferences_file: app.preferences_file.clone(),
        ..Default::default()
    };
    restored.restore_preferences(false);
    assert_eq!(restored.recent_build_folders, expected);
    assert!(restored.receiver.is_none());
    app.scan_build(directory.path().join("missing"));
    finish_job(&mut app);
    assert!(app.error.is_some());
    assert_eq!(app.recent_build_folders, expected);
}

#[test]
fn preferences_do_not_convert_obsolete_workspace_fields() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let elf = root.join("firmware.elf");
    std::fs::write(
        &elf,
        include_bytes!("../../../fixtures/build/gcc/cortex-m.elf"),
    )
    .unwrap();
    let mut app = Explorer::default();
    app.apply_preferences_with_workspace(
        &serde_json::json!({
            "version": 1, "folder": root, "firmware": elf,
            "layout": AnalysisOptions::default(), "layout_source": "old.json",
        }),
        false,
    );
    assert!(app.recent_build_folders.is_empty());
    assert!(app.build_settings.is_empty());
    assert!(app.remembered_firmware.is_none());
    let saved = app.preference_value();
    for field in ["firmware", "layout", "layout_source"] {
        assert!(saved.get(field).is_none());
    }
}

#[test]
fn recent_build_folders_sanitize_saved_history() {
    let mut app = Explorer::default();
    app.apply_preferences_with_workspace(&serde_json::json!({"version": 1, "recent_build_folders": ["/build/a", "/build/a", "/build/b", "/build/c", "/build/d", "/build/e", "/build/f"]}), false);
    assert_eq!(
        app.recent_build_folders,
        ["/build/a", "/build/b", "/build/c", "/build/d", "/build/e"].map(PathBuf::from)
    );
}

#[test]
fn recent_folder_submenu_opens_right_and_remains_clickable() {
    check_recent_folder_submenu(1280.0, "build");
    check_recent_folder_submenu(1280.0, &"long-build-folder-".repeat(3));
}

#[test]
fn long_recent_folder_submenu_stays_right_and_clickable_in_small_windows() {
    check_recent_folder_submenu(640.0, &"long-build-folder-".repeat(12));
}

fn check_recent_folder_submenu(width: f32, folder_name: &str) {
    let directory = tempfile::tempdir().unwrap();
    let folder = directory.path().join(folder_name);
    std::fs::create_dir(&folder).unwrap();
    let folder = folder.canonicalize().unwrap();
    let mut app = Explorer::default();
    app.recent_build_folders.push(folder.clone());
    let ctx = egui::Context::default();
    shell::configure_style(&ctx);
    let frame = |app: &mut Explorer, events| {
        ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(width, 800.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| app.show(ctx),
        )
    };
    let text_rect = |output: &egui::FullOutput, text: &str| {
        output
            .shapes
            .iter()
            .rev()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text_shape) if text_shape.galley.text() == text => Some(
                    egui::Rect::from_min_size(text_shape.pos, text_shape.galley.size()),
                ),
                _ => None,
            })
            .next()
            .unwrap_or_else(|| panic!("Missing {text}"))
    };
    frame(&mut app, vec![]);
    let output = frame(&mut app, vec![]);
    let menu_text = text_rect(&output, "Menu");
    let icon_center = egui::pos2(25.0, menu_text.center().y);
    for offset in [-5.0, 0.0, 5.0] {
        assert!(
            output.shapes.iter().any(|shape| matches!(&shape.shape,
                egui::Shape::LineSegment { points, .. }
                    if (points[0] - (icon_center + egui::vec2(-8.0, offset))).length() < 1.0
                        && (points[1] - (icon_center + egui::vec2(8.0, offset))).length() < 1.0
            )),
            "Menu icon must render three horizontal lines"
        );
    }
    assert!(
        menu_text.left() >= icon_center.x + 8.0 + 8.0,
        "Menu icon and label must have a clear gap"
    );
    let menu = menu_text.center();
    for pressed in [true, false] {
        frame(
            &mut app,
            vec![
                egui::Event::PointerMoved(menu),
                egui::Event::PointerButton {
                    pos: menu,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
    }
    let output = frame(&mut app, vec![]);
    for label in [
        "Open build folder...",
        "Open recent build folder ▶",
        "Refresh",
        "Reset settings for this build folder",
        "Open configuration folder",
        "Check for updates",
        "Check for updates on startup",
        "Analysis notes",
        "Support developer",
        "About",
    ] {
        let rect = text_rect(&output, label);
        assert!(
            rect.bottom() < icon_center.y - 17.0 && rect.left() >= 0.0 && rect.right() <= width,
            "Menu action must fit above its sidebar button: {label}: {rect:?}"
        );
    }
    let refresh = text_rect(&output, "Refresh");
    let f5 = text_rect(&output, "F5");
    assert!(f5.left() > refresh.right());
    assert!((f5.center().y - refresh.center().y).abs() < 1.0);
    let open_hint = text_rect(&output, "Ctrl+O");
    assert!((open_hint.right() - f5.right()).abs() < 1.0);
    assert!(!output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == "Rescan folder")));
    let recent = text_rect(&output, "Open recent build folder ▶");
    frame(&mut app, vec![egui::Event::PointerMoved(recent.center())]);
    frame(&mut app, vec![]);
    let output = frame(&mut app, vec![]);
    let full_path = display::display_path(&folder.to_string_lossy()).into_owned();
    let child = text_rect(&output, &full_path);
    let path_shape = output
        .shapes
        .iter()
        .find(|shape| {
            matches!(&shape.shape,
                egui::Shape::Text(text) if text.galley.text() == full_path
            )
        })
        .unwrap();
    let egui::Shape::Text(path_text) = &path_shape.shape else {
        unreachable!()
    };
    assert!(
        !path_text.galley.elided,
        "Recent folder paths must be shown in full"
    );
    assert_eq!(
        path_text.galley.rows.len(),
        1,
        "Paths must stay on one line"
    );
    let visible_child = child.intersect(path_shape.clip_rect);
    assert!(
        visible_child.right() <= width,
        "Submenu must fit within the window"
    );
    assert!(visible_child.width() > 0.0);
    if width >= 1280.0 {
        assert!(
            path_shape.clip_rect.contains_rect(child),
            "Path must be fully visible when space permits"
        );
    }
    let parent = text_rect(&output, "Open build folder...");
    assert!(
        child.left() > parent.right(),
        "Submenu must be right of parent: {child:?}, {parent:?}"
    );
    frame(
        &mut app,
        vec![egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }],
    );
    let output = frame(&mut app, vec![]);
    assert!(!output.shapes.iter().any(
        |shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == "Refresh")
    ), "Escape must dismiss the parent and recent-folder submenu");
    for pressed in [true, false] {
        frame(
            &mut app,
            vec![
                egui::Event::PointerMoved(menu),
                egui::Event::PointerButton {
                    pos: menu,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
    }
    frame(&mut app, vec![egui::Event::PointerMoved(recent.center())]);
    frame(&mut app, vec![]);
    let output = frame(&mut app, vec![]);
    // The bounding rectangle of both menus includes empty space below the
    // shorter submenu. Clicking there must dismiss the menu hierarchy.
    let outside = egui::pos2(
        visible_child.center().x,
        text_rect(&output, "About").center().y,
    );
    for pressed in [true, false] {
        frame(
            &mut app,
            vec![
                egui::Event::PointerMoved(outside),
                egui::Event::PointerButton {
                    pos: outside,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
    }
    let output = frame(&mut app, vec![]);
    assert!(!output.shapes.iter().any(
        |shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == "Refresh")
    ));
    for pressed in [true, false] {
        frame(
            &mut app,
            vec![
                egui::Event::PointerMoved(menu),
                egui::Event::PointerButton {
                    pos: menu,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
    }
    frame(&mut app, vec![egui::Event::PointerMoved(recent.center())]);
    frame(&mut app, vec![]);
    frame(&mut app, vec![]);
    for pressed in [true, false] {
        frame(
            &mut app,
            vec![
                egui::Event::PointerMoved(visible_child.center()),
                egui::Event::PointerButton {
                    pos: visible_child.center(),
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
    }
    assert!(app.receiver.is_some());
    finish_job(&mut app);
    assert_eq!(app.build.as_ref().unwrap().root, folder);
    let output = frame(&mut app, vec![]);
    assert!(!output.shapes.iter().any(
        |shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == "Refresh")
    ));
    frame(
        &mut app,
        vec![egui::Event::Key {
            key: egui::Key::F1,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }],
    );
    assert!(app.show_about);
}

#[test]
fn dashboard_cards_fit_and_navigation_remains_visible_at_both_widths() {
    let analysis = snout_core::analyze_bytes(
        include_bytes!("../../../fixtures/build/gcc/cortex-m.elf"),
        "fixture.elf",
        &Default::default(),
    )
    .unwrap();
    for (width, height) in [(900.0, 600.0), (1280.0, 820.0), (1536.0, 1024.0)] {
        let ctx = egui::Context::default();
        shell::configure_style(&ctx);
        let mut app = Explorer {
            analysis: Some(Arc::new(analysis.clone())),
            ..Default::default()
        };
        let mut output = egui::FullOutput::default();
        for _ in 0..3 {
            output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width, height),
                    )),
                    ..Default::default()
                },
                |ctx| app.show(ctx),
            );
        }
        for label in View::ALL
            .iter()
            .map(|view| view.label())
            .chain(["Menu", "Build files"])
        {
            assert!(
                output.shapes.iter().any(|shape| matches!(&shape.shape,
                    egui::Shape::Text(text) if text.galley.text() == label
                        && shape.clip_rect.contains(text.pos + text.galley.size() * 0.5)
                )),
                "Control must remain visible at {width}: {label}"
            );
        }
        let position = |label: &str| {
            output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text() == label => Some(text.pos),
                    _ => None,
                })
                .unwrap()
        };
        let menu = position("Menu");
        assert!(menu.x < 165.0 && menu.y > height - 85.0);
        assert!(!output.shapes.iter().any(|shape| matches!(&shape.shape,
            egui::Shape::Text(text) if text.galley.text() == "Settings")));
        assert!(!output.shapes.iter().any(|shape| matches!(&shape.shape,
            egui::Shape::Text(text) if text.galley.text() == "ELF")));
        for label in [
            analysis.metadata.architecture.clone(),
            format!("{}-bit", analysis.metadata.bitness),
        ] {
            let metadata = position(&label);
            assert!(
                metadata.y < 140.0,
                "Firmware metadata belongs in the header: {label}"
            );
            assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,
                egui::Shape::Text(text) if text.galley.text() == label
                    && shape.clip_rect.contains(egui::Rect::from_min_size(text.pos, text.galley.size()).right_bottom())
            )), "Header metadata must be visible at {width}: {label}");
        }
        let overview = position("Overview");
        let build_files = position("Build files");
        let files = position("Files");
        assert!((build_files.x - overview.x).abs() < 0.1);
        assert!(overview.y < build_files.y && build_files.y < files.y);
        assert_eq!(
            output
                .shapes
                .iter()
                .filter(|shape| matches!(&shape.shape,
                    egui::Shape::Text(text) if text.galley.text() == "Build files"
                ))
                .count(),
            1,
            "Build files belongs only in the sidebar"
        );
        let cards: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Rect(rect) if rect.fill == egui::Color32::from_rgb(16, 25, 35) => {
                    Some((rect, shape.clip_rect))
                }
                _ => None,
            })
            .collect();
        assert!(
            cards.len() == 6,
            "Visible summary cards must render at {width}"
        );
        for title in [
            "Binary information",
            "Memory usage",
            "Section type distribution",
            "Top functions by size",
            "Top files by size",
        ] {
            assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,
                egui::Shape::Text(text) if text.galley.text() == title
                    && shape.clip_rect.contains_rect(egui::Rect::from_min_size(text.pos, text.galley.size()))
            )), "Dashboard title must be fully visible at {width}x{height}: {title}");
        }
        let top_y = cards[0].0.rect.top();
        assert_eq!(
            cards
                .iter()
                .filter(|(card, _)| (card.rect.top() - top_y).abs() < 1.0)
                .count(),
            4
        );
        for label in ["Flash", "RAM"] {
            let center = position(label);
            assert!(
                cards[..4]
                    .iter()
                    .any(|(card, _)| card.rect.contains(center)),
                "{label} chart must stay in the top row at {width}"
            );
        }
        for (card, clip) in cards {
            assert!(
                clip.expand(1.0).contains_rect(card.rect),
                "Card must fit entirely at {width}: {:?}, clip {clip:?}",
                card.rect
            );
        }
    }
}

#[test]
fn distribution_legend_opens_sections_without_resetting_overview_filters() {
    let analysis = snout_core::analyze_bytes(
        include_bytes!("../../../fixtures/build/gcc/cortex-m.elf"),
        "fixture.elf",
        &Default::default(),
    )
    .unwrap();
    let mut app = Explorer {
        search: "saved filter".into(),
        overview_metric: overview::Metric::Ram,
        ..Default::default()
    };
    let ctx = egui::Context::default();
    let frame = |app: &mut Explorer, events| {
        ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(600.0, 300.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    app.section_distribution(ui, &analysis, overview::Metric::Flash)
                });
            },
        )
    };
    frame(&mut app, vec![]);
    let output = frame(&mut app, vec![]);
    let (pos, name) = output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.text().starts_with('.') => Some((
                text.pos + text.galley.size() * 0.5,
                text.galley
                    .text()
                    .split_whitespace()
                    .next()
                    .unwrap()
                    .to_owned(),
            )),
            _ => None,
        })
        .unwrap();
    for pressed in [true, false] {
        frame(
            &mut app,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
    }
    assert!(app.view == View::Sections);
    assert_eq!(app.search, name);
    app.change_view(View::Overview);
    assert_eq!(app.search, "saved filter");
}

#[test]
fn build_files_scan_notes_remain_accessible_after_loading_firmware() {
    let analysis = snout_core::analyze_bytes(
        include_bytes!("../../../fixtures/build/gcc/cortex-m.elf"),
        "fixture.elf",
        &Default::default(),
    )
    .unwrap();
    for loaded in [false, true] {
        let mut app = Explorer {
            build: Some(Arc::new(snout_core::build::BuildFolder {
                root: PathBuf::from("build"),
                artifacts: vec![],
                warnings: vec!["build/private: Permission denied".into()],
            })),
            analysis: loaded.then(|| Arc::new(analysis.clone())),
            view: View::BuildFiles,
            ..Default::default()
        };
        let ctx = egui::Context::default();
        shell::configure_style(&ctx);
        ctx.style_mut(|style| style.animation_time = 0.0);
        let frame = |app: &mut Explorer, events| {
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1280.0, 820.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ctx| app.show(ctx),
            )
        };
        frame(&mut app, vec![]);
        let output = frame(&mut app, vec![]);
        let header = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == "1 scan notes" => {
                    Some(text.pos + text.galley.size() * 0.5)
                }
                _ => None,
            })
            .expect("Scan notes must be reachable with or without loaded firmware");
        for pressed in [true, false] {
            frame(
                &mut app,
                vec![
                    egui::Event::PointerMoved(header),
                    egui::Event::PointerButton {
                        pos: header,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
        let output = frame(&mut app, vec![]);
        assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,
            egui::Shape::Text(text) if text.galley.text() == "build/private: Permission denied")));
    }
}

#[test]
fn header_switches_elf_and_build_files_selects_support_without_leaving_the_tab() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    for name in ["app.elf", "second.elf"] {
        std::fs::write(
            root.join(name),
            include_bytes!("../../../fixtures/build/gcc/cortex-m.elf"),
        )
        .unwrap();
    }
    for name in ["app.map", "other.map"] {
        std::fs::write(
            root.join(name),
            include_bytes!("../../../fixtures/build/gcc/cortex-m.map"),
        )
        .unwrap();
    }
    std::fs::write(root.join("frame.su"), "diag.c:22:36:diagnose\t56\tstatic\n").unwrap();
    let mut app = Explorer::default();
    app.scan_build(root.clone());
    finish_job(&mut app);
    app.open(root.join("app.elf"));
    finish_job(&mut app);
    let ctx = egui::Context::default();
    shell::configure_style(&ctx);
    fn frame(
        ctx: &egui::Context,
        app: &mut Explorer,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1280.0, 820.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| app.show(ctx),
        )
    }
    fn text_pos(output: &egui::FullOutput, label: &str) -> egui::Pos2 {
        output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == label => {
                    Some(text.pos + text.galley.size() * 0.5)
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("Missing control: {label}"))
    }
    fn click(ctx: &egui::Context, app: &mut Explorer, pos: egui::Pos2) {
        for pressed in [true, false] {
            frame(
                ctx,
                app,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
    }
    frame(&ctx, &mut app, vec![]);
    let output = frame(&ctx, &mut app, vec![]);
    click(&ctx, &mut app, text_pos(&output, "app.elf"));
    let output = frame(&ctx, &mut app, vec![]);
    let second_path = root.join("second.elf");
    let second_path = second_path.to_string_lossy();
    let second_path = display::display_path(&second_path);
    let name_pos = text_pos(&output, "second.elf");
    let path_pos = text_pos(&output, &second_path);
    assert!(
        path_pos.y > name_pos.y,
        "Full path belongs below the filename"
    );
    click(&ctx, &mut app, path_pos);
    assert!(app.receiver.is_some());
    finish_job(&mut app);
    assert_eq!(
        std::path::Path::new(&app.analysis.as_ref().unwrap().path),
        root.join("second.elf")
    );
    let output = frame(&ctx, &mut app, vec![]);
    click(&ctx, &mut app, text_pos(&output, "Build files"));
    assert!(app.view == View::BuildFiles);
    let output = frame(&ctx, &mut app, vec![]);
    let report = text_pos(&output, "frame.su");
    let before = app
        .current_stack_selection()
        .contains(&root.join("frame.su"));
    click(&ctx, &mut app, report);
    finish_job(&mut app);
    assert!(app.view == View::BuildFiles);
    assert_eq!(
        app.current_stack_selection()
            .contains(&root.join("frame.su")),
        !before
    );
    let output = frame(&ctx, &mut app, vec![]);
    let map = text_pos(&output, "other.map");
    let radio = output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Circle(circle)
                if circle.center.x < map.x && (circle.center.y - map.y).abs() < 2.0 =>
            {
                Some(circle.center)
            }
            _ => None,
        })
        .unwrap();
    click(&ctx, &mut app, radio);
    finish_job(&mut app);
    assert!(app.map_in_use(&root.join("other.map")));
    assert!(app.view == View::BuildFiles);
    let output = frame(&ctx, &mut app, vec![]);
    click(&ctx, &mut app, text_pos(&output, "other.map"));
    assert!(
        app.receiver.is_none(),
        "clicking the active map must not reload or open contents"
    );
    assert!(app.view == View::BuildFiles);
    let output = frame(&ctx, &mut app, vec![]);
    click(&ctx, &mut app, text_pos(&output, "app.map"));
    assert!(app.receiver.is_some());
    finish_job(&mut app);
    assert!(app.map_in_use(&root.join("app.map")));
    assert!(app.view == View::BuildFiles);
}

#[test]
fn overview_matches_reference_column_and_memory_bar_alignment() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/build/gcc");
    let options = snout_core::map::parse_map_regions(
        &std::fs::read_to_string(root.join("cortex-m.map")).unwrap(),
    )
    .unwrap();
    let analysis = analyze_path(root.join("cortex-m.elf"), &options).unwrap();
    let mut app = Explorer {
        analysis: Some(Arc::new(analysis.clone())),
        ..Default::default()
    };
    let ctx = egui::Context::default();
    shell::configure_style(&ctx);
    let mut output = egui::FullOutput::default();
    for _ in 0..3 {
        output = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1280.0, 820.0),
                )),
                ..Default::default()
            },
            |ctx| app.show(ctx),
        );
    }
    let text_pos = |label: &str| {
        output
            .shapes
            .iter()
            .rev()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == label => Some(text.pos),
                _ => None,
            })
            .unwrap_or_else(|| panic!("Missing reference label: {label}"))
    };
    let pairs = [
        (
            "Format",
            format!(
                "ELF ({}-bit, {} endian)",
                analysis.metadata.bitness, analysis.metadata.endianness
            ),
        ),
        ("Architecture", analysis.metadata.architecture.clone()),
        (
            "Entry point",
            format!("0x{:08x}", analysis.metadata.entry_point),
        ),
        (
            "ELF file size",
            snout_core::format_bytes(analysis.metadata.file_size),
        ),
        ("Debug info", "DWARF present".into()),
    ];
    let label_x = text_pos(pairs[0].0).x;
    let value_x = text_pos(&pairs[0].1).x;
    assert!(value_x > label_x + 70.0);
    for (label, value) in pairs {
        let left = text_pos(label);
        let right = text_pos(&value);
        assert!((left.x - label_x).abs() < 0.1 && (right.x - value_x).abs() < 0.1);
        assert!((left.y - right.y).abs() < 0.1);
    }
    let bars: Vec<_> = output
        .shapes
        .iter()
        .filter_map(|shape| match &shape.shape {
            egui::Shape::Rect(rect) if rect.fill == egui::Color32::from_rgb(45, 60, 79) => {
                Some(rect.rect)
            }
            _ => None,
        })
        .collect();
    assert_eq!(bars.len(), 2);
    for (index, kind) in [snout_core::MemoryKind::Flash, snout_core::MemoryKind::Ram]
        .into_iter()
        .enumerate()
    {
        let region = analysis
            .options
            .regions
            .iter()
            .find(|region| region.kind == kind)
            .unwrap();
        let usage = snout_core::regions::region_usage(&analysis, region);
        let detail = format!(
            "{} / {} · {:.1}% · {} free",
            snout_core::format_bytes(usage.used),
            snout_core::format_bytes(region.size),
            usage.used as f64 * 100.0 / region.size as f64,
            snout_core::format_bytes(usage.free)
        );
        let position = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text)
                    if text
                        .galley
                        .text()
                        .starts_with(&detail[..detail.find(" ·").unwrap()]) =>
                {
                    Some(text.pos)
                }
                _ => None,
            })
            .expect("Region occupancy detail must be visible");
        assert!((bars[index].height() - 14.0).abs() < 0.1);
        assert!(position.y > bars[index].bottom());
    }
    assert!(!output.shapes.iter().any(|shape| matches!(&shape.shape,
        egui::Shape::Text(text) if text.galley.text() == "Memory breakdown"
    )));
}

#[test]
fn overview_region_defaults_use_percentage_and_validate_saved_names() {
    use snout_core::{MemoryKind, MemoryRegion};
    let mut a = snout_core::analyze_bytes(
        include_bytes!("../../../fixtures/build/gcc/cortex-m.elf"),
        "fixture",
        &Default::default(),
    )
    .unwrap();
    a.options.regions = vec![
        MemoryRegion {
            name: "large".into(),
            start: 0,
            size: 1000,
            kind: MemoryKind::Ram,
        },
        MemoryRegion {
            name: "tight".into(),
            start: 1000,
            size: 100,
            kind: MemoryKind::Ram,
        },
        MemoryRegion {
            name: "rom".into(),
            start: 2000,
            size: 100,
            kind: MemoryKind::Flash,
        },
        MemoryRegion {
            name: "tie".into(),
            start: 3000,
            size: 200,
            kind: MemoryKind::Ram,
        },
    ];
    let mut app = Explorer {
        region_cache: [(500, 500), (90, 10), (100, 0), (180, 20)]
            .into_iter()
            .map(|(used, free)| snout_core::regions::RegionUsage {
                used,
                free,
                symbols: vec![],
            })
            .collect(),
        ..Default::default()
    };
    assert_eq!(app.overview_region(&a, 1, MemoryKind::Ram), Some(1));
    assert_eq!(app.overview_region(&a, 0, MemoryKind::Flash), Some(2));
    app.overview_regions[1] = Some("large".into());
    assert_eq!(app.overview_region(&a, 1, MemoryKind::Ram), Some(0));
    app.overview_regions[1] = Some("rom".into());
    assert_eq!(app.overview_region(&a, 1, MemoryKind::Ram), Some(1));
    app.overview_regions[1] = Some("missing".into());
    assert_eq!(app.overview_region(&a, 1, MemoryKind::Ram), Some(1));
    a.options.regions.clear();
    assert_eq!(app.overview_region(&a, 1, MemoryKind::Ram), None);
}

#[test]
fn overview_memory_keeps_totals_and_bars_visible_with_many_regions() {
    let options = snout_core::map::parse_map_regions(include_str!(
        "../../../fixtures/build/gcc/cortex-m.map"
    ))
    .unwrap();
    let mut analysis = snout_core::analyze_bytes(
        include_bytes!("../../../fixtures/build/gcc/cortex-m.elf"),
        "fixture",
        &options,
    )
    .unwrap();
    let original = analysis.options.regions.clone();
    for i in 0..30 {
        let mut region = original[i % original.len()].clone();
        region.name = format!("extra_{i}");
        region.start = 0x60000000 + i as u64 * 0x100000;
        analysis.options.regions.push(region);
    }
    for width in [220.0, 340.0, 500.0] {
        let ctx = egui::Context::default();
        shell::configure_style(&ctx);
        let mut app = Explorer::default();
        app.ensure_region_cache(&analysis);
        let mut output = egui::FullOutput::default();
        for _ in 0..2 {
            output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width, 220.0),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| app.compact_memory(ui, &analysis));
                },
            );
        }
        assert!(!output.shapes.iter().any(|shape| matches!(&shape.shape,
            egui::Shape::Text(t) if matches!(t.galley.text(), "Flash payload" | "Static RAM")
        )));
        assert_eq!(
            output
                .shapes
                .iter()
                .filter(|shape| matches!(&shape.shape,
                    egui::Shape::Rect(r) if r.fill == egui::Color32::from_rgb(45, 60, 79)
                ))
                .count(),
            2
        );
        assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,
            egui::Shape::Text(t) if t.galley.text().starts_with("View all regions") && shape.clip_rect.contains_rect(egui::Rect::from_min_size(t.pos, t.galley.size()))
        )));
    }
}

#[test]
fn overview_memory_dropdown_and_regions_link_work_and_unknown_capacity_has_no_bar() {
    use snout_core::{MemoryKind, MemoryRegion};
    let options = snout_core::map::parse_map_regions(include_str!(
        "../../../fixtures/build/gcc/cortex-m.map"
    ))
    .unwrap();
    let mut a = snout_core::analyze_bytes(
        include_bytes!("../../../fixtures/build/gcc/cortex-m.elf"),
        "fixture",
        &options,
    )
    .unwrap();
    a.options.regions.push(MemoryRegion {
        name: "spare_RAM".into(),
        start: 0x60000000,
        size: 4096,
        kind: MemoryKind::Ram,
    });
    let mut app = Explorer::default();
    app.ensure_region_cache(&a);
    let ctx = egui::Context::default();
    let frame = |app: &mut Explorer, events| {
        ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(500.0, 240.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| app.compact_memory(ui, &a));
            },
        )
    };
    let text_position = |output: &egui::FullOutput, label: &str| {
        output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(t) if t.galley.text() == label => {
                    Some(t.pos + t.galley.size() * 0.5)
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("Missing {label}"))
    };
    let click = |app: &mut Explorer, pos| {
        let mut output = egui::FullOutput::default();
        for pressed in [true, false] {
            output = frame(
                app,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
        output
    };
    frame(&mut app, vec![]);
    let output = frame(&mut app, vec![]);
    let ram = app.overview_region(&a, 1, MemoryKind::Ram).unwrap();
    let pos = text_position(&output, &a.options.regions[ram].name);
    click(&mut app, pos);
    let output = frame(&mut app, vec![]);
    click(&mut app, text_position(&output, "spare_RAM"));
    assert_eq!(app.overview_regions[1].as_deref(), Some("spare_RAM"));
    let output = frame(&mut app, vec![]);
    assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,
        egui::Shape::Text(t) if t.galley.text() == "0.0%"
    )));
    app.tab_options[View::MemoryMap as usize].search = "old filter".into();
    click(
        &mut app,
        text_position(
            &output,
            &format!("View all regions ({})", a.options.regions.len()),
        ),
    );
    assert!(app.view == View::MemoryMap);
    assert!(app.search.is_empty());

    a.options.regions.clear();
    app.ensure_region_cache(&a);
    let output = ctx.run(egui::RawInput::default(), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| app.compact_memory(ui, &a));
    });
    assert!(!output.shapes.iter().any(|shape| matches!(&shape.shape,
        egui::Shape::Text(text) if text.galley.text() == "Capacity unknown"
    )));
    for label in [
        "No map selected",
        "Capacity and free space are unknown.",
        "Open Build files",
    ] {
        text_position(&output, label);
    }
    for label in [
        "Flash payload".to_owned(),
        "Static RAM".to_owned(),
        snout_core::format_bytes(a.totals.flash),
        snout_core::format_bytes(a.totals.ram),
    ] {
        assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,
            egui::Shape::Text(t) if t.galley.text() == label
                && shape.clip_rect.contains_rect(egui::Rect::from_min_size(t.pos, t.galley.size()))
        )), "Missing known usage without map capacities: {label}");
    }
    assert!(!output.shapes.iter().any(|shape| matches!(&shape.shape,
        egui::Shape::Rect(r) if r.fill == egui::Color32::from_rgb(45, 60, 79)
    )));
}

#[test]
fn overview_region_choices_survive_disk_restart_and_are_scoped_to_elf_and_build() {
    use snout_core::MemoryKind;
    let directory = tempfile::tempdir().unwrap();
    let parent = directory.path().canonicalize().unwrap();
    let folder = parent.join("build");
    std::fs::create_dir(&folder).unwrap();
    for name in ["first", "second"] {
        std::fs::write(
            folder.join(format!("{name}.elf")),
            include_bytes!("../../../fixtures/build/gcc/cortex-m.elf"),
        )
        .unwrap();
        std::fs::write(
            folder.join(format!("{name}.map")),
            include_str!("../../../fixtures/build/gcc/cortex-m.map"),
        )
        .unwrap();
    }
    let config = parent.join("config/workspace.json");
    let first = folder.join("first.elf");
    let second = folder.join("second.elf");
    let mut app = Explorer {
        preferences_file: Some(config.clone()),
        ..Default::default()
    };
    app.scan_build(folder.clone());
    finish_job(&mut app);
    app.open(first.clone());
    finish_job(&mut app);
    let a = app.analysis.clone().unwrap();
    let flash = a
        .options
        .regions
        .iter()
        .find(|r| r.kind == MemoryKind::Flash)
        .unwrap()
        .name
        .clone();
    let ram = a
        .options
        .regions
        .iter()
        .find(|r| r.kind == MemoryKind::Ram)
        .unwrap()
        .name
        .clone();
    app.select_overview_region(&a, 0, flash.clone());
    app.select_overview_region(&a, 1, ram.clone());
    let choices = [Some(flash.clone()), Some(ram.clone())];
    let disk: serde_json::Value = serde_json::from_slice(&std::fs::read(&config).unwrap()).unwrap();
    assert_eq!(
        disk["build_settings"][folder.to_str().unwrap()]["overview_regions"]
            [first.to_str().unwrap()],
        serde_json::json!(choices)
    );
    app.open(second);
    finish_job(&mut app);
    assert_eq!(app.overview_regions, [None, None]);
    let a = app.analysis.clone().unwrap();
    app.select_overview_region(&a, 0, flash);
    app.open(first.clone());
    finish_job(&mut app);
    assert_eq!(app.overview_regions, choices);
    app.refresh();
    finish_job(&mut app);
    assert_eq!(app.overview_regions, choices);
    // The exact same ELF path can be scanned through a different build root.
    app.scan_build(parent);
    finish_job(&mut app);
    app.open(first);
    finish_job(&mut app);
    assert_eq!(app.overview_regions, [None, None]);
    let a = app.analysis.clone().unwrap();
    app.select_overview_region(&a, 1, ram);
    app.scan_build(folder);
    finish_job(&mut app);
    finish_job(&mut app);
    assert_eq!(app.overview_regions, choices);
    let mut restarted = Explorer {
        preferences_file: Some(config),
        ..Default::default()
    };
    restarted.restore_preferences(true);
    finish_job(&mut restarted);
    finish_job(&mut restarted);
    assert!(restarted.error.is_none(), "{:?}", restarted.error);
    assert_eq!(restarted.overview_regions, choices);
    assert_eq!(
        restarted.analysis.as_ref().unwrap().path,
        app.analysis.as_ref().unwrap().path
    );
}

#[test]
fn preferences_without_overview_regions_load_without_migration() {
    let directory = tempfile::tempdir().unwrap();
    let folder = directory.path().canonicalize().unwrap();
    let elf = folder.join("firmware.elf");
    std::fs::write(
        &elf,
        include_bytes!("../../../fixtures/build/gcc/cortex-m.elf"),
    )
    .unwrap();
    let value = serde_json::json!({"version": 1, "folder": folder,
        "build_settings": {folder.to_string_lossy(): {"firmware": elf, "layouts": {}}}
    });
    let mut app = Explorer::default();
    app.apply_preferences(&value);
    finish_job(&mut app);
    finish_job(&mut app);
    assert!(app.analysis.is_some());
    assert!(app.error.is_none());
    assert_eq!(app.overview_regions, [None, None]);
    assert!(app.build_settings.contains_key(&folder));
}

#[test]
fn distribution_colors_are_distinct_match_legend_and_ignore_section_names() {
    let mut a = snout_core::analyze_bytes(
        include_bytes!("../../../fixtures/build/gcc/cortex-m.elf"),
        "fixture",
        &Default::default(),
    )
    .unwrap();
    let template = a.sections[0].clone();
    a.sections = [
        "text",
        "rodata",
        "datas",
        "device_area",
        "shell_subcommands",
        "custom",
        "extra",
    ]
    .into_iter()
    .enumerate()
    .map(|(i, name)| {
        let mut section = template.clone();
        section.name = name.into();
        section.usage.flash = (7 - i) as u64 * 1000;
        section
    })
    .collect();
    let render_colors = |a: &snout_core::Analysis| {
        let ctx = egui::Context::default();
        let mut app = Explorer::default();
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(600.0, 250.0),
                )),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    app.section_distribution(ui, a, overview::Metric::Flash)
                });
            },
        );
        let slices: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Mesh(mesh) => mesh
                    .vertices
                    .iter()
                    .find(|v| v.color != egui::Color32::TRANSPARENT)
                    .map(|v| v.color),
                _ => None,
            })
            .collect();
        let markers: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Circle(circle) if circle.radius == 5.0 => Some(circle.fill),
                _ => None,
            })
            .collect();
        assert_eq!(slices.len(), 6);
        assert_eq!(slices, markers);
        for (i, color) in slices.iter().enumerate() {
            assert!(
                !slices[..i].contains(color),
                "Every visible slice must have its own color"
            );
        }
        slices
    };
    let colors = render_colors(&a);
    for section in &mut a.sections {
        section.name.insert(0, '.');
    }
    assert_eq!(render_colors(&a), colors);
}

#[test]
fn section_distributions_use_separate_flash_and_static_ram_sizes() {
    let mut analysis = snout_core::analyze_bytes(
        include_bytes!("../../../fixtures/build/gcc/cortex-m.elf"),
        "fixture",
        &Default::default(),
    )
    .unwrap();
    let template = analysis.sections[0].clone();
    analysis.sections = [(".data", 900, 100), (".text", 100, 0), (".bss", 0, 300)]
        .into_iter()
        .map(|(name, flash, ram)| {
            let mut section = template.clone();
            section.name = name.into();
            section.usage = snout_core::Usage { flash, ram };
            section
        })
        .collect();
    for (metric, expected, excluded) in [
        (
            overview::Metric::Flash,
            [".data", ".text", "90.0%", "10.0%"],
            ".bss",
        ),
        (
            overview::Metric::Ram,
            [".bss", ".data", "75.0%", "25.0%"],
            ".text",
        ),
    ] {
        let ctx = egui::Context::default();
        let mut app = Explorer::default();
        let render = |app: &mut Explorer, analysis: &snout_core::Analysis| {
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(300.0, 210.0),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default()
                        .show(ctx, |ui| app.section_distribution(ui, analysis, metric));
                },
            )
        };
        let output = render(&mut app, &analysis);
        let texts: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text) => Some(text.galley.text()),
                _ => None,
            })
            .collect();
        for label in expected.into_iter().chain([metric.label()]) {
            assert!(
                texts.contains(&label),
                "Missing {label} in {} chart",
                metric.label()
            );
        }
        assert!(!texts.contains(&excluded));
        let label = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == metric.label() => Some(text),
                _ => None,
            })
            .unwrap();
        let bounds = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Mesh(mesh) => Some(mesh.calc_bounds()),
                _ => None,
            })
            .reduce(|a, b| a.union(b))
            .unwrap();
        assert!((label.pos + label.galley.size() * 0.5 - bounds.center()).length() < 1.0);
        let mut empty = analysis.clone();
        for section in &mut empty.sections {
            match metric {
                overview::Metric::Ram => section.usage.ram = 0,
                _ => section.usage.flash = 0,
            }
        }
        let output = render(&mut app, &empty);
        assert!(!output
            .shapes
            .iter()
            .any(|shape| matches!(shape.shape, egui::Shape::Mesh(_))));
    }
}

#[test]
fn dashboard_distribution_legends_remain_readable_at_minimum_window_size() {
    let mut analysis = snout_core::analyze_bytes(
        include_bytes!("../../../fixtures/build/gcc/cortex-m.elf"),
        "fixture.elf",
        &Default::default(),
    )
    .unwrap();
    let template = analysis.sections[0].clone();
    let names = [".text", ".data", ".bss", ".rodata", ".init", ".extra"];
    analysis.sections = names
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let mut section = template.clone();
            section.name = (*name).into();
            section.usage = snout_core::Usage {
                flash: (6 - i) as u64 * 100,
                ram: (6 - i) as u64 * 100,
            };
            section
        })
        .collect();
    let mut app = Explorer {
        analysis: Some(Arc::new(analysis)),
        ..Default::default()
    };
    let ctx = egui::Context::default();
    shell::configure_style(&ctx);
    let mut output = egui::FullOutput::default();
    for _ in 0..3 {
        output = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(900.0, 600.0),
                )),
                ..Default::default()
            },
            |ctx| app.show(ctx),
        );
    }
    for name in names[..5].iter().copied().chain(["Other"]) {
        let labels: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == name => {
                    Some((text, shape.clip_rect))
                }
                _ => None,
            })
            .collect();
        assert_eq!(labels.len(), 2, "Both distributions must show {name}");
        for (text, clip) in labels {
            let rendered: String = text
                .galley
                .rows
                .iter()
                .flat_map(|row| row.glyphs.iter().map(|glyph| glyph.chr))
                .collect();
            assert_eq!(
                rendered, name,
                "Legend name must not collapse into an ellipsis"
            );
            assert!(clip.contains_rect(egui::Rect::from_min_size(text.pos, text.galley.size())));
        }
    }
}

#[test]
fn dashboard_memory_controls_fit_at_minimum_window_size() {
    let options = snout_core::map::parse_map_regions(include_str!(
        "../../../fixtures/build/gcc/cortex-m.map"
    ))
    .unwrap();
    let analysis = snout_core::analyze_bytes(
        include_bytes!("../../../fixtures/build/gcc/cortex-m.elf"),
        "fixture.elf",
        &options,
    )
    .unwrap();
    let mut app = Explorer {
        analysis: Some(Arc::new(analysis)),
        ..Default::default()
    };
    let ctx = egui::Context::default();
    shell::configure_style(&ctx);
    let mut output = egui::FullOutput::default();
    for _ in 0..3 {
        output = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(900.0, 600.0),
                )),
                ..Default::default()
            },
            |ctx| app.show(ctx),
        );
    }
    let bars: Vec<_> = output
        .shapes
        .iter()
        .filter_map(|shape| match &shape.shape {
            egui::Shape::Rect(rect) if rect.fill == egui::Color32::from_rgb(45, 60, 79) => {
                Some((rect.rect, shape.clip_rect))
            }
            _ => None,
        })
        .collect();
    assert_eq!(bars.len(), 2);
    for (bar, clip) in bars {
        assert!(
            clip.contains_rect(bar),
            "Memory bar {bar:?} clipped by {clip:?}"
        );
    }
    assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,
        egui::Shape::Text(text) if text.galley.text().starts_with("View all regions")
            && shape.clip_rect.contains_rect(egui::Rect::from_min_size(text.pos, text.galley.size()))
    )), "Region navigation must be visible");
}

#[test]
fn firmware_dropdown_scrolling_keeps_the_last_row_at_the_bottom() {
    use snout_core::build::{Artifact, ArtifactKind, BuildFolder};
    let mut app = Explorer {
        build: Some(Arc::new(BuildFolder {
            root: PathBuf::from("/build"),
            artifacts: (0..1000)
                .map(|i| Artifact {
                    path: PathBuf::from(format!("/build/app_{i:04}.elf")),
                    kind: ArtifactKind::Firmware,
                })
                .collect(),
            warnings: vec![],
        })),
        ..Default::default()
    };
    let ctx = egui::Context::default();
    shell::configure_style(&ctx);
    let frame = |app: &mut Explorer, events| {
        ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1000.0, 600.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| app.firmware_selector(ui));
            },
        )
    };
    frame(&mut app, vec![]);
    let pos = egui::pos2(100.0, 30.0);
    for pressed in [true, false] {
        frame(
            &mut app,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
    }
    for _ in 0..3 {
        frame(&mut app, vec![]);
    }
    frame(
        &mut app,
        vec![
            egui::Event::PointerMoved(egui::pos2(100.0, 130.0)),
            egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0.0, -100000.0),
                modifiers: egui::Modifiers::NONE,
            },
        ],
    );
    let mut output = egui::FullOutput::default();
    for _ in 0..30 {
        output = frame(&mut app, vec![]);
    }
    let (row, clip) = output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.text() == "app_0999.elf" => Some((
                egui::Rect::from_min_size(text.pos, text.galley.size()),
                shape.clip_rect,
            )),
            _ => None,
        })
        .expect("Last firmware should be visible after scrolling to the end");
    assert!(clip.contains_rect(row));
    let path = output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.text() == "/build/app_0999.elf" => {
                assert_eq!(text.galley.job.sections[0].format.font_id.size, 12.0);
                Some(egui::Rect::from_min_size(text.pos, text.galley.size()))
            }
            _ => None,
        })
        .expect("Each dropdown row must show the full firmware path");
    assert!(path.top() > row.bottom());
    assert!(clip.contains_rect(path));
    assert!(
        clip.bottom() - path.bottom() < 26.0,
        "Unexpected blank space below final row: path {path:?}, clip {clip:?}"
    );
    let pos = row.center();
    frame(&mut app, vec![egui::Event::PointerMoved(pos)]);
    let hovered = frame(&mut app, vec![]);
    assert!(
        hovered.shapes.iter().any(|shape| matches!(&shape.shape,
            egui::Shape::Rect(rect) if rect.rect.contains(pos)
                && rect.fill == egui::Color32::from_rgb(23, 40, 49)
        )),
        "Hovered firmware rows must use the application's navigation highlight"
    );
    for pressed in [true, false] {
        frame(
            &mut app,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
    }
    assert!(
        app.receiver.is_some(),
        "The final firmware must remain selectable"
    );
}

#[test]
fn dashboard_rankings_show_ten_functions_and_refresh_after_report_replacement() {
    let mut analysis = snout_core::analyze_bytes(
        include_bytes!("../../../fixtures/build/gcc/cortex-m.elf"),
        "fixture.elf",
        &Default::default(),
    )
    .unwrap();
    let template = analysis
        .symbols
        .iter()
        .find(|s| s.kind == "Function")
        .unwrap()
        .clone();
    analysis.symbols = (0..12)
        .map(|index| {
            let mut symbol = template.clone();
            symbol.name = format!("ranked_function_{index:02}");
            symbol.demangled_name = symbol.name.clone();
            symbol.usage.flash = 100 + index;
            symbol
        })
        .collect();
    let mut constant = template;
    constant.name = "large_constant".into();
    constant.demangled_name = constant.name.clone();
    constant.kind = "Constant".into();
    constant.usage.flash = 1_000_000;
    analysis.symbols.push(constant);
    let ctx = egui::Context::default();
    shell::configure_style(&ctx);
    let mut app = Explorer {
        analysis: Some(Arc::new(analysis)),
        ..Default::default()
    };
    let frame = |app: &mut Explorer| {
        ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1920.0, 1200.0),
                )),
                ..Default::default()
            },
            |ctx| app.show(ctx),
        )
    };
    frame(&mut app);
    let output = frame(&mut app);
    let names: Vec<_> = output
        .shapes
        .iter()
        .filter_map(|shape| match &shape.shape {
            egui::Shape::Text(t) if t.galley.text().starts_with("ranked_function_") => {
                Some(t.galley.text().to_owned())
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        names,
        (2..12)
            .rev()
            .map(|i| format!("ranked_function_{i:02}"))
            .collect::<Vec<_>>()
    );
    assert!(!output.shapes.iter().any(|shape| matches!(&shape.shape,
        egui::Shape::Text(t) if t.galley.text() == "large_constant"
    )));
    let mut replacement = (**app.analysis.as_ref().unwrap()).clone();
    replacement.symbols[0].usage.flash = 2_000_000;
    app.analysis = Some(Arc::new(replacement));
    let output = frame(&mut app);
    let first = output.shapes.iter().find_map(|shape| match &shape.shape {
        egui::Shape::Text(t) if t.galley.text().starts_with("ranked_function_") => {
            Some(t.galley.text())
        }
        _ => None,
    });
    assert_eq!(first, Some("ranked_function_00"));
}

#[test]
fn dashboard_file_labels_match_baseline_tables_and_reuse_path_cache() {
    let mut current = snout_core::analyze_bytes(
        include_bytes!("../../../fixtures/build/gcc/cortex-m.elf"),
        "fixture.elf",
        &Default::default(),
    )
    .unwrap();
    current.symbols.truncate(1);
    current.symbols[0].source_file = Some("/project/app/src/main.c".into());
    current.symbols[0].dwarf_compilation_unit = current.symbols[0].source_file.clone();
    current.symbols[0].compilation_unit = None;
    current.files = vec![snout_core::FileUsage {
        path: "/project/app/src/main.c".into(),
        attribution: "DWARF".into(),
        usage: snout_core::Usage { flash: 4, ram: 0 },
        symbol_count: 1,
    }];
    current.dependencies.nodes.clear();
    let mut old = current.clone();
    old.files[0].path = "/project/lib/src/main.c".into();
    old.symbols[0].source_file = Some(old.files[0].path.clone());
    old.symbols[0].dwarf_compilation_unit = old.symbols[0].source_file.clone();
    let mut app = Explorer {
        baseline_display: Some(baseline_display::BaselineDisplay::new(
            &current, &old, None, None,
        )),
        analysis: Some(Arc::new(current)),
        ..Default::default()
    };
    let display = app.baseline_display_analysis().unwrap();
    let paths = app.source_paths(&display);
    assert_eq!(paths.short(&display.files[0].path), "app/src/main.c");
    let ctx = egui::Context::default();
    shell::configure_style(&ctx);
    for _ in 0..2 {
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1920.0, 1200.0),
                )),
                ..Default::default()
            },
            |ctx| app.show(ctx),
        );
        assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,
            egui::Shape::Text(text) if text.galley.text() == "app/src/main.c")));
        assert!(std::rc::Rc::ptr_eq(&paths, &app.source_paths(&display)));
    }
}

#[test]
fn sections_expose_tls_template_and_variable_details_without_inventing_total_ram() {
    let mut analysis = snout_core::analyze_bytes(
        include_bytes!("../../../fixtures/build/gcc/cortex-m.elf"),
        "fixture.elf",
        &Default::default(),
    )
    .unwrap();
    analysis.tls = Some(snout_core::TlsReport {
        source: "PT_TLS".into(),
        initialized_size: 8,
        zero_initialized_size: 24,
        template_size: 32,
        alignment: 8,
        total_runtime_ram: None,
        symbols: vec![snout_core::TlsSymbol {
            name: "thread_counter".into(),
            offset: 8,
            size: 4,
            section: ".tbss".into(),
        }],
    });
    let totals = analysis.totals;
    let mut app = Explorer {
        analysis: Some(Arc::new(analysis)),
        view: View::Sections,
        ..Default::default()
    };
    let ctx = egui::Context::default();
    shell::configure_style(&ctx);
    let frame = |app: &mut Explorer, events| {
        ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1280.0, 820.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| app.show(ctx),
        )
    };
    app.search = "stale section filter".into();
    // The dashboard link must reveal details even after a user collapses them.
    ctx.style_mut(|style| style.animation_time = 0.0);
    frame(&mut app, vec![]);
    let output = frame(&mut app, vec![]);
    let header = output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.text() == "Thread-local storage (TLS)" => {
                Some(text.pos + text.galley.size() * 0.5)
            }
            _ => None,
        })
        .unwrap();
    for pressed in [true, false] {
        frame(
            &mut app,
            vec![
                egui::Event::PointerMoved(header),
                egui::Event::PointerButton {
                    pos: header,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
    }
    let output = frame(&mut app, vec![]);
    assert!(!output.shapes.iter().any(|shape| matches!(&shape.shape,
        egui::Shape::Text(text) if text.galley.text().contains("Template per thread:"))));
    app.change_view(View::Overview);
    frame(&mut app, vec![]);
    let output = frame(&mut app, vec![]);
    let link = output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.text() == "Thread-local storage details" => {
                Some(text.pos + text.galley.size() * 0.5)
            }
            _ => None,
        })
        .expect("TLS details must be reachable from the dashboard");
    for pressed in [true, false] {
        frame(
            &mut app,
            vec![
                egui::Event::PointerMoved(link),
                egui::Event::PointerButton {
                    pos: link,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
    }
    assert!(app.view == View::Sections);
    assert!(app.search.is_empty());
    let mut output = egui::FullOutput::default();
    for _ in 0..3 {
        output = frame(&mut app, vec![]);
    }
    let texts: Vec<_> = output
        .shapes
        .iter()
        .filter_map(|shape| match &shape.shape {
            egui::Shape::Text(text) => Some(text.galley.text()),
            _ => None,
        })
        .collect();
    assert!(
        texts
            .iter()
            .any(|text| text.contains("Template per thread: 32 B")),
        "{texts:?}"
    );
    assert!(texts.iter().any(|text| text.contains("8 B initialized")
        && text.contains("24 B zero-initialized")
        && text.contains("alignment 8 B")));
    assert!(texts
        .iter()
        .any(|text| text.contains("Total TLS RAM is unknown")));
    assert!(texts.iter().any(|text| text.contains("thread_counter")
        && text.contains("0x00000008")
        && text.contains("4 B")
        && text.contains(".tbss")));
    assert_eq!(app.analysis.as_ref().unwrap().totals, totals);
}

#[test]
fn build_files_selection_panels_fit_small_and_large_windows() {
    for size in [egui::vec2(900.0, 600.0), egui::vec2(1280.0, 820.0)] {
        let mut app = Explorer::default();
        app.scan_build(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/build/gcc"));
        finish_job(&mut app);
        app.open(app.build.as_ref().unwrap().root.join("cortex-m.elf"));
        finish_job(&mut app);
        app.change_view(View::BuildFiles);
        let ctx = egui::Context::default();
        shell::configure_style(&ctx);
        let frame = |app: &mut Explorer| {
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                    ..Default::default()
                },
                |ctx| app.show(ctx),
            )
        };
        frame(&mut app);
        let output = frame(&mut app);
        for label in [
            "Map file",
            "Stack reports",
            "Load map file...",
            "Autodetect map file",
        ] {
            let shape = output
                .shapes
                .iter()
                .find(|shape| {
                    matches!(&shape.shape,
                        egui::Shape::Text(text) if text.galley.text() == label
                    )
                })
                .unwrap_or_else(|| panic!("Missing {label} at {size:?}"));
            if let egui::Shape::Text(text) = &shape.shape {
                let center = text.pos + text.galley.size() * 0.5;
                assert!(
                    shape.clip_rect.contains(center),
                    "Clipped {label} at {size:?}: {center:?}"
                );
                assert!(
                    egui::Rect::from_min_size(egui::Pos2::ZERO, size).contains(center),
                    "Offscreen {label} at {size:?}: {center:?}"
                );
            }
        }
        assert!(!output.shapes.iter().any(|shape| matches!(&shape.shape,
            egui::Shape::Text(text) if ["Back to file selection", "Back to firmware", "Detected format: GNU ld", "Cross-reference map", "Load cross-reference map...", "Back to Overview"].contains(&text.galley.text())
        )));
    }
}

#[test]
fn overview_no_map_warning_and_build_files_link_fit_and_navigate() {
    let analysis = snout_core::analyze_bytes(
        include_bytes!("../../../fixtures/build/gcc/cortex-m.elf"),
        "fixture.elf",
        &Default::default(),
    )
    .unwrap();
    for size in [egui::vec2(900.0, 600.0), egui::vec2(1280.0, 820.0)] {
        let mut app = Explorer {
            analysis: Some(Arc::new(analysis.clone())),
            layout_source: "ELF inference".into(),
            ..Default::default()
        };
        let ctx = egui::Context::default();
        shell::configure_style(&ctx);
        let frame = |app: &mut Explorer, events| {
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                    events,
                    ..Default::default()
                },
                |ctx| app.show(ctx),
            )
        };
        frame(&mut app, vec![]);
        let output = frame(&mut app, vec![]);
        let mut link = None;
        for label in [
            "Flash payload",
            "Static RAM",
            "No map selected",
            "Capacity and free space are unknown.",
            "Open Build files",
        ] {
            let matches: Vec<_> = output
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text() == label => {
                        Some((shape.clip_rect, text))
                    }
                    _ => None,
                })
                .collect();
            assert_eq!(matches.len(), 1, "{label} at {size:?}");
            let (clip, text) = matches[0];
            let rect = egui::Rect::from_min_size(text.pos, text.galley.size());
            assert!(
                clip.contains_rect(rect),
                "Clipped {label} at {size:?}: {rect:?} outside {clip:?}"
            );
            if label == "Open Build files" {
                link = Some(rect.center());
            }
        }
        assert!(!output.shapes.iter().any(|shape| matches!(&shape.shape,
            egui::Shape::Text(text) if text.galley.text() == "Capacity unknown"
        )));
        let pos = link.unwrap();
        for pressed in [true, false] {
            frame(
                &mut app,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
        assert!(app.view == View::BuildFiles);
    }
}

// Tests of persistence and report loading explicitly keep manually selected maps.
fn finish_map_job(app: &mut Explorer) {
    finish_job(app);
    if app.map_warning.is_some() {
        app.resolve_map_warning(true);
        finish_job(app);
    }
}

#[test]
fn map_warning_blocks_background_and_reverts_or_ignores_without_losing_report() {
    let mut app = Explorer::default();
    app.scan_build(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/build/gcc"));
    finish_job(&mut app);
    let root = app.build.as_ref().unwrap().root.clone();
    let elf = root.join("cortex-m.elf");
    let map = root.join("cortex-m.map");
    let stale = root.join("cortex-m-grown.map");
    app.open(elf.clone());
    finish_job(&mut app);
    let original = app.analysis.clone().unwrap();
    let preferences = app.preference_value();
    let revision = app.report_revision;
    app.apply_map(stale.clone());
    finish_job(&mut app);
    assert!(app
        .map_warning
        .as_ref()
        .unwrap()
        .reasons
        .iter()
        .any(|r| r.contains("size:")));
    assert!(Arc::ptr_eq(&original, app.analysis.as_ref().unwrap()));
    assert_eq!(preferences, app.preference_value());
    assert_eq!(revision, app.report_revision);
    assert!(app.map_in_use(&map));

    let ctx = egui::Context::default();
    shell::configure_style(&ctx);
    fn frame(
        ctx: &egui::Context,
        app: &mut Explorer,
        events: Vec<egui::Event>,
        dropped_files: Vec<egui::DroppedFile>,
    ) -> egui::FullOutput {
        ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(900.0, 600.0),
                )),
                events,
                dropped_files,
                ..Default::default()
            },
            |ctx| app.show(ctx),
        )
    }
    fn click(ctx: &egui::Context, app: &mut Explorer, pos: egui::Pos2) {
        for pressed in [true, false] {
            frame(
                ctx,
                app,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
                vec![],
            );
        }
    }
    fn text_center(output: &egui::FullOutput, label: &str) -> egui::Pos2 {
        output
            .shapes
            .iter()
            .find_map(|s| match &s.shape {
                egui::Shape::Text(t) if t.galley.text() == label => {
                    Some(t.pos + t.galley.size() * 0.5)
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("Missing modal text: {label}"))
    }
    frame(&ctx, &mut app, vec![], vec![]);
    let output = frame(&ctx, &mut app, vec![], vec![]);
    let rect = ctx
        .memory(|m| m.area_rect(egui::Id::new("map_mismatch")))
        .unwrap();
    assert!(
        (rect.center() - egui::pos2(450.0, 300.0)).length() < 2.0,
        "{rect:?}"
    );
    assert!(rect.min.y >= 0.0 && rect.max.y <= 600.0);
    text_center(&output, "Why it does not match");
    // Clicking the backdrop and sending shortcuts/dropped firmware must not change the report.
    click(&ctx, &mut app, egui::pos2(10.0, 10.0));
    frame(
        &ctx,
        &mut app,
        vec![egui::Event::Key {
            key: egui::Key::F5,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }],
        vec![egui::DroppedFile {
            path: Some(elf),
            ..Default::default()
        }],
    );
    assert!(app.receiver.is_none());
    assert!(app.map_warning.is_some());
    assert!(Arc::ptr_eq(&original, app.analysis.as_ref().unwrap()));
    let output = frame(&ctx, &mut app, vec![], vec![]);
    click(&ctx, &mut app, text_center(&output, "Revert"));
    assert!(app.map_warning.is_none());
    assert!(app.receiver.is_none());
    assert_eq!(preferences, app.preference_value());
    assert!(Arc::ptr_eq(&original, app.analysis.as_ref().unwrap()));

    app.apply_map(stale.clone());
    finish_job(&mut app);
    frame(&ctx, &mut app, vec![], vec![]);
    let output = frame(&ctx, &mut app, vec![], vec![]);
    click(&ctx, &mut app, text_center(&output, "Ignore"));
    assert!(app.map_warning.is_none());
    finish_job(&mut app);
    assert!(app.map_in_use(&stale));
    assert!(!Arc::ptr_eq(&original, app.analysis.as_ref().unwrap()));
    assert_ne!(preferences, app.preference_value());
    app.apply_map(map);
    finish_job(&mut app);
    assert!(app.map_warning.is_none());
}
