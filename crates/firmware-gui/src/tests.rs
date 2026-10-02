use super::*;

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
                app.show_details = expanded;
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
    click(&ctx, &mut app, text_position(&output, ".text"));
    assert!(app.show_details);
    assert_eq!(app.details.as_ref().unwrap().0, ".text");
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
    assert!(!app.show_details);
}
