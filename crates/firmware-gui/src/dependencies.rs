use super::{display::short_path, egui, Explorer};
use firmware_analysis_core::{dependencies::DependencyGraph, format_bytes, Analysis};
use std::collections::{BTreeMap, BTreeSet};

pub(super) struct GraphView {
    selected: Option<String>,
    edge: Option<(String, String)>,
    focused: bool,
    ram: bool,
    grouped: bool,
    zoom: f32,
    pan: egui::Vec2,
}
impl Default for GraphView {
    fn default() -> Self {
        Self {
            selected: None,
            edge: None,
            focused: false,
            ram: false,
            grouped: false,
            zoom: 1.0,
            pan: egui::Vec2::ZERO,
        }
    }
}

fn visible_nodes<'a>(
    graph: &'a DependencyGraph,
    state: &GraphView,
    search: &str,
) -> Vec<&'a firmware_analysis_core::dependencies::DependencyNode> {
    let mut neighbors = BTreeSet::new();
    if let Some(selected) = &state.selected {
        neighbors.insert(selected.as_str());
        for edge in &graph.edges {
            if &edge.from == selected {
                neighbors.insert(&edge.to);
            }
            if &edge.to == selected {
                neighbors.insert(&edge.from);
            }
        }
    }
    let search = search.to_lowercase();
    graph
        .nodes
        .iter()
        .filter(|node| {
            (!state.focused || state.selected.is_none() || neighbors.contains(node.id.as_str()))
                && (search.is_empty()
                    || node.label.to_lowercase().contains(&search)
                    || node
                        .objects
                        .iter()
                        .any(|o| o.to_lowercase().contains(&search)))
        })
        .collect()
}

/// Concentric rings keep nodes apart; directory columns offer a second, deterministic layout.
fn layout(
    nodes: &[&firmware_analysis_core::dependencies::DependencyNode],
    grouped: bool,
) -> (BTreeMap<String, egui::Pos2>, Vec<(String, egui::Pos2)>) {
    let mut positions = BTreeMap::new();
    let mut headings = vec![];
    if grouped {
        let mut groups: BTreeMap<String, Vec<_>> = BTreeMap::new();
        for node in nodes {
            let directory = node
                .label
                .rsplit_once('/')
                .map(|p| p.0)
                .unwrap_or("[no directory]");
            groups.entry(directory.into()).or_default().push(*node);
        }
        for (column, (directory, nodes)) in groups.into_iter().enumerate() {
            let x = column as f32 * 240.0;
            headings.push((directory, egui::pos2(x, -100.0)));
            for (row, node) in nodes.into_iter().enumerate() {
                positions.insert(node.id.clone(), egui::pos2(x, row as f32 * 155.0));
            }
        }
    } else {
        let mut index = 0;
        let mut radius: f32 = 230.0;
        while index < nodes.len() {
            let count = ((std::f32::consts::TAU * radius / 210.0) as usize)
                .min(nodes.len() - index)
                .max(1);
            for offset in 0..count {
                let angle = offset as f32 / count as f32 * std::f32::consts::TAU
                    - std::f32::consts::FRAC_PI_2;
                positions.insert(
                    nodes[index + offset].id.clone(),
                    egui::pos2(radius * angle.cos(), radius * angle.sin()),
                );
            }
            index += count;
            radius += 180.0;
        }
    }
    (positions, headings)
}

