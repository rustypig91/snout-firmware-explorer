use super::{display::short_path, egui, Explorer};
use firmware_analysis_core::{dependencies::DependencyGraph, Analysis};
use std::collections::{BTreeMap, BTreeSet};

mod graph_layout;
use graph_layout::{CachedLayout, LayoutInput, NodeSpec};

#[cfg(test)]
pub(super) fn settle_graph(
    app: &mut Explorer,
    mut frame: impl FnMut(&mut Explorer) -> egui::FullOutput,
) -> egui::FullOutput {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let output = frame(app);
        if app.visible_rows == 0 || app.graph_view.layout_ready() {
            return output;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "graph layout did not finish"
        );
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

pub(super) struct GraphView {
    selected: Option<String>,
    edge: Option<(String, String)>,
    focused: bool,
    ram: bool,
    grouped: bool,
    zoom: f32,
    pan: egui::Vec2,
    readable_size: bool,
    layout: Option<CachedLayout>,
    layout_revision: u64,
    requested_layout: Option<LayoutInput>,
    layout_job: Option<std::sync::mpsc::Receiver<(u64, CachedLayout)>>,
    filter: Option<(String, Option<String>)>,
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
            readable_size: false,
            layout: None,
            layout_revision: 0,
            requested_layout: None,
            layout_job: None,
            filter: None,
        }
    }
}

