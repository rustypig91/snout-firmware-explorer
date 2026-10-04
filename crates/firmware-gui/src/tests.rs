use super::*;
use firmware_analysis_core::stack::analyze_stack;

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
fn overview_explains_reservations_and_links_growth_to_section_comparison() {
    let old = analyze_path(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/build/cortex-m.elf"),
        &Default::default(),
    )
    .unwrap();
    let new = analyze_path(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/build/cortex-m-grown.elf"),
        &Default::default(),
    )
    .unwrap();
    let mut app = Explorer {
        analysis: Some(Arc::new(new.clone())),
        comparison: Some(compare(&old, &new)),
        ..Default::default()
    };
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
                    egui::vec2(1280.0, 1600.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| app.show(ctx),
        )
    }
    fn click_text(ctx: &egui::Context, app: &mut Explorer, text: &str) {
        let output = frame(ctx, app, vec![]);
        let pos = output
            .shapes
            .iter()
            .find_map(|s| match &s.shape {
                egui::Shape::Text(t) if t.galley.text() == text => {
                    Some(t.pos + t.galley.size() * 0.5)
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("Missing text: {text}"));
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
    let has_text = |text: &str| {
        output
            .shapes
            .iter()
            .any(|s| matches!(&s.shape, egui::Shape::Text(t) if t.galley.text() == text))
    };
    assert!(has_text("Flash: 1.45 KiB used | Capacity unknown"));
    assert!(has_text("RAM: 388 B used | Capacity unknown"));
    assert!(!has_text("Flash payload"));
    assert!(!has_text("Static RAM"));
    assert!(has_text("RAM code: 28 B"));
    assert!(output.shapes.iter().any(
        |s| matches!(&s.shape, egui::Shape::Text(t) if t.galley.text().ends_with("src/main.c"))
    ));
    click_text(&ctx, &mut app, "Explain 128 B unattributed RAM");
    frame(&ctx, &mut app, vec![]);
    click_text(
        &ctx,
        &mut app,
        "128 B in .reserved · reserved without a source owner",
    );
    assert!(app.overview_metric == overview::Metric::Ram);
    assert_eq!(
        app.overview_section,
        new.sections
            .iter()
            .find(|s| s.name == ".reserved")
            .map(|s| s.index)
    );
    click_text(&ctx, &mut app, "RAM +32 B | .bss");
    assert!(app.view == View::Compare);
    assert_eq!(app.comparison_group, 2);
    assert_eq!(app.search, ".bss");
    frame(&ctx, &mut app, vec![]);
    assert_eq!(app.visible_rows, 1);
}

#[test]
fn same_named_symbols_at_different_addresses_expand_independently() {
    let mut analysis = analyze_path(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/build/cortex-m.elf"),
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
fn refresh_reloads_uppercase_maps_and_json_layouts_and_keeps_comparison() {
    let folder = tempfile::tempdir().unwrap();
    let elf = folder.path().join("app.elf");
    std::fs::write(&elf, include_bytes!("../../../fixtures/build/cortex-m.elf")).unwrap();
    let map = folder.path().join("manual.MAP");
    std::fs::write(&map, include_bytes!("../../../fixtures/build/cortex-m.map")).unwrap();
    let layout = folder.path().join("memory.JSON");
    std::fs::write(
        &layout,
        include_bytes!("../../../examples/cortex-m-memory.json"),
    )
    .unwrap();
    let mut app = Explorer::default();
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
        finish_job(&mut app);
        let old = app.analysis.as_ref().unwrap().as_ref().clone();
        app.job(move || Ok(Loaded::Baseline(old)));
        finish_job(&mut app);
        if source == &map {
            std::fs::write(
                source,
                include_str!("../../../fixtures/build/cortex-m-grown.map").replacen(
                    "0x00040000",
                    "0x00080000",
                    1,
                ),
            )
            .unwrap();
        } else {
            let mut options = app.options.clone();
            options.regions[0].size *= 2;
            std::fs::write(source, serde_json::to_vec(&options).unwrap()).unwrap();
        }
        let previous = app.options.clone();
        std::fs::write(
            &elf,
            include_bytes!("../../../fixtures/build/cortex-m-grown.elf"),
        )
        .unwrap();
        app.refresh();
        finish_job(&mut app);
        assert!(app.error.is_none(), "{:?}", app.error);
        assert_ne!(app.options, previous);
        assert!(app.comparison.is_some());
        assert!(app.baseline.is_some());
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
        include_bytes!("../../../fixtures/build/CMakeFiles/cortex-m-objects.dir/src/main.c.su"),
    )
    .unwrap();
    let analysis = analyze_path(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/build/cortex-m-stripped.elf"),
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
        .join("../../fixtures/build/cortex-m-grown.elf")
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
fn active_map_follows_analysis_instead_of_preview_selection() {
    let mut app = Explorer::default();
    let folder = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/build");
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
    app.preview = Some((other_map.clone(), String::new()));
    assert!(app.map_in_use(&map));
    assert!(!app.map_in_use(&other_map));
    app.apply_map(other_map.clone());
    finish_job(&mut app);
    assert!(!app.map_in_use(&map));
    assert!(app.map_in_use(&other_map));
    app.refresh();
    finish_job(&mut app);
    assert!(app.map_in_use(&other_map));
    app.apply_map(root.join("missing.map"));
    finish_job(&mut app);
    assert!(app.error.is_some());
    assert!(app.map_in_use(&other_map));
    let mut restored = Explorer::default();
    restored.apply_preferences(&app.preference_value());
    finish_job(&mut restored);
    finish_job(&mut restored);
    assert!(restored.map_in_use(&other_map));
    app.configure(Some(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/cortex-m-memory.json"),
    ));
    finish_job(&mut app);
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
fn unsupported_matching_map_is_not_marked_in_use() {
    let folder = tempfile::tempdir().unwrap();
    std::fs::write(
        folder.path().join("app.elf"),
        include_bytes!("../../../fixtures/build/cortex-m.elf"),
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
        "/../../fixtures/build"
    )));
    finish_job(&mut app);
    let path = app.build.as_ref().unwrap().root.join("cortex-m.elf");
    app.open(path.clone());
    finish_job(&mut app);
    app.configure(Some(PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/cortex-m-memory.json"
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
            include_bytes!("../../../fixtures/build/cortex-m.elf"),
        )
        .unwrap();
    }
    std::fs::write(
        folder.path().join("app.map"),
        include_bytes!("../../../fixtures/build/cortex-m.map"),
    )
    .unwrap();
    std::fs::write(
        folder.path().join("manual.map"),
        include_bytes!("../../../fixtures/build/cortex-m-grown.map"),
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
        finish_job(&mut app);
    }
    app.preview = Some((root.join("manual.map"), "Preview".into()));
    let preferences = app.preference_value();
    let analysis = app.analysis.clone().unwrap();
    let preview = app.preview.clone();
    std::fs::write(root.join("app.elf"), b"incomplete rebuild").unwrap();
    app.reset_build_settings();
    finish_job(&mut app);
    assert!(app.error.is_some());
    assert!(Arc::ptr_eq(&analysis, app.analysis.as_ref().unwrap()));
    assert_eq!(app.preference_value(), preferences);
    assert_eq!(app.preview, preview);
    assert_eq!(app.build_settings[&root].layouts.len(), 2);

    std::fs::write(
        root.join("app.elf"),
        include_bytes!("../../../fixtures/build/cortex-m.elf"),
    )
    .unwrap();
    app.reset_build_settings();
    finish_job(&mut app);
    assert!(app.error.is_none());
    assert!(app.build_settings[&root].layouts.is_empty());
    assert!(app.map_in_use(&root.join("app.map")));
    assert!(app.preview.is_none());
}

#[test]
fn folder_workflow_selects_firmware_and_loads_stack_automatically() {
    let mut app = Explorer::default();
    app.scan_build(PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/build"
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
            .filter(|a| a.kind == firmware_analysis_core::build::ArtifactKind::Firmware)
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
        .any(|command| matches!(command, egui::ViewportCommand::Title(title) if title.ends_with("build - Rusty's Snout - Firmware Explorer"))));
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
    let analysis = firmware_analysis_core::analyze_bytes(
        include_bytes!("../../../fixtures/build/cortex-m.elf"),
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
    let analysis = firmware_analysis_core::analyze_bytes(
        include_bytes!("../../../fixtures/build/cortex-m.elf"),
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
                "/../../fixtures/build/CMakeFiles/cortex-m-objects.dir/src/main.c.su"
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
                        View::Compare => app.compare_view(ui),
                    });
                },
            );
            assert!(!output.shapes.is_empty());
        }
    }
}

