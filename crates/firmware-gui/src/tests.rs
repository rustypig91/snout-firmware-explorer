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