impl GraphView {
    // At most one layout runs at a time. While it runs, requests coalesce to
    // the latest controls; obsolete geometry never reaches the canvas.
    fn request_layout(&mut self, input: LayoutInput, ctx: &egui::Context) -> bool {
        if self.requested_layout.as_ref() != Some(&input) {
            self.layout_revision = self.layout_revision.wrapping_add(1);
            self.requested_layout = Some(input);
        }
        let mut changed = false;
        if let Some(job) = &self.layout_job {
            match job.try_recv() {
                Ok((revision, cached)) => {
                    self.layout_job = None;
                    if revision == self.layout_revision {
                        self.layout = Some(cached);
                        changed = true;
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.layout_job = None;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
        }
        if !self.layout_ready() && self.layout_job.is_none() {
            let input = self.requested_layout.as_ref().unwrap().clone();
            let revision = self.layout_revision;
            let (sender, receiver) = std::sync::mpsc::channel();
            self.layout_job = Some(receiver);
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                let geometry = graph_layout::compute(&input);
                let _ = sender.send((revision, CachedLayout { input, geometry }));
                ctx.request_repaint();
            });
        }
        changed
    }

    fn layout_ready(&self) -> bool {
        self.layout
            .as_ref()
            .is_some_and(|cached| Some(&cached.input) == self.requested_layout.as_ref())
    }

    fn fit_graph(&mut self) {
        self.zoom = 1.0;
        self.pan = egui::Vec2::ZERO;
        self.readable_size = false;
    }

    fn select_edge(&mut self, from: &str, to: &str) {
        self.selected = None;
        self.edge = Some((from.into(), to.into()));
    }
}

fn node_directory(label: &str) -> String {
    let normalized = label.replace('\\', "/");
    normalized
        .rsplit_once('/')
        .map(|(directory, _)| directory)
        .unwrap_or("[no directory]")
        .into()
}

fn scroll_zoom(zoom: f32, scroll: f32, fit_scale: f32) -> f32 {
    // The upper limit is an absolute rendering scale, not a multiple of fit.
    (zoom * (scroll * 0.002).exp()).clamp(0.2, 8.0 / fit_scale)
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
    let candidates: Vec<_> = graph
        .nodes
        .iter()
        .filter(|node| {
            !state.focused || state.selected.is_none() || neighbors.contains(node.id.as_str())
        })
        .collect();
    if search.is_empty() {
        return candidates;
    }
    let search = search.to_lowercase();
    let matches: BTreeSet<_> = candidates
        .iter()
        .filter(|node| {
            node.label.to_lowercase().contains(&search)
                || node
                    .objects
                    .iter()
                    .any(|o| o.to_lowercase().contains(&search))
        })
        .map(|node| node.id.as_str())
        .collect();
    let mut connected = matches.clone();
    // Expand only the original matches, so a filter does not pull in an entire
    // connected component through neighbors of neighbors.
    for edge in &graph.edges {
        if matches.contains(edge.from.as_str()) {
            connected.insert(edge.to.as_str());
        }
        if matches.contains(edge.to.as_str()) {
            connected.insert(edge.from.as_str());
        }
    }
    candidates
        .into_iter()
        .filter(|node| connected.contains(node.id.as_str()))
        .collect()
}

fn arrow_head(points: &[egui::Pos2], scale: f32, target: egui::Rect) -> Option<[egui::Pos2; 3]> {
    let tip = *points.last()?;
    let previous = points.iter().rev().find(|p| p.distance(tip) > 0.01)?;
    let mut direction = (tip - *previous).normalized();
    // Obstacle detours can approach a box almost tangentially. In that case
    // one wing would lie inside the box and be hidden when cards are painted.
    let inward = [
        ((tip.x - target.left()).abs(), egui::Vec2::X),
        ((tip.x - target.right()).abs(), -egui::Vec2::X),
        ((tip.y - target.top()).abs(), egui::Vec2::Y),
        ((tip.y - target.bottom()).abs(), -egui::Vec2::Y),
    ]
    .into_iter()
    .min_by(|a, b| a.0.total_cmp(&b.0))?
    .1;
    if direction.dot(inward) < 0.5 {
        direction = inward;
    }
    let normal = egui::vec2(-direction.y, direction.x);
    let length = (10.0 * scale).clamp(7.0, 16.0);
    Some([
        tip,
        tip - direction * length + normal * length * 0.5,
        tip - direction * length - normal * length * 0.5,
    ])
}

fn curve_distance(points: &[egui::Pos2], pointer: egui::Pos2) -> f32 {
    points
        .windows(2)
        .map(|segment| {
            let delta = segment[1] - segment[0];
            let t =
                ((pointer - segment[0]).dot(delta) / delta.length_sq().max(0.001)).clamp(0.0, 1.0);
            pointer.distance(segment[0] + delta * t)
        })
        .fold(f32::INFINITY, f32::min)
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
                self.graph_view.fit_graph();
            }
            if ui.button("Readable size").clicked() {
                self.graph_view.readable_size = true;
            }
        });
        ui.small("Arrows point to dependencies (uses → defines) · Box area: visible min–max Flash / RAM scaled from 1× to 25× · Drag to pan; scroll to zoom");
        if let Some(path) = &graph.map_path {
            ui.small(format!("Cross references: {path}"));
        }
        if graph.map_path.is_none() {
            ui.group(|ui| {
                ui.colored_label(egui::Color32::YELLOW, "Warning: linker cross references are unavailable. File dependencies cannot be shown.");
                ui.label("The file boxes do not mean these files are independent.");
                if let Some(hint) = &graph.connection_hint {
                    ui.label(hint);
                } else {
                    ui.label("No linker cross-reference map is loaded. Select a GNU ld or LLVM lld ELF map from the same build in Build files and use its cross references.");
                }
                if let Some(flags) = &graph.cross_reference_flags {
                    ui.monospace(flags);
                    ui.label("Rebuild and rescan, then load the updated map.");
                }
            });
        } else if graph.edges.is_empty() {
            ui.label("Cross references were loaded, but no connections between distinct files were found.");
        }
        ui.collapsing("Evidence and limitations", |ui| {
            for note in &graph.notes { ui.label(note); }
            ui.label("Sizes include uniquely attributed ELF symbol bytes only. Padding, unowned symbols and units removed by optimization are not assigned to source units. Object-only nodes have unknown size.");
        });
        let nodes = visible_nodes(graph, &self.graph_view, &self.search);
        let filter = (
            self.search.clone(),
            if self.graph_view.focused {
                self.graph_view.selected.clone()
            } else {
                None
            },
        );
        let filter_changed = self.graph_view.filter.as_ref() != Some(&filter);
        self.graph_view.filter = Some(filter);
        self.visible_rows = nodes.len();
        if nodes.is_empty() {
            ui.weak("No compilation units match. Stripped firmware may have no unit ownership information.");
            return;
        }
        ui.horizontal_top(|ui| {
            let size = egui::vec2((ui.available_width() - 295.0).max(180.0), ui.available_height().max(200.0));
            self.graph_canvas(ui, graph, &nodes, size, filter_changed);
            ui.vertical(|ui| {
                ui.set_width(280.0);
                egui::ScrollArea::vertical().id_salt("graph_inspector").show(ui, |ui| {
                    if let Some((from, to)) = &self.graph_view.edge {
                        if let Some(edge) = graph.edges.iter().find(|e| &e.from == from && &e.to == to) {
                            ui.heading("Symbol references");
                            for (index, id) in [from, to].into_iter().enumerate() {
                                if index == 1 { ui.small("uses symbols defined by ↓"); }
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
                            if let Some(usage) = node.usage { ui.label(format!("Flash {} · RAM {}", self.snapshot_bytes("dependency", &node.id, "flash", usage.flash), self.snapshot_bytes("dependency", &node.id, "ram", usage.ram))); }
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
                                            self.graph_view.select_edge(&edge.from, &edge.to);
                                        }
                                    }
                                }
                            }

                        }
                    } else {
                        ui.heading("Dependency map");
                        ui.label(format!("{} units / objects\n{} dependency connections", self.snapshot_count("counts", "", "dependency_nodes", graph.nodes.len() as u64), self.snapshot_count("counts", "", "dependency_edges", graph.edges.len() as u64)));
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
        filter_changed: bool,
    ) {
        let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click_and_drag());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 6.0, ui.visuals().extreme_bg_color);
        let (smallest, largest) = nodes
            .iter()
            .filter_map(|node| {
                node.usage
                    .map(|u| if self.graph_view.ram { u.ram } else { u.flash })
            })
            .fold((u64::MAX, 0), |(smallest, largest), bytes| {
                (smallest.min(bytes), largest.max(bytes))
            });
        let short_labels = super::display::short_paths(
            &nodes.iter().map(|n| n.label.as_str()).collect::<Vec<_>>(),
        );
        let texts: BTreeMap<_, _> = nodes
            .iter()
            .zip(short_labels)
            .map(|(node, label)| {
                let label = if label.chars().count() > 30 {
                    format!(
                        "…{}",
                        label
                            .chars()
                            .skip(label.chars().count() - 29)
                            .collect::<String>()
                    )
                } else {
                    label
                };
                let bytes = node
                    .usage
                    .map(|u| if self.graph_view.ram { u.ram } else { u.flash });
                let text = format!(
                    "{}\n{} {}",
                    label,
                    if self.graph_view.ram { "RAM" } else { "Flash" },
                    bytes
                        .map(|value| self.snapshot_bytes(
                            "dependency",
                            &node.id,
                            if self.graph_view.ram { "ram" } else { "flash" },
                            value
                        ))
                        .unwrap_or_else(|| "unknown".into())
                );
                (
                    node.id.as_str(),
                    painter.layout_no_wrap(
                        text,
                        egui::FontId::proportional(13.0),
                        ui.visuals().text_color(),
                    ),
                )
            })
            .collect();
        // Share a text-sized baseline so box area compares bytes consistently,
        // rather than also growing with filename length. No fixed width floor.
        let minimum = texts
            .values()
            .fold(egui::Vec2::ZERO, |size, text| size.max(text.size()))
            + egui::vec2(20.0, 12.0);
        let input = LayoutInput {
            nodes: nodes
                .iter()
                .map(|node| {
                    let bytes = node
                        .usage
                        .map(|u| if self.graph_view.ram { u.ram } else { u.flash })
                        .unwrap_or(0);
                    NodeSpec {
                        id: node.id.clone(),
                        directory: node_directory(&node.label),
                        size: graph_layout::card_size(minimum, bytes, smallest, largest),
                    }
                })
                .collect(),
            edges: graph
                .edges
                .iter()
                .filter(|e| {
                    texts.contains_key(e.from.as_str()) && texts.contains_key(e.to.as_str())
                })
                .map(|e| (e.from.clone(), e.to.clone()))
                .collect(),
            grouped: self.graph_view.grouped,
            ram: self.graph_view.ram,
            // Tall canvases read better as a DOT-style hierarchy; wide canvases
            // have enough room for horizontal mindmap branches.
            vertical: rect.width() < rect.height() * 1.8,
        };
        let layout_changed = self.graph_view.request_layout(input, ui.ctx());
        if !self.graph_view.layout_ready() {
            // Poll independently of the worker's one-shot repaint notification.
            // This also covers completion while egui is finishing a frame.
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(50));
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "Preparing dependency layout…",
                egui::FontId::proportional(14.0),
                ui.visuals().text_color(),
            );
            return;
        }
        if layout_changed || filter_changed {
            self.graph_view.zoom = 1.0;
            self.graph_view.pan = egui::Vec2::ZERO;
        }
        let bounds = self.graph_view.layout.as_ref().unwrap().geometry.bounds;
        let base_scale = (rect.width() / bounds.width())
            .min(rect.height() / bounds.height())
            .min(1.0);
        if self.graph_view.readable_size {
            self.graph_view.zoom = 1.0 / base_scale;
            self.graph_view.pan = self
                .graph_view
                .selected
                .as_ref()
                .and_then(|id| {
                    self.graph_view
                        .layout
                        .as_ref()
                        .unwrap()
                        .geometry
                        .cards
                        .get(id)
                })
                .map(|card| bounds.center() - card.center())
                .unwrap_or(egui::Vec2::ZERO);
            self.graph_view.readable_size = false;
        }
        if response.dragged() {
            self.graph_view.pan += response.drag_delta();
        }
        // A newly fitted graph should not inherit smooth-scroll inertia from
        // the previous layout when a filter or memory mode changes.
        if response.hovered() && !layout_changed && !filter_changed {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0 {
                let old_zoom = self.graph_view.zoom;
                self.graph_view.zoom = scroll_zoom(old_zoom, scroll, base_scale);
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
        let geometry = &self.graph_view.layout.as_ref().unwrap().geometry;
        let labels: BTreeMap<_, _> = nodes
            .iter()
            .map(|node| {
                let world_card = geometry.cards[&node.id];
                let card = egui::Rect::from_min_max(screen(world_card.min), screen(world_card.max));
                let galley = painter.layout_no_wrap(
                    texts[node.id.as_str()].text().into(),
                    egui::FontId::proportional((13.0 * scale).max(1.0)),
                    ui.visuals().text_color(),
                );
                (
                    node.id.as_str(),
                    (card, card.center() - galley.size() / 2.0, galley),
                )
            })
            .collect();
        let pointer = response.hover_pos();
        let mut hit_edge = None;
        let mut distance = 8.0;
        for route in &geometry.edges {
            let points: Vec<_> = route.points.iter().map(|p| screen(*p)).collect();
            let Some(head) = arrow_head(&points, scale, labels[route.to.as_str()].0) else {
                continue;
            };
            let edge = route;
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
                ui.visuals().text_color().gamma_multiply(0.7)
            };
            painter.add(egui::Shape::line(
                points.clone(),
                egui::Stroke::new(if highlighted { 2.5_f32 } else { 1.5_f32 }, color),
            ));
            painter.add(egui::Shape::convex_polygon(
                head.to_vec(),
                color,
                egui::Stroke::NONE,
            ));
            if let Some(pointer) = pointer {
                let d = curve_distance(&points, pointer);
                if d < distance {
                    distance = d;
                    hit_edge = Some(edge);
                }
            }
        }
        let mut hit_node = None;
        for node in nodes {
            let card = labels[node.id.as_str()].0;
            if !rect.intersects(card) {
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
            painter.rect_filled(card, (7.0 * scale).min(12.0), color);
            painter.rect_stroke(
                card,
                (7.0 * scale).min(12.0),
                egui::Stroke::new(
                    if selected { 2.0_f32 } else { 1.0_f32 },
                    ui.visuals().text_color().gamma_multiply(0.6),
                ),
            );
            if pointer.is_some_and(|p| card.contains(p)) {
                hit_node = Some(*node);
            }
        }
        for (_, position, galley) in labels.values() {
            painter.galley(*position, galley.clone(), ui.visuals().text_color());
        }

        for (label, pos) in &geometry.headings {
            painter.text(
                screen(*pos),
                egui::Align2::CENTER_CENTER,
                label,
                egui::FontId::proportional((14.0 * scale).max(1.0)),
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
                        self.snapshot_bytes("dependency", &node.id, "flash", u.flash),
                        self.snapshot_bytes("dependency", &node.id, "ram", u.ram)
                    ))
                    .unwrap_or_else(|| "Memory contribution unknown".into())
            ));
            if response.clicked() {
                self.graph_view.selected = Some(node.id.clone());
                self.graph_view.edge = None;
            }
        } else if let Some(route) = hit_edge {
            let Some(edge) = graph
                .edges
                .iter()
                .find(|edge| edge.from == route.from && edge.to == route.to)
            else {
                return;
            };
            response.clone().on_hover_text(format!(
                "{} symbol references\n{}",
                edge.symbols.len(),
                edge.symbols.join("\n")
            ));
            if response.clicked() {
                self.graph_view.select_edge(&edge.from, &edge.to);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use firmware_analysis_core::dependencies::{DependencyEdge, DependencyNode};

    #[test]
    fn fit_cancels_readable_size_waiting_for_layout() {
        let mut state = GraphView {
            readable_size: true,
            zoom: 4.0,
            pan: egui::vec2(100.0, -50.0),
            ..GraphView::default()
        };
        assert!(!state.layout_ready());
        state.fit_graph();
        assert!(!state.readable_size);
        assert_eq!(state.zoom, 1.0);
        assert_eq!(state.pan, egui::Vec2::ZERO);
    }

    #[test]
    fn zoom_reaches_readable_scale_even_for_very_large_graphs() {
        for fit_scale in [1.0, 0.1, 0.001] {
            let mut zoom = 1.0;
            for _ in 0..100 {
                zoom = scroll_zoom(zoom, 120.0, fit_scale);
            }
            assert!((zoom * fit_scale - 8.0).abs() < 0.001);
            assert_eq!(scroll_zoom(zoom, -100_000.0, fit_scale), 0.2);
        }
    }

    #[test]
    fn changing_filters_coalesces_requests_and_discards_obsolete_geometry() {
        let input = |name: &str| LayoutInput {
            nodes: vec![NodeSpec {
                id: name.into(),
                directory: "src".into(),
                size: egui::vec2(100.0, 40.0),
            }],
            edges: vec![],
            grouped: false,
            ram: false,
            vertical: false,
        };
        let ctx = egui::Context::default();
        let (sender, receiver) = std::sync::mpsc::channel();
        let old = input("old");
        let mut state = GraphView {
            requested_layout: Some(old.clone()),
            layout_revision: 1,
            layout_job: Some(receiver),
            ..Default::default()
        };
        // The injected worker is deliberately held until after two changes.
        assert!(!state.request_layout(input("intermediate"), &ctx));
        assert!(!state.request_layout(input("latest"), &ctx));
        assert_eq!(state.layout_revision, 3);
        assert!(state.layout.is_none());
        sender
            .send((
                1,
                CachedLayout {
                    geometry: graph_layout::compute(&old),
                    input: old,
                },
            ))
            .unwrap();
        assert!(!state.request_layout(input("latest"), &ctx));
        assert!(state.layout.is_none(), "obsolete results must not appear");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !state.layout_ready() {
            state.request_layout(input("latest"), &ctx);
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        let geometry = &state.layout.as_ref().unwrap().geometry;
        assert!(geometry.cards.contains_key("latest"));
        assert!(!geometry.cards.contains_key("old"));
        assert!(!geometry.cards.contains_key("intermediate"));
    }

    #[test]
    fn directory_grouping_accepts_debug_paths_from_either_platform() {
        assert_eq!(
            node_directory(r"C:\project\drivers\spi.c"),
            "C:/project/drivers"
        );
        assert_eq!(
            node_directory("C:/project/drivers/spi.c"),
            "C:/project/drivers"
        );
        assert_eq!(node_directory(r"drivers\spi.c"), "drivers");
        assert_eq!(node_directory("spi.c"), "[no directory]");
    }

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
            ..Default::default()
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
        let filtered = visible_nodes(&graph, &state, "main.c");
        assert_eq!(filtered.len(), 2);
        assert_eq!(filtered[0].id, "app/main.c");
        assert!(visible_nodes(&graph, &state, "no match").is_empty());
    }

    #[test]
    fn search_includes_direct_dependencies_and_dependents_without_transitive_expansion() {
        let mut graph = graph();
        graph.nodes[1].objects.push("build/drivers/spi.c.o".into());
        graph.edges.push(DependencyEdge {
            from: "drivers/spi.c".into(),
            to: "unused/main.c".into(),
            symbols: vec!["config".into()],
        });
        let state = GraphView::default();
        let ids = |state: &GraphView, query| {
            visible_nodes(&graph, state, query)
                .iter()
                .map(|node| node.id.as_str())
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(&state, "APP/MAIN.C"), ["app/main.c", "drivers/spi.c"]);
        assert_eq!(ids(&state, "unused"), ["drivers/spi.c", "unused/main.c"]);
        assert_eq!(
            ids(&state, "spi.c.o"),
            ["app/main.c", "drivers/spi.c", "unused/main.c"]
        );
        assert_eq!(
            ids(&state, "main.c"),
            ["app/main.c", "drivers/spi.c", "unused/main.c"]
        );
        assert!(ids(&state, "no match").is_empty());
        let focused = GraphView {
            focused: true,
            selected: Some("app/main.c".into()),
            ..Default::default()
        };
        assert_eq!(ids(&focused, "spi.c"), ["app/main.c", "drivers/spi.c"]);
        assert!(ids(&focused, "unused").is_empty());
    }

    #[test]
    fn filter_changes_refit_even_when_the_visible_graph_is_unchanged() {
        let mut analysis = firmware_analysis_core::analyze_bytes(
            include_bytes!("../../../fixtures/build/cortex-m.elf"),
            "test.elf",
            &Default::default(),
        )
        .unwrap();
        analysis.dependencies = graph();
        analysis.dependencies.nodes.truncate(2);
        let mut app = Explorer::default();
        let ctx = egui::Context::default();
        let frame = |app: &mut Explorer| {
            settle_graph(app, |app| {
                ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(1100.0, 700.0),
                        )),
                        ..Default::default()
                    },
                    |ctx| {
                        egui::CentralPanel::default()
                            .show(ctx, |ui| app.dependency_view(ui, &analysis));
                    },
                )
            })
        };
        let _ = frame(&mut app);
        let cached_points = app.graph_view.layout.as_ref().unwrap().geometry.edges[0]
            .points
            .as_ptr();
        for search in ["spi.c", "main.c", ""] {
            app.graph_view.zoom = 2.0;
            app.graph_view.pan = egui::vec2(150.0, 80.0);
            app.search = search.into();
            let _ = frame(&mut app);
            assert_eq!(app.visible_rows, 2);
            assert_eq!(app.graph_view.zoom, 1.0);
            assert_eq!(app.graph_view.pan, egui::Vec2::ZERO);
            assert_eq!(
                app.graph_view.layout.as_ref().unwrap().geometry.edges[0]
                    .points
                    .as_ptr(),
                cached_points
            );
        }
        app.graph_view.zoom = 2.0;
        app.graph_view.pan = egui::vec2(150.0, 80.0);
        let _ = frame(&mut app);
        assert_eq!(app.graph_view.zoom, 2.0);
        assert_eq!(app.graph_view.pan, egui::vec2(150.0, 80.0));
        app.graph_view.focused = true;
        for selected in ["app/main.c", "drivers/spi.c"] {
            app.graph_view.zoom = 2.0;
            app.graph_view.pan = egui::vec2(150.0, 80.0);
            app.graph_view.selected = Some(selected.into());
            let _ = frame(&mut app);
            assert_eq!(app.visible_rows, 2);
            assert_eq!(app.graph_view.zoom, 1.0);
            assert_eq!(app.graph_view.pan, egui::Vec2::ZERO);
        }
        app.graph_view.zoom = 2.0;
        app.search = "no match".into();
        let _ = frame(&mut app);
        assert_eq!(app.visible_rows, 0);
        app.search.clear();
        let _ = frame(&mut app);
        assert_eq!(app.visible_rows, 2);
        assert_eq!(app.graph_view.zoom, 1.0);
    }

    #[test]
    fn graph_nodes_arrows_and_zoom_respond_to_pointer_input() {
        let mut analysis = firmware_analysis_core::analyze_bytes(
            include_bytes!("../../../fixtures/build/cortex-m.elf"),
            "test.elf",
            &Default::default(),
        )
        .unwrap();
        analysis.dependencies = graph();
        analysis.dependencies.nodes[0].usage =
            Some(firmware_analysis_core::Usage { flash: 400, ram: 4 });
        analysis.dependencies.nodes[1].usage =
            Some(firmware_analysis_core::Usage { flash: 4, ram: 400 });
        let mut app = Explorer::default();
        let ctx = egui::Context::default();
        super::super::shell::configure_style(&ctx);
        let frame = |app: &mut Explorer, mut events| {
            settle_graph(app, |app| {
                ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(1100.0, 700.0),
                        )),
                        events: std::mem::take(&mut events),
                        ..Default::default()
                    },
                    |ctx| {
                        egui::CentralPanel::default()
                            .show(ctx, |ui| app.dependency_view(ui, &analysis));
                    },
                )
            })
        };
        let output = frame(&mut app, vec![]);
        let cards: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Rect(card) if card.fill == egui::Color32::from_rgb(40, 100, 140) => {
                    Some(card.rect.center())
                }
                _ => None,
            })
            .collect();
        assert_eq!(cards.len(), 3);
        let cached_points = app.graph_view.layout.as_ref().unwrap().geometry.edges[0]
            .points
            .as_ptr();
        let flash_cards = &app.graph_view.layout.as_ref().unwrap().geometry.cards;
        assert!(flash_cards["app/main.c"].area() > flash_cards["drivers/spi.c"].area());
        let last_card = output
            .shapes
            .iter()
            .rposition(|shape| matches!(shape.shape, egui::Shape::Rect(_)))
            .unwrap();
        for label in ["app/main.c", "drivers/spi.c", "unused/main.c"] {
            let index = output.shapes.iter().position(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text().starts_with(label))).unwrap();
            assert!(index > last_card);
        }

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
        click(&mut app, cards[0]);
        assert_eq!(app.graph_view.selected.as_deref(), Some("app/main.c"));
        let selected_output = frame(&mut app, vec![]);
        let connection = selected_output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == "drivers/spi.c (1 symbols)" => {
                    Some(text.pos + text.galley.size() / 2.0)
                }
                _ => None,
            })
            .unwrap();
        click(&mut app, connection);
        assert!(app.graph_view.selected.is_none());
        assert_eq!(
            app.graph_view.edge,
            Some(("app/main.c".into(), "drivers/spi.c".into()))
        );
        click(&mut app, cards[0]);
        assert_eq!(app.graph_view.selected.as_deref(), Some("app/main.c"));
        assert!(app.graph_view.edge.is_none());
        let arrow = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Path(path) if !path.closed && path.points.len() > 3 => {
                    Some(path.points[path.points.len() / 2])
                }
                _ => None,
            })
            .unwrap();
        click(&mut app, arrow);
        assert!(app.graph_view.selected.is_none());
        assert_eq!(
            app.graph_view.edge,
            Some(("app/main.c".into(), "drivers/spi.c".into()))
        );
        let output = frame(&mut app, vec![]);
        assert_eq!(
            output
                .shapes
                .iter()
                .filter(|shape| matches!(&shape.shape,
                    egui::Shape::Rect(card) if card.fill == egui::Color32::from_rgb(40, 100, 140)
                ))
                .count(),
            3
        );
        assert!(output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == "spi_transfer")));
        click(&mut app, cards[0]);
        assert_eq!(app.graph_view.selected.as_deref(), Some("app/main.c"));
        assert!(app.graph_view.edge.is_none());
        frame(
            &mut app,
            vec![
                egui::Event::PointerMoved(cards[0]),
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(0.0, 120.0),
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        assert!(app.graph_view.zoom > 1.0);
        assert_eq!(
            app.graph_view.layout.as_ref().unwrap().geometry.edges[0]
                .points
                .as_ptr(),
            cached_points
        );
        app.graph_view.ram = true;
        // A mode switch happens in the toolbar, outside the scrollable canvas.
        let ram_output = frame(&mut app, vec![egui::Event::PointerGone]);
        let ram_cards = &app.graph_view.layout.as_ref().unwrap().geometry.cards;
        assert!(ram_cards["app/main.c"].area() < ram_cards["drivers/spi.c"].area());
        assert_eq!(app.graph_view.zoom, 1.0);
        assert!(ram_output.shapes.iter().any(|shape| matches!(&shape.shape,
            egui::Shape::Text(text) if text.galley.text().contains("RAM 400 B")
        )));
        app.graph_view.grouped = true;
        frame(&mut app, vec![]);
        assert_eq!(
            app.graph_view
                .layout
                .as_ref()
                .unwrap()
                .geometry
                .headings
                .len(),
            3
        );
        app.graph_view.focused = true;
        let focused_output = frame(&mut app, vec![]);
        assert_eq!(app.visible_rows, 2);
        let focused_arrow = focused_output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Path(path) if !path.closed && path.points.len() > 3 => {
                    Some(path.points[path.points.len() / 2])
                }
                _ => None,
            })
            .unwrap();
        click(&mut app, focused_arrow);
        assert!(app.graph_view.selected.is_none());
        assert_eq!(
            app.graph_view.edge,
            Some(("app/main.c".into(), "drivers/spi.c".into()))
        );
        frame(&mut app, vec![]);
        assert_eq!(app.visible_rows, 3);
        app.graph_view.focused = false;
        app.search = "spi.c".into();
        frame(&mut app, vec![]);
        assert_eq!(app.visible_rows, 2);
        let geometry = &app.graph_view.layout.as_ref().unwrap().geometry;
        assert!(geometry.cards.contains_key("app/main.c"));
        assert!(!geometry.cards.contains_key("unused/main.c"));
        assert_eq!(geometry.edges.len(), 1);
        app.search = "no match".into();
        frame(&mut app, vec![]);
        assert_eq!(app.visible_rows, 0);
    }
}