#[test]
fn compact_shell_renders_all_views_with_optional_panes() {
    let analysis = firmware_analysis_core::analyze_bytes(
        include_bytes!("../../../fixtures/build/cortex-m.elf"),
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
    let analysis = firmware_analysis_core::analyze_bytes(
        include_bytes!("../../../fixtures/build/cortex-m.elf"),
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
    let mut analysis = firmware_analysis_core::analyze_bytes(
        include_bytes!("../../../fixtures/build/cortex-m.elf"),
        "fixture.elf",
        &Default::default(),
    )
    .unwrap();
    analysis.path = root.join("firmware.elf").to_string_lossy().into_owned();
    let mut app = Explorer {
        analysis: Some(Arc::new(analysis)),
        ..Default::default()
    };
    app.build = Some(Arc::new(
        firmware_analysis_core::build::scan_folder(&root).unwrap(),
    ));
    app.select_stack_reports(vec![root.clone()]);
    let result = app
        .receiver
        .take()
        .unwrap()
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap()
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
fn configured_region_symbols_render_and_search() {
    let options =
        serde_json::from_str(include_str!("../../../examples/cortex-m-memory.json")).unwrap();
    let analysis = firmware_analysis_core::analyze_bytes(
        include_bytes!("../../../fixtures/build/cortex-m.elf"),
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
        app.selected_region = Some(region);
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
                assert!(app.visible_rows > 0);
            } else {
                assert_eq!(app.visible_rows, 0);
            }
        }
    }
}

#[test]
fn overview_mouse_back_returns_from_section_to_root() {
    let analysis = analyze_path(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/build/cortex-m.elf"),
        &AnalysisOptions::default(),
    )
    .unwrap();
    let section = analysis
        .sections
        .iter()
        .find(|s| s.allocated && s.size > 0)
        .unwrap()
        .index;
    let mut app = Explorer {
        analysis: Some(Arc::new(analysis)),
        overview_section: Some(section),
        overview_unit: Some(pie::UnitKey::Other),
        ..Default::default()
    };
    let ctx = egui::Context::default();
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1280.0, 820.0),
        )),
        ..Default::default()
    };
    let output = ctx.run(input.clone(), |ctx| app.show(ctx));
    assert!(!output.shapes.is_empty());
    assert_eq!(app.overview_section, Some(section));
    let mut back = input;
    back.events.push(egui::Event::PointerButton {
        pos: egui::pos2(600.0, 400.0),
        button: egui::PointerButton::Extra1,
        pressed: true,
        modifiers: egui::Modifiers::NONE,
    });
    let _ = ctx.run(back.clone(), |ctx| app.show(ctx));
    assert_eq!(app.overview_section, Some(section));
    assert_eq!(app.overview_unit, None);
    if let egui::Event::PointerButton { pressed, .. } = &mut back.events[0] {
        *pressed = false;
    }
    let _ = ctx.run(back.clone(), |ctx| app.show(ctx));
    if let egui::Event::PointerButton { pressed, .. } = &mut back.events[0] {
        *pressed = true;
    }
    let _ = ctx.run(back, |ctx| app.show(ctx));
    assert_eq!(app.overview_section, None);
}