impl Explorer {
    pub(super) fn dependency_view(&mut self, ui: &mut egui::Ui, analysis: &Analysis) {
        let graph = &analysis.dependencies;
        ui.horizontal_wrapped(|ui| {
            ui.label("Compilation units");
            ui.checkbox(&mut self.graph_view.focused, "Focus on selected unit");
            ui.checkbox(&mut self.graph_view.grouped, "Group by directory");
            ui.selectable_value(&mut self.graph_view.ram, false, "Flash");
            ui.selectable_value(&mut self.graph_view.ram, true, "RAM");
            if ui.button("Clear selection").clicked() {
                self.graph_view.selected = None;
                self.graph_view.edge = None;
                self.graph_view.focused = false;
            }
            if ui.button("Fit graph").clicked() {
                self.graph_view.zoom = 1.0;
                self.graph_view.pan = egui::Vec2::ZERO;
            }
        });
        ui.small("Arrow: uses → defines · Node size: attributed symbol bytes · Drag to pan; scroll to zoom");
        if let Some(path) = &graph.map_path {
            ui.small(format!("Cross references: {path}"));
        }
        if graph.map_path.is_none() {
            ui.label("Connections unavailable. Build with -Wl,-Map,app.map,--cref,--no-demangle, then rescan. You can also select a map in Build files and use its cross references.");
        }
        ui.collapsing("Evidence and limitations", |ui| {
            for note in &graph.notes { ui.label(note); }
            ui.label("Sizes include uniquely attributed ELF symbol bytes only. Padding, unowned symbols and units removed by optimization are not assigned to source units. Object-only nodes have unknown size.");
        });
        let nodes = visible_nodes(graph, &self.graph_view, &self.search);
        self.visible_rows = nodes.len();
        if nodes.is_empty() {
            ui.weak("No compilation units match. Stripped firmware may have no unit ownership information.");
            return;
        }
        ui.horizontal_top(|ui| {
            let size = egui::vec2((ui.available_width() - 295.0).max(180.0), ui.available_height().max(200.0));
            self.graph_canvas(ui, graph, &nodes, size);
            ui.vertical(|ui| {
                ui.set_width(280.0);
                egui::ScrollArea::vertical().id_salt("graph_inspector").show(ui, |ui| {
                    if let Some((from, to)) = &self.graph_view.edge {
                        if let Some(edge) = graph.edges.iter().find(|e| &e.from == from && &e.to == to) {
                            ui.heading("Symbol references");
                            for id in [from, to] {
                                if let Some(node) = graph.nodes.iter().find(|n| &n.id == id) { ui.label(&node.label); }
                            }
                            ui.separator();
                            for symbol in &edge.symbols { ui.label(symbol); }
                        }
                    } else if let Some(id) = self.graph_view.selected.clone() {
                        if let Some(node) = graph.nodes.iter().find(|n| n.id == id) {
                            ui.heading("Selected unit");
                            ui.label(&node.label);
                            ui.small(&node.evidence);
                            if let Some(usage) = node.usage { ui.label(format!("Flash {} · RAM {}", format_bytes(usage.flash), format_bytes(usage.ram))); }
                            else { ui.label("Memory contribution unknown"); }
                            for object in &node.objects { ui.small(object); }
                            for (outgoing, title) in [(true, "Depends on"), (false, "Used by")] {
                                ui.separator(); ui.strong(title);
                                let edges: Vec<_> = graph.edges.iter().filter(|e| if outgoing { e.from == id } else { e.to == id }).collect();
                                if edges.is_empty() { ui.weak("No evidenced connections"); }
                                for edge in edges {
                                    let peer = if outgoing { &edge.to } else { &edge.from };
                                    if let Some(node) = graph.nodes.iter().find(|n| &n.id == peer) {
                                        if ui.button(format!("{} ({} symbols)", short_path(&node.label, []), edge.symbols.len())).on_hover_text(&node.label).clicked() {
                                            self.graph_view.edge = Some((edge.from.clone(), edge.to.clone()));
                                        }
                                    }
                                }
                            }

                        }
                    } else {
                        ui.heading("Dependency map");
                        ui.label(format!("{} units / objects\n{} dependency connections", graph.nodes.len(), graph.edges.len()));
                        ui.label("Select a node to inspect dependencies. Select an arrow to see the symbols connecting its units.");
                    }
                });
            });
        });
    }

