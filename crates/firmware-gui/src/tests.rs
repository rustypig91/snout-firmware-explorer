use super::*;

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
fn map_rediscovery_commits_layout_only_after_successful_analysis() {
    let mut app = Explorer::default();
    app.scan_build(PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures"
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
fn folder_workflow_selects_firmware_and_loads_stack_automatically() {
    let mut app = Explorer::default();
    app.scan_build(PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures"
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
        .any(|command| matches!(command, egui::ViewportCommand::Title(title) if title.ends_with("fixtures - Rusty's Snout - Firmware Explorer"))));
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
        include_bytes!("../../../fixtures/cortex-m.elf"),
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
        include_bytes!("../../../fixtures/cortex-m.elf"),
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
                "/../../fixtures/cortex-m-main.su"
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
        include_bytes!("../../../fixtures/cortex-m.elf"),
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
        include_bytes!("../../../fixtures/cortex-m.elf"),
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
        include_bytes!("../../../fixtures/cortex-m.elf"),
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
    app.load_build_stack();
    let result = app
        .receiver
        .take()
        .unwrap()
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap()
        .unwrap();
    std::fs::remove_dir_all(&root).unwrap();
    let Loaded::Stack(report) = result else {
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
        include_bytes!("../../../fixtures/cortex-m.elf"),
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
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/cortex-m.elf"),
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
    app.scan_build(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures"));
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
fn preferences_restore_selected_firmware_layout_and_view() {
    let mut original = Explorer::default();
    original.scan_build(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures"));
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
    app.scan_build(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures"));
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