#[test]
fn refresh_restores_selection_and_layout_and_preserves_report_on_failure() {
    let mut app = Explorer::default();
    app.scan_build(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/build"));
    finish_job(&mut app);
    let path = app.build.as_ref().unwrap().root.join("cortex-m.elf");
    app.open(path.clone());
    finish_job(&mut app);
    app.configure(Some(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/cortex-m-memory.json"),
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
    let first = folder.path().join("first.elf");
    let second = folder.path().join("second.elf");
    for path in [&first, &second] {
        std::fs::write(path, include_bytes!("../../../fixtures/build/cortex-m.elf")).unwrap();
    }
    let value = serde_json::json!({
        "version": 1, "folder": folder.path(), "firmware": first,
    });
    for explicit in [None, Some(second.clone())] {
        let mut app = Explorer::default();
        app.apply_preferences_with_workspace(&value, false);
        assert!(app.receiver.is_none());
        app.open_startup(&startup::Startup {
            folder: Some(folder.path().to_owned()),
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
    let first = folder.path().join("first.elf");
    let second = folder.path().join("second.elf");
    let map = folder.path().join("manual.map");
    for path in [&first, &second] {
        std::fs::write(path, include_bytes!("../../../fixtures/build/cortex-m.elf")).unwrap();
    }
    std::fs::write(&map, include_bytes!("../../../fixtures/build/cortex-m.map")).unwrap();
    let mut original = Explorer::default();
    original.scan_build(folder.path().to_owned());
    finish_job(&mut original);
    original.open(second.clone());
    finish_job(&mut original);
    original.apply_map(map.clone());
    finish_job(&mut original);
    original.open(first);
    finish_job(&mut original);

    let updated_map = include_str!("../../../fixtures/build/cortex-m.map").replacen(
        "0x00040000",
        "0x00080000",
        1,
    );
    std::fs::write(&map, &updated_map).unwrap();
    let expected = firmware_analysis_core::build::parse_map_regions(&updated_map).unwrap();
    let mut restored = Explorer::default();
    restored.apply_preferences_with_workspace(&original.preference_value(), false);
    restored.open_startup(&startup::Startup {
        folder: Some(folder.path().to_owned()),
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
    let mut app = Explorer::default();
    app.apply_preferences(&serde_json::json!({"version": 1, "folder": folder.path()}));
    app.open_startup(&startup::Startup::default());
    finish_job(&mut app);
    assert_eq!(app.build.as_ref().unwrap().root, folder.path());
    assert!(app.analysis.is_none());
    assert!(app.receiver.is_none());
    assert!(app.error.is_none());
}

#[test]
fn reopening_same_folder_restores_firmware_and_layout() {
    let mut app = Explorer::default();
    app.scan_build(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/build"));
    finish_job(&mut app);
    let folder = app.build.as_ref().unwrap().root.clone();
    let elf = folder.join("cortex-m.elf");
    app.open(elf.clone());
    finish_job(&mut app);
    app.configure(Some(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/cortex-m-memory.json"),
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
    let elf = folder.path().join("firmware.elf");
    std::fs::write(&elf, include_bytes!("../../../fixtures/build/cortex-m.elf")).unwrap();
    let value = serde_json::json!({"version": 1, "folder": folder.path(), "firmware": elf});
    let mut app = Explorer::default();
    app.apply_preferences_with_workspace(&value, false);
    std::fs::remove_file(elf).unwrap();
    app.open_startup(&startup::Startup {
        folder: Some(folder.path().to_owned()),
        ..Default::default()
    });
    finish_job(&mut app);
    assert!(app.analysis.is_none());
    assert!(app.receiver.is_none());
    assert!(app.error.is_none());
    assert_eq!(app.build.as_ref().unwrap().root, folder.path());
}

#[test]
fn preferences_restore_selected_firmware_layout_and_view() {
    let mut original = Explorer::default();
    original.scan_build(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/build"));
    finish_job(&mut original);
    original.open(original.build.as_ref().unwrap().root.join("cortex-m.elf"));
    finish_job(&mut original);
    original.configure(Some(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/cortex-m-memory.json"),
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
    assert!(restored.tree);
    assert!(restored.overview_metric == overview::Metric::Ram);
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
    app.scan_build(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/build"));
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
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/cortex-m-memory.json"),
    ));
    finish_job(&mut app);
    let a = app.analysis.clone().unwrap();
    app.ensure_region_cache(&a);
    assert_eq!(app.region_cache.len(), a.options.regions.len());
    assert!(!app.region_cache.is_empty());
    for (region, usage) in a.options.regions.iter().zip(&app.region_cache) {
        assert_eq!(
            usage.used,
            firmware_analysis_core::regions::region_usage(&a, region).used
        );
    }
}

#[test]
fn startup_elf_is_selected_after_scan_with_map_and_stack_reports() {
    let folder = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/build");
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
fn stack_view_scopes_rows_to_selected_elf_and_keeps_unresolved_available() {
    let analysis = firmware_analysis_core::analyze_bytes(
        include_bytes!("../../../fixtures/build/cortex-m.elf"),
        "fixture.elf",
        &Default::default(),
    )
    .unwrap();
    let mut report = analyze_stack(
        &analysis,
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../fixtures/build/CMakeFiles/cortex-m-objects.dir/src/main.c.su"
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
fn supporting_file_preview_is_confined_to_overview_and_preserves_elf() {
    let mut app = Explorer::default();
    app.scan_build(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/build"));
    finish_job(&mut app);
    let build = app.build.clone().unwrap();
    app.open(build.root.join("cortex-m.elf"));
    finish_job(&mut app);
    let analysis = app.analysis.clone().unwrap();
    let ctx = egui::Context::default();
    for kind in [
        firmware_analysis_core::build::ArtifactKind::Map,
        firmware_analysis_core::build::ArtifactKind::StackUsage,
    ] {
        app.change_view(View::Symbols);
        app.select_artifact(
            build
                .artifacts
                .iter()
                .find(|a| a.kind == kind)
                .unwrap()
                .clone(),
        );
        finish_job(&mut app);
        assert!(app.view == View::Overview);
        assert!(Arc::ptr_eq(&analysis, app.analysis.as_ref().unwrap()));
        // Use distinctive text to verify what the shell actually renders on every tab.
        app.preview.as_mut().unwrap().1 = "supporting-file-preview-marker".into();
        for view in View::ALL {
            app.change_view(view);
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
            let preview_visible = output.shapes.iter().any(|s| matches!(&s.shape, egui::Shape::Text(t) if t.galley.text().contains("supporting-file-preview-marker")));
            assert_eq!(preview_visible, view == View::Overview, "{}", view.label());
            assert!(Arc::ptr_eq(&analysis, app.analysis.as_ref().unwrap()));
        }
    }
}

#[test]
fn map_choices_survive_elf_folder_switching_restart_and_folder_reset() {
    let mut app = Explorer::default();
    app.scan_build(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/build"));
    finish_job(&mut app);
    let root = app.build.as_ref().unwrap().root.clone();
    let elf = root.join("cortex-m.elf");
    let other_elf = root.join("cortex-m-grown.elf");
    let map = root.join("cortex-m.map");
    let other_map = root.join("cortex-m-grown.map");
    app.open(elf.clone());
    finish_job(&mut app);
    app.apply_map(other_map.clone());
    finish_job(&mut app);
    app.open(other_elf.clone());
    finish_job(&mut app);
    app.apply_map(map.clone());
    finish_job(&mut app);
    app.open(elf.clone());
    finish_job(&mut app);
    assert!(app.map_in_use(&other_map));

    let folder = tempfile::tempdir().unwrap();
    std::fs::write(
        folder.path().join("app.elf"),
        include_bytes!("../../../fixtures/build/cortex-m.elf"),
    )
    .unwrap();
    std::fs::write(
        folder.path().join("manual.map"),
        include_bytes!("../../../fixtures/build/cortex-m.map"),
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
    finish_job(&mut app);

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
        include_bytes!("../../../fixtures/build/cortex-m-grown.map"),
    )
    .unwrap();
    restored.refresh();
    finish_job(&mut restored);
    let expected = firmware_analysis_core::build::parse_map_regions(include_str!(
        "../../../fixtures/build/cortex-m-grown.map"
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
fn dependency_map_choices_survive_refresh_restart_and_failed_import() {
    let folder = tempfile::tempdir().unwrap();
    std::fs::write(
        folder.path().join("app.elf"),
        include_bytes!("../../../fixtures/build/cortex-m.elf"),
    )
    .unwrap();
    let map = folder.path().join("manual.map");
    let table =
        "Cross Reference Table\n\nSymbol File\nReset_Handler  main.o\ndiagnose  diag.o\n  main.o\n";
    std::fs::write(&map, table).unwrap();
    let mut app = Explorer::default();
    app.scan_build(folder.path().into());
    finish_job(&mut app);
    let firmware = app.build.as_ref().unwrap().root.join("app.elf");
    app.open(firmware.clone());
    finish_job(&mut app);
    assert!(app.analysis.as_ref().unwrap().dependencies.edges.is_empty());
    app.apply_dependency_map(map.clone());
    finish_job(&mut app);
    assert!(app.view == View::Dependencies);
    assert_eq!(app.analysis.as_ref().unwrap().dependencies.edges.len(), 1);
    assert_eq!(app.saved_dependency_map(&firmware), Some(map.clone()));
    let prefs = app.preference_value();
    app.apply_dependency_map(folder.path().join("missing.map"));
    finish_job(&mut app);
    assert!(app.error.is_some());
    assert_eq!(app.analysis.as_ref().unwrap().dependencies.edges.len(), 1);
    std::fs::write(&map, format!("{table}initialized  main.o\n  diag.o\n")).unwrap();
    app.refresh();
    finish_job(&mut app);
    assert_eq!(app.analysis.as_ref().unwrap().dependencies.edges.len(), 2);
    let mut restored = Explorer::default();
    restored.apply_preferences(&prefs);
    finish_job(&mut restored);
    finish_job(&mut restored);
    assert!(restored.view == View::Dependencies);
    assert_eq!(
        restored.analysis.as_ref().unwrap().dependencies.edges.len(),
        2
    );
    std::fs::remove_file(&map).unwrap();
    restored.refresh();
    finish_job(&mut restored);
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
        .notes
        .iter()
        .any(|n| n.contains("manual.map")));
    restored.configure(None);
    finish_job(&mut restored);
    assert!(restored
        .analysis
        .as_ref()
        .unwrap()
        .dependencies
        .notes
        .iter()
        .any(|n| n.contains("manual.map")));
    std::fs::write(&map, table).unwrap();
    restored.configure(None);
    finish_job(&mut restored);
    assert_eq!(
        restored.analysis.as_ref().unwrap().dependencies.edges.len(),
        1
    );
    assert_eq!(restored.saved_dependency_map(&firmware), Some(map));
    restored.reset_build_settings();
    finish_job(&mut restored);
    assert_eq!(restored.saved_dependency_map(&firmware), None);
}

#[test]
fn committed_fixture_renders_six_bubbles_and_sixteen_dependency_arrowheads() {
    let mut app = Explorer::default();
    app.scan_build(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/build"));
    finish_job(&mut app);
    app.open(app.build.as_ref().unwrap().root.join("cortex-m.elf"));
    finish_job(&mut app);
    assert_eq!(app.analysis.as_ref().unwrap().dependencies.edges.len(), 16);
    app.change_view(View::Dependencies);
    let ctx = egui::Context::default();
    shell::configure_style(&ctx);
    let output = ctx.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 900.0),
            )),
            ..Default::default()
        },
        |ctx| app.show(ctx),
    );
    let bubbles = output
        .shapes
        .iter()
        .filter(|shape| {
            matches!(&shape.shape,
                egui::Shape::Circle(circle) if circle.fill == egui::Color32::from_rgb(40, 100, 140)
            )
        })
        .count();
    let arrow_color = ctx.style().visuals.text_color().gamma_multiply(0.8);
    let arrows = output.shapes.iter().filter(|shape| matches!(&shape.shape,
        egui::Shape::Path(path) if path.closed && path.points.len() == 3 && path.fill == arrow_color
    )).count();
    assert_eq!(bubbles, 6);
    assert_eq!(arrows, 16);
}

#[test]
fn stack_selection_is_build_independent_persisted_per_elf_and_refreshed() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let elf = root.join("app.elf");
    let other = root.join("other.elf");
    let bytes = include_bytes!("../../../fixtures/build/cortex-m.elf");
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
    std::fs::write(&elf, include_bytes!("../../../fixtures/build/cortex-m.elf")).unwrap();
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
fn sidebar_checkboxes_and_map_radios_apply_choices_to_current_elf() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let elf = root.join("app.elf");
    std::fs::write(&elf, include_bytes!("../../../fixtures/build/cortex-m.elf")).unwrap();
    for name in ["app.map", "other.map"] {
        std::fs::write(
            root.join(name),
            include_bytes!("../../../fixtures/build/cortex-m.map"),
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
}

#[test]
fn stack_folder_rules_save_immediately_and_survive_disk_restart_and_file_changes() {
    for whole_folder in [true, false] {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let elf = root.join("app.elf");
        std::fs::write(&elf, include_bytes!("../../../fixtures/build/cortex-m.elf")).unwrap();
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
        std::fs::write(elf, include_bytes!("../../../fixtures/build/cortex-m.elf")).unwrap();
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