    fn graph_canvas(
        &mut self,
        ui: &mut egui::Ui,
        graph: &DependencyGraph,
        nodes: &[&firmware_analysis_core::dependencies::DependencyNode],
        size: egui::Vec2,
    ) {
        let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click_and_drag());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 6.0, ui.visuals().extreme_bg_color);
        let (positions, headings) = layout(nodes, self.graph_view.grouped);
        let bounds = positions.values().fold(egui::Rect::NOTHING, |bounds, p| {
            bounds.union(egui::Rect::from_center_size(*p, egui::vec2(210.0, 150.0)))
        });
        let base_scale = (rect.width() / bounds.width())
            .min(rect.height() / (bounds.height() + 100.0))
            .min(1.0);
        if response.dragged() {
            self.graph_view.pan += response.drag_delta();
        }
        if response.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0 {
                let old_zoom = self.graph_view.zoom;
                self.graph_view.zoom = (old_zoom * (scroll * 0.002).exp()).clamp(0.2, 8.0);
                if let Some(pointer) = response.hover_pos() {
                    let offset = pointer - rect.center();
                    self.graph_view.pan =
                        offset - (offset - self.graph_view.pan) * (self.graph_view.zoom / old_zoom);
                }
            }
        }
        let scale = base_scale * self.graph_view.zoom;
        let screen =
            |p: egui::Pos2| rect.center() + (p - bounds.center()) * scale + self.graph_view.pan;
        let maximum = nodes
            .iter()
            .filter_map(|n| {
                n.usage
                    .map(|u| if self.graph_view.ram { u.ram } else { u.flash })
            })
            .max()
            .unwrap_or(0)
            .max(1) as f32;
        let radii: BTreeMap<_, _> = nodes
            .iter()
            .map(|node| {
                let bytes = node
                    .usage
                    .map(|u| if self.graph_view.ram { u.ram } else { u.flash })
                    .unwrap_or(0);
                (
                    node.id.as_str(),
                    (24.0 + 23.0 * (bytes as f32 / maximum).sqrt()) * scale,
                )
            })
            .collect();
        let pointer = response.hover_pos();
        let mut hit_edge = None;
        let mut distance = 8.0;
        for edge in &graph.edges {
            let (Some(from), Some(to)) = (positions.get(&edge.from), positions.get(&edge.to))
            else {
                continue;
            };
            let from = screen(*from);
            let to = screen(*to);
            let direction = (to - from).normalized();
            let start = from + direction * radii[edge.from.as_str()];
            let end = to - direction * radii[edge.to.as_str()];
            // Offset reciprocal edges so each arrow remains individually selectable.
            let normal = egui::vec2(-direction.y, direction.x) * 5.0;
            let start = start + normal;
            let end = end + normal;
            let highlighted = self
                .graph_view
                .edge
                .as_ref()
                .is_some_and(|(f, t)| f == &edge.from && t == &edge.to)
                || self
                    .graph_view
                    .selected
                    .as_ref()
                    .is_some_and(|id| id == &edge.from || id == &edge.to);
            let color = if highlighted {
                ui.visuals().selection.stroke.color
            } else {
                ui.visuals().weak_text_color().gamma_multiply(0.6)
            };
            painter.arrow(
                start,
                end - start,
                egui::Stroke::new(if highlighted { 2.0_f32 } else { 1.0_f32 }, color),
            );
            if let Some(pointer) = pointer {
                let delta = end - start;
                let t = ((pointer - start).dot(delta) / delta.length_sq().max(1.0)).clamp(0.0, 1.0);
                let d = pointer.distance(start + delta * t);
                if d < distance {
                    distance = d;
                    hit_edge = Some(edge);
                }
            }
        }
        let mut hit_node = None;
        for node in nodes {
            let center = screen(positions[&node.id]);
            let radius = radii[node.id.as_str()];
            if !rect.intersects(egui::Rect::from_center_size(
                center,
                egui::vec2(radius * 2.0 + 160.0, radius * 2.0 + 60.0),
            )) {
                continue;
            }
            let selected = self.graph_view.selected.as_ref() == Some(&node.id);
            let color = if selected {
                ui.visuals().selection.bg_fill
            } else if node.usage.is_none() {
                egui::Color32::from_rgb(110, 85, 50)
            } else {
                egui::Color32::from_rgb(40, 100, 140)
            };
            painter.circle(
                center,
                radius.max(3.0),
                color,
                egui::Stroke::new(
                    if selected { 2.0_f32 } else { 1.0_f32 },
                    ui.visuals().text_color(),
                ),
            );
            let label = short_path(&node.label, nodes.iter().map(|n| n.label.as_str()));
            let label = if label.chars().count() > 32 {
                format!(
                    "…{}",
                    label
                        .chars()
                        .rev()
                        .take(31)
                        .collect::<String>()
                        .chars()
                        .rev()
                        .collect::<String>()
                )
            } else {
                label
            };
            let label_rect = egui::Rect::from_center_size(
                center + egui::vec2(0.0, radius + 14.0),
                egui::vec2(200.0 * scale.clamp(0.6, 1.0), 28.0),
            );
            painter.text(
                label_rect.center(),
                egui::Align2::CENTER_CENTER,
                label,
                egui::FontId::proportional((12.0 * scale).clamp(9.0, 16.0)),
                ui.visuals().text_color(),
            );
            if pointer
                .is_some_and(|p| p.distance(center) <= radius.max(8.0) || label_rect.contains(p))
            {
                hit_node = Some(*node);
            }
        }
        for (label, pos) in headings {
            painter.text(
                screen(pos),
                egui::Align2::CENTER_CENTER,
                short_path(&label, []),
                egui::FontId::proportional(14.0),
                ui.visuals().text_color(),
            );
        }
        if let Some(node) = hit_node {
            response.clone().on_hover_text(format!(
                "{}\n{}\n{}",
                node.label,
                node.evidence,
                node.usage
                    .map(|u| format!(
                        "Flash {} · RAM {}",
                        format_bytes(u.flash),
                        format_bytes(u.ram)
                    ))
                    .unwrap_or_else(|| "Memory contribution unknown".into())
            ));
            if response.clicked() {
                self.graph_view.selected = Some(node.id.clone());
                self.graph_view.edge = None;
            }
        } else if let Some(edge) = hit_edge {
            response.clone().on_hover_text(format!(
                "{} symbol references\n{}",
                edge.symbols.len(),
                edge.symbols.join("\n")
            ));
            if response.clicked() {
                self.graph_view.edge = Some((edge.from.clone(), edge.to.clone()));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use firmware_analysis_core::dependencies::{DependencyEdge, DependencyNode};

    fn graph() -> DependencyGraph {
        DependencyGraph {
            nodes: ["app/main.c", "drivers/spi.c", "unused/main.c"]
                .into_iter()
                .map(|label| DependencyNode {
                    id: label.into(),
                    label: label.into(),
                    evidence: "DWARF compilation unit".into(),
                    usage: Some(Default::default()),
                    objects: vec![],
                })
                .collect(),
            edges: vec![DependencyEdge {
                from: "app/main.c".into(),
                to: "drivers/spi.c".into(),
                symbols: vec!["spi_transfer".into()],
            }],
            map_path: Some("app.map".into()),
            notes: vec![],
        }
    }

    #[test]
    fn focus_includes_incoming_and_outgoing_units_and_search_preserves_identity() {
        let graph = graph();
        let state = GraphView {
            selected: Some("drivers/spi.c".into()),
            focused: true,
            ..Default::default()
        };
        let visible = visible_nodes(&graph, &state, "");
        assert_eq!(visible.len(), 2);
        assert!(!visible.iter().any(|n| n.id == "unused/main.c"));
        assert_eq!(visible_nodes(&graph, &state, "main.c")[0].id, "app/main.c");
        assert!(visible_nodes(&graph, &state, "no match").is_empty());
        for grouped in [false, true] {
            let (positions, _) = layout(&visible_nodes(&graph, &GraphView::default(), ""), grouped);
            assert_eq!(positions.len(), 3);
            assert_ne!(positions["app/main.c"], positions["unused/main.c"]);
            assert!(positions
                .values()
                .all(|p| p.x.is_finite() && p.y.is_finite()));
        }
    }

    #[test]
    fn graph_nodes_arrows_and_zoom_respond_to_pointer_input() {
        let mut analysis = firmware_analysis_core::analyze_bytes(
            include_bytes!("../../../fixtures/cortex-m.elf"),
            "test.elf",
            &Default::default(),
        )
        .unwrap();
        analysis.dependencies = graph();
        let mut app = Explorer::default();
        let ctx = egui::Context::default();
        super::super::shell::configure_style(&ctx);
        let frame = |app: &mut Explorer, events| {
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1100.0, 700.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default()
                        .show(ctx, |ui| app.dependency_view(ui, &analysis));
                },
            )
        };
        let output = frame(&mut app, vec![]);
        let circles: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Circle(circle)
                    if circle.fill == egui::Color32::from_rgb(40, 100, 140) =>
                {
                    Some(circle.center)
                }
                _ => None,
            })
            .collect();
        assert_eq!(circles.len(), 3);
        let click = |app: &mut Explorer, position| {
            for pressed in [true, false] {
                frame(
                    app,
                    vec![
                        egui::Event::PointerMoved(position),
                        egui::Event::PointerButton {
                            pos: position,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                );
            }
        };
        click(&mut app, circles[0]);
        assert_eq!(app.graph_view.selected.as_deref(), Some("app/main.c"));
        let direction = (circles[1] - circles[0]).normalized();
        let arrow = circles[0].lerp(circles[1], 0.5) + egui::vec2(-direction.y, direction.x) * 5.0;
        click(&mut app, arrow);
        assert_eq!(
            app.graph_view.edge,
            Some(("app/main.c".into(), "drivers/spi.c".into()))
        );
        let output = frame(&mut app, vec![]);
        assert!(output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == "spi_transfer")));
        frame(
            &mut app,
            vec![
                egui::Event::PointerMoved(circles[0]),
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(0.0, 120.0),
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        assert!(app.graph_view.zoom > 1.0);
        app.graph_view.focused = true;
        frame(&mut app, vec![]);
        assert_eq!(app.visible_rows, 2);
        app.search = "no match".into();
        frame(&mut app, vec![]);
        assert_eq!(app.visible_rows, 0);
    }
}
