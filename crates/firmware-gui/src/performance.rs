//! Reproducible headless UI timings; run with --ignored --nocapture --test-threads=1.
use super::*;

#[test]
#[ignore = "manual performance measurement"]
fn large_report_latency() {
    let mut analysis = firmware_analysis_core::analyze_bytes(
        include_bytes!("../../../fixtures/build/cortex-m.elf"),
        "large.elf",
        &Default::default(),
    )
    .unwrap();
    let template = analysis.symbols[0].clone();
    analysis.symbols = (0..50_000)
        .map(|i| {
            let mut symbol = template.clone();
            symbol.demangled_name = format!("function_{i:05}");
            symbol.name = symbol.demangled_name.clone();
            symbol.size = (i * 7919 % 65536) as u64;
            symbol
        })
        .collect();
    let mut app = Explorer {
        view: View::Symbols,
        ..Default::default()
    };
    let ctx = egui::Context::default();
    let frame = |app: &mut Explorer| {
        let start = std::time::Instant::now();
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1200.0, 800.0),
                )),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| app.symbols(ui, &analysis));
            },
        );
        start.elapsed().as_secs_f64() * 1000.0
    };
    println!("50,000 symbols, cold frame: {:.3} ms", frame(&mut app));
    let mut times: Vec<_> = (0..30).map(|_| frame(&mut app)).collect();
    times.sort_by(f64::total_cmp);
    println!(
        "unchanged frame median: {:.3} ms; p95: {:.3} ms",
        times[15], times[28]
    );
    app.search = "function_1".into();
    println!("search interaction: {:.3} ms", frame(&mut app));
    app.descending = !app.descending;
    println!("sort interaction: {:.3} ms", frame(&mut app));
}

#[test]
#[ignore = "manual performance measurement"]
fn dense_graph_latency() {
    use firmware_analysis_core::dependencies::{DependencyEdge, DependencyNode};
    let mut analysis = firmware_analysis_core::analyze_bytes(
        include_bytes!("../../../fixtures/build/cortex-m.elf"),
        "dense.elf",
        &Default::default(),
    )
    .unwrap();
    analysis.dependencies.nodes = (0..48)
        .map(|i| DependencyNode {
            id: format!("src/unit_{i:02}.c"),
            label: format!("src/unit_{i:02}.c"),
            objects: vec![],
            usage: Some(firmware_analysis_core::Usage {
                flash: 100 + i * 17,
                ram: i * 3,
            }),
            evidence: "Synthetic benchmark".into(),
        })
        .collect();
    analysis.dependencies.edges = (0..48)
        .flat_map(|i| {
            ((i + 1)..(i + 5).min(48)).map(move |j| DependencyEdge {
                from: format!("src/unit_{i:02}.c"),
                to: format!("src/unit_{j:02}.c"),
                symbols: vec![format!("symbol_{j}")],
            })
        })
        .collect();
    let mut app = Explorer {
        view: View::Dependencies,
        ..Default::default()
    };
    let ctx = egui::Context::default();
    let frame = |app: &mut Explorer| {
        let start = std::time::Instant::now();
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1200.0, 800.0),
                )),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| app.dependency_view(ui, &analysis));
            },
        );
        let pending = output.shapes.iter().any(|s| {
            matches!(&s.shape,
            egui::Shape::Text(t) if t.galley.text() == "Preparing dependency layout…")
        });
        (start.elapsed().as_secs_f64() * 1000.0, pending)
    };
    let settle = |app: &mut Explorer| {
        let start = std::time::Instant::now();
        while frame(app).1 {
            assert!(start.elapsed() < Duration::from_secs(60));
            std::thread::sleep(Duration::from_millis(1));
        }
    };
    println!(
        "48 nodes / {} edges, cold frame: {:.3} ms",
        analysis.dependencies.edges.len(),
        frame(&mut app).0
    );
    settle(&mut app);
    let mut times: Vec<_> = (0..30).map(|_| frame(&mut app).0).collect();
    times.sort_by(f64::total_cmp);
    println!(
        "graph unchanged frame median: {:.3} ms; p95: {:.3} ms",
        times[15], times[28]
    );
    app.search = "unit_20".into();
    println!("graph search interaction: {:.3} ms", frame(&mut app).0);
    settle(&mut app);
}
