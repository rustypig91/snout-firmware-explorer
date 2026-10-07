//! In-process DOT-style layout, with geometry retained for egui hit testing.
use super::egui;
use layout::{
    core::{
        base::Orientation,
        format::{ClipHandle, RenderBackend},
        geometry::Point,
        style::StyleAttr,
    },
    std_shapes::{
        render::render_arrow,
        shapes::{Arrow, Element, ShapeKind},
    },
    topo::layout::VisualGraph,
};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, PartialEq)]
pub(super) struct NodeSpec {
    pub id: String,
    pub directory: String,
    pub size: egui::Vec2,
}

#[derive(Clone, PartialEq)]
pub(super) struct LayoutInput {
    pub nodes: Vec<NodeSpec>,
    pub edges: Vec<(String, String)>,
    pub grouped: bool,
    pub ram: bool,
    pub vertical: bool,
}

pub(super) struct RoutedEdge {
    pub from: String,
    pub to: String,
    /// Always ordered from the referencing unit to the defining unit.
    pub points: Vec<egui::Pos2>,
}

pub(super) struct GraphLayout {
    pub cards: BTreeMap<String, egui::Rect>,
    pub edges: Vec<RoutedEdge>,
    pub headings: Vec<(String, egui::Pos2)>,
    pub bounds: egui::Rect,
}

pub(super) struct CachedLayout {
    pub input: LayoutInput,
    pub geometry: GraphLayout,
}

/// Zero bytes uses the label-sized floor; the largest visible contribution
/// uses 25 times that area. Normalize against zero even when all sizes are equal.
pub(super) fn card_size(minimum: egui::Vec2, bytes: u64, largest: u64) -> egui::Vec2 {
    let relative = if largest == 0 {
        0.0
    } else {
        bytes.min(largest) as f64 / largest as f64
    };
    minimum * (1.0 + 24.0 * relative).sqrt() as f32
}

pub(super) fn compute(input: &LayoutInput) -> GraphLayout {
    if input.grouped || input.nodes.len() < 2 {
        return compute_connected(input);
    }
    // Lay out weakly connected components separately. Unconnected units must
    // not turn into one enormous rank that shrinks every label in the graph.
    let mut neighbors: BTreeMap<&str, Vec<&str>> = input
        .nodes
        .iter()
        .map(|node| (node.id.as_str(), vec![]))
        .collect();
    for (from, to) in &input.edges {
        neighbors.get_mut(from.as_str()).unwrap().push(to);
        neighbors.get_mut(to.as_str()).unwrap().push(from);
    }
    let mut component_ids = BTreeMap::new();
    let mut components = vec![];
    for node in &input.nodes {
        if component_ids.contains_key(node.id.as_str()) {
            continue;
        }
        let component = components.len();
        let mut pending = vec![node.id.as_str()];
        while let Some(id) = pending.pop() {
            if component_ids.contains_key(id) {
                continue;
            }
            component_ids.insert(id, component);
            pending.extend(neighbors[id].iter().copied());
        }
        if component_ids.len() == input.nodes.len() && component == 0 {
            return compute_connected(input);
        }
        components.push(LayoutInput {
            nodes: vec![],
            edges: vec![],
            grouped: input.grouped,
            ram: input.ram,
            vertical: input.vertical,
        });
    }
    // Partition once, preserving node and edge order within each component.
    // Rescanning or cloning the entire input per isolated unit is quadratic.
    for node in &input.nodes {
        components[component_ids[node.id.as_str()]]
            .nodes
            .push(node.clone());
    }
    for edge in &input.edges {
        components[component_ids[edge.0.as_str()]]
            .edges
            .push(edge.clone());
    }
    let mut parts: Vec<_> = components.iter().map(compute_connected).collect();
    parts.sort_by(|a, b| b.bounds.height().total_cmp(&a.bounds.height()));
    let area: f32 = parts.iter().map(|p| p.bounds.area()).sum();
    let width = (area * 1.5)
        .sqrt()
        .max(parts.iter().map(|p| p.bounds.width()).fold(0.0, f32::max));
    let mut result = GraphLayout {
        cards: BTreeMap::new(),
        edges: vec![],
        headings: vec![],
        bounds: egui::Rect::NOTHING,
    };
    let (mut x, mut y, mut row_height) = (0.0, 0.0, 0.0_f32);
    for part in parts {
        if x > 0.0 && x + part.bounds.width() > width {
            x = 0.0;
            y += row_height + 24.0;
            row_height = 0.0;
        }
        let offset = egui::pos2(x, y) - part.bounds.min;
        result.bounds = result.bounds.union(part.bounds.translate(offset));
        row_height = row_height.max(part.bounds.height());
        x += part.bounds.width() + 24.0;
        result.cards.extend(
            part.cards
                .into_iter()
                .map(|(id, rect)| (id, rect.translate(offset))),
        );
        result.edges.extend(part.edges.into_iter().map(|mut edge| {
            for point in &mut edge.points {
                *point += offset;
            }
            edge
        }));
    }
    result
}

/// Bounded traversal and wrapped rank placement for large connected graphs. Routes
/// leave each column through its gutters and run outside the cards, so no
/// per-edge all-pairs obstacle search is needed.
fn compute_large(input: &LayoutInput) -> GraphLayout {
    let indexes: BTreeMap<_, _> = input
        .nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.id.as_str(), i))
        .collect();
    let mut outgoing = vec![Vec::new(); input.nodes.len()];
    let mut incoming = vec![0usize; input.nodes.len()];
    for (from, to) in &input.edges {
        let (a, b) = (indexes[from.as_str()], indexes[to.as_str()]);
        outgoing[a].push(b);
        incoming[b] += 1;
    }
    let mut rank = vec![usize::MAX; input.nodes.len()];
    let mut queue = std::collections::VecDeque::new();
    for (i, count) in incoming.iter().enumerate() {
        if *count == 0 {
            rank[i] = 0;
            queue.push_back(i);
        }
    }
    // Sort cyclic seeds once, then visit each candidate at most once. Repeated
    // full scans become quadratic for graphs with many separate cycles.
    let mut seeds: Vec<_> = (0..rank.len()).collect();
    seeds.sort_unstable_by_key(|&i| (std::cmp::Reverse(outgoing[i].len()), i));
    let mut seeds = seeds.into_iter();
    loop {
        while let Some(i) = queue.pop_front() {
            for &next in &outgoing[i] {
                if rank[next] == usize::MAX {
                    rank[next] = rank[i] + 1;
                    queue.push_back(next);
                }
            }
        }
        // Cyclic components have no zero-incoming root. Choose a deterministic
        // seed with many references rather than leaving their cards unplaced.
        let next = seeds.find(|&i| rank[i] == usize::MAX);
        let Some(next) = next else {
            break;
        };
        rank[next] = 0;
        queue.push_back(next);
    }
    let mut lanes: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (i, node) in input.nodes.iter().enumerate() {
        let lane = if input.grouped {
            node.directory.clone()
        } else {
            format!("{:020}", rank[i])
        };
        lanes.entry(lane).or_default().push(i);
    }
    let vertical = input.vertical && !input.grouped;
    let transform = |point: egui::Pos2| {
        if vertical {
            egui::pos2(point.y, point.x)
        } else {
            point
        }
    };
    // A broad rank (many sources referencing the same few dependencies) must
    // not become an unbounded row after transposing to top-to-bottom. Wrap each
    // rank into bands with a cross-axis budget based on total card area.
    let area: f32 = input
        .nodes
        .iter()
        .map(|node| node.size.x * node.size.y)
        .sum();
    let cross_limit = (area * if vertical { 1.5 } else { 0.75 }).sqrt().max(300.0);
    let mut cards = BTreeMap::new();
    let mut headings = Vec::new();
    let mut gutters = BTreeMap::new();
    let mut band_ids = BTreeMap::new();
    let mut band = 0usize;
    let mut x = 0.0;
    for (lane, members) in lanes {
        let mut chunks: Vec<Vec<usize>> = vec![vec![]];
        let mut length = 0.0;
        for i in members {
            let size = input.nodes[i].size;
            let cross = if vertical { size.x } else { size.y };
            if length > 0.0 && length + cross > cross_limit {
                chunks.push(vec![]);
                length = 0.0;
            }
            chunks.last_mut().unwrap().push(i);
            length += cross + 36.0;
        }
        for members in chunks {
            let width = members
                .iter()
                .map(|&i| {
                    if vertical {
                        input.nodes[i].size.y
                    } else {
                        input.nodes[i].size.x
                    }
                })
                .fold(0.0, f32::max);
            // Repeated directory headings need the same horizontal clearance
            // as cards, otherwise long names overlap across wrapped bands.
            let width = if input.grouped {
                width.max(lane.chars().count() as f32 * 8.0)
            } else {
                width
            };
            if input.grouped {
                // Every wrapped band needs its own label to preserve directory
                // ownership when a group spans several columns.
                headings.push((lane.clone(), transform(egui::pos2(x + width / 2.0, -24.0))));
            }
            let mut y = 0.0;
            for i in members {
                let node = &input.nodes[i];
                let size = if vertical {
                    egui::vec2(node.size.y, node.size.x)
                } else {
                    node.size
                };
                let min = egui::pos2(x + (width - size.x) / 2.0, y);
                cards.insert(
                    node.id.clone(),
                    egui::Rect::from_min_max(transform(min), transform(min + size)),
                );
                gutters.insert(node.id.as_str(), (x - 24.0, x + width + 24.0));
                band_ids.insert(node.id.as_str(), band);
                y += size.y + 36.0;
            }
            x += width + 96.0;
            band += 1;
        }
    }
    let mut bounds = cards.values().fold(egui::Rect::NOTHING, |b, r| b.union(*r));
    let bottom = if vertical {
        bounds.right()
    } else {
        bounds.bottom()
    };
    let edge_pairs: BTreeSet<_> = input.edges.iter().map(|(from, to)| (from, to)).collect();
    let edges = input
        .edges
        .iter()
        .enumerate()
        .map(|(i, (from, to))| {
            let a = cards[from];
            let b = cards[to];
            // Work in left-to-right coordinates, transposing back only once.
            let a = egui::Rect::from_min_max(transform(a.min), transform(a.max));
            let b = egui::Rect::from_min_max(transform(b.min), transform(b.max));
            let source_band = band_ids[from.as_str()];
            let target_band = band_ids[to.as_str()];
            let forward = source_band <= target_band;
            let start = if forward {
                a.right_center()
            } else {
                a.left_center()
            };
            let end = if source_band == target_band || !forward {
                b.right_center()
            } else {
                b.left_center()
            };
            let source_gutter = if forward {
                gutters[from.as_str()].1
            } else {
                gutters[from.as_str()].0
            };
            let target_gutter = if source_band == target_band || !forward {
                gutters[to.as_str()].1
            } else {
                gutters[to.as_str()].0
            };
            let route = if source_band.abs_diff(target_band) == 1
                && (start.y - end.y).abs() < 1.0
                && edge_pairs.contains(&(to, from))
            {
                // Aligned reciprocal connections otherwise collapse to the
                // same straight line, making one direction impossible to pick.
                // Separate their horizontal tracks inside the shared gutter.
                let track = start.y + if from < to { -6.0 } else { 6.0 };
                vec![
                    start,
                    egui::pos2(source_gutter, start.y),
                    egui::pos2(source_gutter, track),
                    egui::pos2(target_gutter, track),
                    egui::pos2(target_gutter, end.y),
                    end,
                ]
            } else if source_band.abs_diff(target_band) <= 1 {
                // Same/adjacent bands can connect entirely through their shared
                // gutter instead of taking every edge around the whole graph.
                let bridge =
                    (source_gutter + target_gutter) / 2.0 + if from < to { -6.0 } else { 6.0 };
                vec![
                    start,
                    egui::pos2(bridge, start.y),
                    egui::pos2(bridge, end.y),
                    end,
                ]
            } else {
                let track = 48.0 + (i % 24) as f32 * 4.0;
                let outside = if forward { -track } else { bottom + track };
                vec![
                    start,
                    egui::pos2(source_gutter, start.y),
                    egui::pos2(source_gutter, outside),
                    egui::pos2(target_gutter, outside),
                    egui::pos2(target_gutter, end.y),
                    end,
                ]
            };
            let points = route.into_iter().map(transform).collect::<Vec<_>>();
            for point in &points {
                bounds.extend_with(*point);
            }
            RoutedEdge {
                from: from.clone(),
                to: to.clone(),
                points,
            }
        })
        .collect();
    for (label, point) in &headings {
        bounds = bounds.union(egui::Rect::from_center_size(
            *point,
            egui::vec2(label.chars().count() as f32 * 8.0, 24.0),
        ));
    }
    GraphLayout {
        cards,
        edges,
        headings,
        bounds: bounds.expand(12.0),
    }
}

fn compute_connected(input: &LayoutInput) -> GraphLayout {
    // Dense firmware graphs make rank optimization, connector expansion and
    // per-edge visibility routing prohibitively expensive. Keep the full graph
    // using bounded placement and gutter routes; focused subsets retain detail.
    if input.nodes.len() > 100 || input.edges.len() > 250 {
        return compute_large(input);
    }
    let orientation = if input.vertical && !input.grouped {
        Orientation::TopToBottom
    } else {
        Orientation::LeftToRight
    };
    let mut vg = VisualGraph::new(orientation);
    // Avoid the library's expensive diagnostic DAG checks on every insertion.
    vg.dag.set_validate(false);
    let mut handles = BTreeMap::new();
    for node in &input.nodes {
        let element = Element::create(
            ShapeKind::new_box(""),
            StyleAttr::simple(),
            orientation,
            Point::new(node.size.x.into(), node.size.y.into()),
        );
        handles.insert(node.id.clone(), vg.add_node(element));
    }
    let mut collector = CurveCollector::default();
    let mut headings = vec![];
    if input.grouped {
        // Directory lanes are an explicit alternative to dependency ranks. The
        // library still supplies connection points and curves for these boxes.
        let mut groups: BTreeMap<&str, Vec<&NodeSpec>> = BTreeMap::new();
        for node in &input.nodes {
            groups.entry(&node.directory).or_default().push(node);
        }
        let mut lane_right = BTreeMap::new();
        let mut x = 0.0;
        for (directory, nodes) in groups {
            let width = nodes.iter().map(|n| n.size.x).fold(0.0_f32, f32::max);
            for node in &nodes {
                lane_right.insert(node.id.as_str(), x + width);
            }
            headings.push((
                super::short_path(directory, input.nodes.iter().map(|n| n.directory.as_str())),
                egui::pos2(x + width / 2.0, -30.0),
            ));
            let mut y = 0.0;
            for node in nodes {
                vg.element_mut(handles[&node.id]).move_to(Point::new(
                    (x + width / 2.0).into(),
                    (y + node.size.y / 2.0).into(),
                ));
                y += node.size.y + 60.0;
            }
            x += width + 150.0;
        }
        for (index, (from, to)) in input.edges.iter().enumerate() {
            let source = vg.element(handles[from]).clone();
            let target = vg.element(handles[to]).clone();
            let mut elements = vec![source.clone()];
            let same_lane = (source.pos.center().x - target.pos.center().x).abs() < 1.0;
            let clearance = 45.0;
            let source_x;
            let target_x;
            if same_lane {
                // Keep parallel routes within the 150-point directory gap,
                // even for dense graphs. Reserve room for curve controls and
                // the reciprocal-edge offset applied below.
                let offset = 60.0 * index as f64 / input.edges.len().max(1) as f64;
                source_x = f64::from(lane_right[from.as_str()]) + clearance + offset;
                target_x = source_x;
            } else if source.pos.center().x < target.pos.center().x {
                source_x = f64::from(lane_right[from.as_str()]) + clearance;
                target_x =
                    2.0 * target.pos.center().x - f64::from(lane_right[to.as_str()]) - clearance;
            } else {
                source_x =
                    2.0 * source.pos.center().x - f64::from(lane_right[from.as_str()]) - clearance;
                target_x = f64::from(lane_right[to.as_str()]) + clearance;
            }
            let mut waypoints = vec![Point::new(source_x, source.pos.center().y)];
            if !same_lane {
                // Cross-directory connections pass above every lane, rather
                // than disappearing underneath intermediate directory boxes.
                let y = -80.0 - index as f64 * 3.0;
                waypoints.extend([Point::new(source_x, y), Point::new(target_x, y)]);
            }
            waypoints.push(Point::new(target_x, target.pos.center().y));
            for point in waypoints {
                let mut connector = Element::empty_connector(Orientation::LeftToRight);
                connector.move_to(point);
                elements.push(connector);
            }
            elements.push(target);
            render_arrow(
                &mut collector,
                false,
                &elements,
                &Arrow::simple_with_properties("", index.to_string()),
            );
        }
    } else if !input.nodes.is_empty() {
        for (index, (from, to)) in input.edges.iter().enumerate() {
            vg.add_edge(
                Arrow::simple_with_properties("", index.to_string()),
                handles[from],
                handles[to],
            );
        }
        vg.do_it(false, false, false, &mut collector);
    }
    let cards: BTreeMap<_, _> = handles
        .into_iter()
        .map(|(id, handle)| {
            let (min, max) = vg.pos(handle).bbox(false);
            (id, egui::Rect::from_min_max(pos(min), pos(max)))
        })
        .collect();
    let edge_pairs: BTreeSet<_> = input.edges.iter().map(|(f, t)| (f, t)).collect();
    let edges: Vec<_> = collector
        .curves
        .into_iter()
        .map(|(index, mut points)| {
            let (from, to) = &input.edges[index];
            if from != to && edge_pairs.contains(&(to, from)) {
                // Reciprocal arrows otherwise share exactly the same curve. Taper
                // the lane offset to zero at the boxes so endpoints stay attached.
                let direction = (cards[to].center() - cards[from].center()).normalized();
                let normal = egui::vec2(-direction.y, direction.x);
                let last = points.len().saturating_sub(1).max(1) as f32;
                for (i, point) in points.iter_mut().enumerate() {
                    *point += normal * (6.0 * (std::f32::consts::PI * i as f32 / last).sin());
                }
            }
            // Cubic controls and reciprocal offsets can overshoot the layout
            // corridors in dense graphs. Reroute only those connections through
            // a visibility graph around the boxes, retaining the real endpoints.
            if !route_is_clear(&points, &cards) {
                let clearance = if from < to { 8.0 } else { 14.0 };
                if let Some(route) =
                    obstacle_route(points[0], *points.last().unwrap(), &cards, clearance)
                        .or_else(|| obstacle_route(points[0], *points.last().unwrap(), &cards, 1.0))
                {
                    let rounded = round_route(&route);
                    points = if route_is_clear(&rounded, &cards) {
                        rounded
                    } else {
                        route
                    };
                }
            }
            RoutedEdge {
                from: from.clone(),
                to: to.clone(),
                points,
            }
        })
        .collect();
    let mut bounds = cards
        .values()
        .fold(egui::Rect::NOTHING, |b, card| b.union(*card));
    for edge in &edges {
        for point in &edge.points {
            bounds.extend_with(*point);
        }
    }
    for (label, center) in &headings {
        bounds = bounds.union(egui::Rect::from_center_size(
            *center,
            egui::vec2(label.chars().count() as f32 * 8.0, 24.0),
        ));
    }
    GraphLayout {
        cards,
        edges,
        headings,
        bounds: bounds.expand(24.0),
    }
}

fn segment_crosses_rect(start: egui::Pos2, end: egui::Pos2, rect: egui::Rect) -> bool {
    if !rect.intersects(egui::Rect::from_two_pos(start, end)) {
        return false;
    }
    if start == end {
        return rect.contains(start);
    }
    let direction = (end - start).normalized();
    rect.intersects_ray(start, direction) && rect.intersects_ray(end, -direction)
}

fn route_is_clear(points: &[egui::Pos2], cards: &BTreeMap<String, egui::Rect>) -> bool {
    cards.values().all(|card| {
        !points
            .windows(2)
            .any(|segment| segment_crosses_rect(segment[0], segment[1], card.shrink(0.1)))
    })
}

/// Find the shortest clear path along box corners. This also treats the source
/// and target as obstacles so the arrow leaves and enters their outside edges.
fn obstacle_route(
    start: egui::Pos2,
    end: egui::Pos2,
    cards: &BTreeMap<String, egui::Rect>,
    clearance: f32,
) -> Option<Vec<egui::Pos2>> {
    let mut vertices = vec![start, end];
    for card in cards.values() {
        let card = card.expand(clearance);
        vertices.extend([
            card.left_top(),
            card.right_top(),
            card.right_bottom(),
            card.left_bottom(),
        ]);
    }
    let mut distances = vec![f32::INFINITY; vertices.len()];
    let mut previous = vec![None; vertices.len()];
    let mut visited = vec![false; vertices.len()];
    distances[0] = 0.0;
    loop {
        let current = (0..vertices.len())
            .filter(|&i| !visited[i] && distances[i].is_finite())
            .min_by(|&a, &b| distances[a].total_cmp(&distances[b]))?;
        if current == 1 {
            break;
        }
        visited[current] = true;
        for next in 0..vertices.len() {
            if visited[next] {
                continue;
            }
            let distance = distances[current] + vertices[current].distance(vertices[next]);
            if distance < distances[next]
                && route_is_clear(&[vertices[current], vertices[next]], cards)
            {
                distances[next] = distance;
                previous[next] = Some(current);
            }
        }
    }
    let mut route = vec![end];
    let mut current = 1;
    while current != 0 {
        current = previous[current]?;
        route.push(vertices[current]);
    }
    route.reverse();
    Some(route)
}

fn round_route(route: &[egui::Pos2]) -> Vec<egui::Pos2> {
    let mut points = vec![route[0]];
    for corner in route.windows(3) {
        let radius = 8.0_f32
            .min(corner[1].distance(corner[0]) / 4.0)
            .min(corner[1].distance(corner[2]) / 4.0);
        let entry = corner[1] + (corner[0] - corner[1]).normalized() * radius;
        let exit = corner[1] + (corner[2] - corner[1]).normalized() * radius;
        for step in 0..=8 {
            let t = step as f32 / 8.0;
            points.push(entry.lerp(corner[1], t).lerp(corner[1].lerp(exit, t), t));
        }
    }
    points.push(*route.last().unwrap());
    points
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hundreds_of_unconnected_units_pack_into_rows_without_hiding_connections() {
        let mut input = input();
        input.nodes = (0..200)
            .map(|i| NodeSpec {
                id: format!("unit{i}"),
                directory: "src".into(),
                size: egui::vec2(120.0, 50.0),
            })
            .collect();
        input.edges = vec![("unit0".into(), "unit1".into())];
        for vertical in [false, true] {
            input.vertical = vertical;
            let geometry = compute(&input);
            assert_eq!(geometry.cards.len(), 200);
            assert_eq!(geometry.edges.len(), 1);
            assert!(geometry.bounds.width() / geometry.bounds.height() < 3.0);
            assert!(geometry.bounds.height() / geometry.bounds.width() < 3.0);
            for (i, card) in geometry.cards.values().enumerate() {
                assert!(geometry.bounds.contains_rect(*card));
                for other in geometry.cards.values().skip(i + 1) {
                    assert!(!card.intersects(*other));
                }
            }
            let edge = &geometry.edges[0];
            assert!(geometry.cards[&edge.from]
                .expand(0.01)
                .contains(edge.points[0]));
            assert!(geometry.cards[&edge.to]
                .expand(0.01)
                .contains(*edge.points.last().unwrap()));
            assert!(route_is_clear(&edge.points, &geometry.cards));
        }
    }

    #[test]
    fn packing_preserves_edges_in_multiple_components() {
        let mut input = input();
        input.nodes = (0..1000)
            .map(|i| NodeSpec {
                id: format!("unit{i}"),
                directory: "src".into(),
                size: egui::vec2(120.0, 50.0),
            })
            .collect();
        input.edges = vec![
            ("unit0".into(), "unit1".into()),
            ("unit501".into(), "unit500".into()),
        ];
        let geometry = compute(&input);
        assert_eq!(geometry.cards.len(), input.nodes.len());
        assert_eq!(geometry.edges.len(), input.edges.len());
        for (from, to) in &input.edges {
            let edge = geometry
                .edges
                .iter()
                .find(|edge| &edge.from == from && &edge.to == to)
                .unwrap();
            assert!(geometry.cards[from].expand(0.01).contains(edge.points[0]));
            assert!(geometry.cards[to]
                .expand(0.01)
                .contains(*edge.points.last().unwrap()));
            assert!(route_is_clear(&edge.points, &geometry.cards));
        }
    }

    fn input() -> LayoutInput {
        LayoutInput {
            nodes: [
                "app/main.c",
                "drivers/spi.c",
                "drivers/config.c",
                "isolated.c",
            ]
            .into_iter()
            .enumerate()
            .map(|(index, id)| NodeSpec {
                id: id.into(),
                directory: id
                    .rsplit_once('/')
                    .map(|p| p.0)
                    .unwrap_or("[no directory]")
                    .into(),
                size: card_size(egui::vec2(100.0, 42.0), index as u64 * 100, 300),
            })
            .collect(),
            edges: [
                ("app/main.c", "drivers/spi.c"),
                ("drivers/spi.c", "drivers/config.c"),
                ("drivers/config.c", "app/main.c"),
                ("drivers/spi.c", "app/main.c"),
            ]
            .into_iter()
            .map(|(f, t)| (f.into(), t.into()))
            .collect(),
            grouped: false,
            ram: false,
            vertical: false,
        }
    }

    #[test]
    fn cyclic_and_reciprocal_routes_preserve_identity_direction_and_clear_boxes() {
        for (grouped, vertical) in [(false, false), (false, true), (true, false)] {
            let mut input = input();
            input.grouped = grouped;
            input.vertical = vertical;
            let geometry = compute(&input);
            assert_eq!(geometry.cards.len(), input.nodes.len());
            assert_eq!(geometry.edges.len(), input.edges.len());
            for (i, card) in geometry.cards.values().enumerate() {
                assert!(card.is_positive() && card.is_finite());
                assert!(geometry.bounds.contains_rect(*card));
                for other in geometry.cards.values().skip(i + 1) {
                    assert!(!card.intersects(*other));
                }
            }
            for edge in &geometry.edges {
                assert!(input.edges.contains(&(edge.from.clone(), edge.to.clone())));
                let source = geometry.cards[&edge.from];
                let target = geometry.cards[&edge.to];
                assert!(source.expand(0.01).contains(edge.points[0]));
                assert!(target.expand(0.01).contains(*edge.points.last().unwrap()));
                for point in &edge.points {
                    assert!(point.x.is_finite() && point.y.is_finite());
                    assert!(geometry.bounds.contains(*point));
                }
                // Both tips and their bases remain outside the target's interior.
                for scale in [0.2, 1.0, 8.0] {
                    let points: Vec<_> = edge
                        .points
                        .iter()
                        .map(|p| egui::Pos2::ZERO + p.to_vec2() * scale)
                        .collect();

                    let scaled_target = egui::Rect::from_min_max(
                        egui::Pos2::ZERO + target.min.to_vec2() * scale,
                        egui::Pos2::ZERO + target.max.to_vec2() * scale,
                    );
                    let head = super::super::arrow_head(&points, scale, scaled_target).unwrap();
                    let base = head[1].lerp(head[2], 0.5);
                    assert!((head[0] - base).dot(head[0] - points[points.len() - 2]) > 0.0);
                    assert!(head
                        .iter()
                        .all(|point| !scaled_target.shrink(0.1).contains(*point)));
                }
            }
            let forward = &geometry.edges[0].points;
            let reverse = &geometry.edges[3].points;
            assert!(forward[forward.len() / 2].distance(reverse[reverse.len() / 2]) > 1.0);
            assert_eq!(geometry.headings.len(), if grouped { 3 } else { 0 });
        }
    }

    #[test]
    fn chain_flows_left_to_right_and_disconnected_graphs_are_supported() {
        let mut input = input();
        input.edges.truncate(2);
        let geometry = compute(&input);
        assert!(geometry.cards["app/main.c"].right() < geometry.cards["drivers/spi.c"].left());
        assert!(
            geometry.cards["drivers/spi.c"].right() < geometry.cards["drivers/config.c"].left()
        );
        input.edges.clear();
        let geometry = compute(&input);
        assert_eq!(geometry.cards.len(), 4);
        assert!(geometry.edges.is_empty());
        input.nodes.truncate(1);
        assert_eq!(compute(&input).cards.len(), 1);
    }

    #[test]
    fn grouped_routes_do_not_cross_intermediate_directory_boxes() {
        let mut input = input();
        input.grouped = true;
        input.nodes.truncate(3);
        for (node, directory) in input.nodes.iter_mut().zip(["a", "b", "c"]) {
            node.directory = directory.into();
        }
        input.edges = vec![
            (input.nodes[0].id.clone(), input.nodes[2].id.clone()),
            (input.nodes[2].id.clone(), input.nodes[0].id.clone()),
        ];
        for same_lane in [false, true] {
            if same_lane {
                for node in &mut input.nodes {
                    node.directory = "shared".into();
                }
                input.nodes[1].size = egui::vec2(300.0, 80.0);
            }
            let geometry = compute(&input);
            for edge in &geometry.edges {
                let obstacle = geometry.cards[&input.nodes[1].id].shrink(0.1);
                for segment in edge.points.windows(2) {
                    assert!(
                        !obstacle.intersects(egui::Rect::from_two_pos(segment[0], segment[1])),
                        "route crosses unrelated box: {segment:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn dense_grouped_routes_stay_inside_the_gap_between_directories() {
        let mut input = input();
        input.grouped = true;
        input.nodes = (0..12)
            .map(|index| NodeSpec {
                id: format!("a/{index}.c"),
                directory: "a".into(),
                size: egui::vec2(100.0, 42.0),
            })
            .collect();
        input.edges = input
            .nodes
            .iter()
            .flat_map(|from| {
                input
                    .nodes
                    .iter()
                    .filter(move |to| from.id != to.id)
                    .map(move |to| (from.id.clone(), to.id.clone()))
            })
            .collect();
        input.nodes.push(NodeSpec {
            id: "b/obstacle.c".into(),
            directory: "b".into(),
            size: egui::vec2(100.0, 1500.0),
        });
        let geometry = compute(&input);
        let obstacle = geometry.cards["b/obstacle.c"].shrink(0.1);
        for edge in &geometry.edges {
            for segment in edge.points.windows(2) {
                assert!(
                    !obstacle.intersects(egui::Rect::from_two_pos(segment[0], segment[1])),
                    "{} -> {} crosses the next directory: {segment:?}",
                    edge.from,
                    edge.to
                );
            }
        }
    }

    #[test]
    fn directory_headings_distinguish_matching_suffixes() {
        let mut input = input();
        input.grouped = true;
        input.nodes.truncate(2);
        input.edges.truncate(1);
        input.nodes[0].directory = "project-a/src/drivers".into();
        input.nodes[1].directory = "project-b/src/drivers".into();
        let geometry = compute(&input);
        let labels: Vec<_> = geometry
            .headings
            .iter()
            .map(|(label, _)| label.as_str())
            .collect();
        assert_eq!(labels, ["project-a/src/drivers", "project-b/src/drivers"]);
    }

    #[test]
    fn dense_routes_clear_unrelated_boxes() {
        for grouped in [false, true] {
            for vertical in [false, true] {
                let mut input = input();
                input.grouped = grouped;
                input.vertical = vertical;
                input.nodes = (0..8)
                    .map(|index| NodeSpec {
                        id: format!("{index}.c"),
                        directory: format!("dir{}", index / 3),
                        size: egui::vec2(100.0 + index as f32 * 5.0, 42.0 + index as f32 * 2.0),
                    })
                    .collect();
                input.edges = input
                    .nodes
                    .iter()
                    .flat_map(|from| {
                        input
                            .nodes
                            .iter()
                            .filter(move |to| to.id != from.id)
                            .map(move |to| (from.id.clone(), to.id.clone()))
                    })
                    .collect();
                let geometry = compute(&input);
                assert_eq!(geometry.edges.len(), input.edges.len());
                for edge in &geometry.edges {
                    assert!(geometry.cards[&edge.from]
                        .expand(0.01)
                        .contains(edge.points[0]));
                    assert!(geometry.cards[&edge.to]
                        .expand(0.01)
                        .contains(*edge.points.last().unwrap()));
                    for (id, card) in &geometry.cards {
                        for segment in edge.points.windows(2) {
                            assert!(!segment_crosses_rect(segment[0], segment[1], card.shrink(0.1)),
                                "grouped={grouped} vertical={vertical}: {} -> {} crosses {id} at {segment:?} ({card:?})", edge.from, edge.to);
                        }
                    }
                    let reverse = geometry
                        .edges
                        .iter()
                        .find(|other| other.from == edge.to && other.to == edge.from)
                        .unwrap();
                    assert!(
                        edge.points.len() != reverse.points.len()
                            || edge
                                .points
                                .iter()
                                .zip(reverse.points.iter().rev())
                                .any(|(a, b)| a.distance(*b) > 1.0),
                        "reciprocal routes overlap: {} -> {}",
                        edge.from,
                        edge.to
                    );
                }
            }
        }
    }

    #[test]
    fn varied_routes_and_arrowhead_wings_clear_boxes_at_every_zoom() {
        for seed in 0..20 {
            for grouped in [false, true] {
                for vertical in [false, true] {
                    let mut input = input();
                    input.grouped = grouped;
                    input.vertical = vertical;
                    input.nodes = (0..12)
                        .map(|i| NodeSpec {
                            id: format!("{i}"),
                            directory: format!("dir{}", i / 4),
                            size: egui::vec2(
                                70.0 + ((i * 37 + seed * 17) % 200) as f32,
                                30.0 + ((i * 19 + seed * 11) % 80) as f32,
                            ),
                        })
                        .collect();
                    input.edges = (0..12)
                        .flat_map(|i| {
                            (0..12)
                                .filter(move |&j| i != j && (i * 31 + j * 17 + seed * 13) % 7 == 0)
                                .map(move |j| (format!("{i}"), format!("{j}")))
                        })
                        .collect();
                    let geometry = compute(&input);
                    for edge in &geometry.edges {
                        for scale in [0.05, 0.2, 1.0, 8.0] {
                            let points: Vec<_> = edge
                                .points
                                .iter()
                                .map(|p| egui::Pos2::ZERO + p.to_vec2() * scale)
                                .collect();
                            let target = geometry.cards[&edge.to];
                            let target = egui::Rect::from_min_max(
                                egui::Pos2::ZERO + target.min.to_vec2() * scale,
                                egui::Pos2::ZERO + target.max.to_vec2() * scale,
                            );
                            let head = super::super::arrow_head(&points, scale, target).unwrap();
                            assert!(head.iter().all(|p| !target.shrink(0.01).contains(*p)), "head inside target seed={seed} grouped={grouped} vertical={vertical} scale={scale}: {} -> {} {head:?} {target:?}", edge.from, edge.to);
                        }
                        assert!(
                            route_is_clear(&edge.points, &geometry.cards),
                            "seed={seed} grouped={grouped} vertical={vertical}: {} -> {}",
                            edge.from,
                            edge.to
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn memory_sizing_is_monotonic_bounded_and_handles_zero_usage() {
        let minimum = egui::vec2(100.0, 42.0);
        let mut previous = minimum;
        for bytes in [0, 1, 100, 10_000, 1_000_000, u64::MAX] {
            let size = card_size(minimum, bytes, u64::MAX);
            assert!(size.x >= previous.x && size.y >= previous.y);
            assert!(size.x <= minimum.x * 5.0 && size.y <= minimum.y * 5.0);
            previous = size;
        }
        assert_eq!(card_size(minimum, 0, 0), minimum);
        assert_eq!(card_size(minimum, 52, 52), minimum * 5.0);
        // Approximately 27.14 KiB versus 52 B: the small nonzero node must
        // exceed the floor; half the largest byte count gets halfway area.
        let largest = card_size(minimum, 27_792, 27_792);
        let smallest = card_size(minimum, 52, 27_792);
        let midpoint = card_size(minimum, 13_896, 27_792);
        let area = |size: egui::Vec2| size.x * size.y;
        assert!(smallest.x > minimum.x);
        assert!((area(largest) / area(minimum) - 25.0).abs() < 0.001);
        assert!((area(midpoint) / area(minimum) - 13.0).abs() < 0.001);
        assert_eq!(card_size(minimum, 0, 27_792), minimum);
    }
}

fn pos(point: Point) -> egui::Pos2 {
    egui::pos2(point.x as f32, point.y as f32)
}

#[derive(Default)]
struct CurveCollector {
    curves: Vec<(usize, Vec<egui::Pos2>)>,
}

impl RenderBackend for CurveCollector {
    fn draw_arrow(
        &mut self,
        path: &[(Point, Point)],
        _: bool,
        head: (bool, bool),
        _: &StyleAttr,
        properties: Option<String>,
        _: &str,
    ) {
        // Match the library SVG backend's initial cubic and subsequent smooth
        // cubic segments; use the same samples for painting and hit testing.
        let mut points = vec![];
        let mut start = pos(path[0].0);
        let mut control1 = pos(path[0].1);
        for segment in path.iter().skip(1) {
            let control2 = pos(segment.0);
            let end = pos(segment.1);
            for i in 0..=24 {
                if i == 0 && !points.is_empty() {
                    continue;
                }
                let t = i as f32 / 24.0;
                let u = 1.0 - t;
                points.push(
                    egui::Pos2::ZERO
                        + start.to_vec2() * u.powi(3)
                        + control1.to_vec2() * (3.0 * u * u * t)
                        + control2.to_vec2() * (3.0 * u * t * t)
                        + end.to_vec2() * t.powi(3),
                );
            }
            start = end;
            control1 = end + (end - control2);
        }
        // layout-rs reverses cyclic edges internally; restore the real direction.
        if head.0 {
            points.reverse();
        }
        self.curves
            .push((properties.unwrap().parse().unwrap(), points));
    }
    fn draw_rect(
        &mut self,
        _: Point,
        _: Point,
        _: &StyleAttr,
        _: Option<String>,
        _: Option<ClipHandle>,
    ) {
    }
    fn draw_line(&mut self, _: Point, _: Point, _: &StyleAttr, _: Option<String>) {}
    fn draw_circle(&mut self, _: Point, _: Point, _: &StyleAttr, _: Option<String>) {}
    fn draw_text(&mut self, _: Point, _: &str, _: &StyleAttr) {}
    fn create_clip(&mut self, _: Point, _: Point, _: usize) -> ClipHandle {
        0
    }
}

#[test]
fn large_connected_firmware_graph_preserves_cards_and_clear_routes() {
    let mut input = LayoutInput {
        nodes: (0..301)
            .map(|i| NodeSpec {
                id: format!("unit{i:03}"),
                directory: format!("directory{}", i % 24),
                size: egui::vec2(180.0 + (i % 5) as f32 * 24.0, 48.0 + (i % 7) as f32 * 12.0),
            })
            .collect(),
        edges: (0..301)
            .flat_map(|i| {
                [1, 17].map(move |step| {
                    (
                        format!("unit{i:03}"),
                        format!("unit{:03}", (i + step) % 301),
                    )
                })
            })
            .collect(),
        grouped: false,
        ram: false,
        vertical: false,
    };
    for (grouped, vertical) in [(false, false), (false, true), (true, false)] {
        input.grouped = grouped;
        input.vertical = vertical;
        let start = std::time::Instant::now();
        let geometry = compute(&input);
        assert!(
            start.elapsed() < std::time::Duration::from_secs(3),
            "large layout exceeded its interactive budget"
        );
        assert_eq!(geometry.cards.len(), input.nodes.len());
        assert_eq!(geometry.edges.len(), input.edges.len());
        for (i, card) in geometry.cards.values().enumerate() {
            assert!(geometry.bounds.contains_rect(*card));
            for other in geometry.cards.values().skip(i + 1) {
                assert!(!card.intersects(*other));
            }
        }
        for (edge, (from, to)) in geometry.edges.iter().zip(&input.edges) {
            assert_eq!((&edge.from, &edge.to), (from, to));
            assert!(geometry.cards[from].contains(edge.points[0]));
            assert!(geometry.cards[to].contains(*edge.points.last().unwrap()));
            assert!(route_is_clear(&edge.points, &geometry.cards));
        }
        assert_eq!(geometry.headings.is_empty(), !grouped);
    }
}

#[test]
fn large_grouped_graph_places_many_separate_cycles_deterministically() {
    let count = 10_000;
    let input = LayoutInput {
        nodes: (0..count)
            .map(|i| NodeSpec {
                id: format!("unit{i:05}"),
                directory: format!("directory{}", i % 20),
                size: egui::vec2(120.0, 50.0),
            })
            .collect(),
        edges: (0..count)
            .map(|i| (format!("unit{i:05}"), format!("unit{:05}", i ^ 1)))
            .collect(),
        grouped: true,
        ram: false,
        vertical: false,
    };
    let start = std::time::Instant::now();
    let geometry = compute(&input);
    assert!(start.elapsed() < std::time::Duration::from_secs(3));
    assert_eq!(geometry.cards.len(), count);
    assert_eq!(geometry.edges.len(), count);
    assert!(geometry.headings.len() > 20, "directories must wrap");
    assert_eq!(
        geometry
            .headings
            .iter()
            .map(|(label, _)| label)
            .collect::<BTreeSet<_>>()
            .len(),
        20
    );
    assert!(geometry
        .cards
        .values()
        .all(|card| geometry.bounds.contains_rect(*card)));
    let repeated = compute(&input);
    assert_eq!(geometry.cards, repeated.cards);
    for (edge, repeated) in geometry.edges.iter().zip(&repeated.edges) {
        assert_eq!(edge.points, repeated.points);
    }
}

#[test]
fn broad_dependency_ranks_wrap_instead_of_collapsing_into_a_strip() {
    // Hundreds of source units with no incoming references share rank zero.
    // This was the pathological case: a single enormous row at fit-to-view.
    let mut input = LayoutInput {
        nodes: (0..301)
            .map(|i| NodeSpec {
                id: format!("unit{i:03}"),
                directory: "src".into(),
                size: card_size(egui::vec2(180.0, 48.0), 10 + (i % 11) as u64 * 100, 1010),
            })
            .collect(),
        edges: (0..300)
            .map(|i| (format!("unit{i:03}"), "unit300".into()))
            .collect(),
        grouped: false,
        ram: false,
        vertical: false,
    };
    for vertical in [false, true] {
        input.vertical = vertical;
        let geometry = compute(&input);
        let aspect = geometry.bounds.width() / geometry.bounds.height();
        assert!(
            (0.25..4.0).contains(&aspect),
            "graph became a strip: {aspect}"
        );
        assert_eq!(geometry.cards.len(), 301);
        assert_eq!(geometry.edges.len(), 300);
        let rank_coordinates: BTreeSet<_> = input.nodes[..300]
            .iter()
            .map(|node| {
                let center = geometry.cards[&node.id].center();
                (if vertical { center.y } else { center.x }) as i64
            })
            .collect();
        assert!(rank_coordinates.len() > 1, "broad rank must wrap");
        for (i, card) in geometry.cards.values().enumerate() {
            assert!(geometry.bounds.contains_rect(*card));
            for other in geometry.cards.values().skip(i + 1) {
                assert!(!card.intersects(*other));
            }
        }
        for edge in &geometry.edges {
            assert!(route_is_clear(&edge.points, &geometry.cards));
            assert!(geometry.cards[&edge.from].contains(edge.points[0]));
            assert!(geometry.cards[&edge.to].contains(*edge.points.last().unwrap()));
        }
    }
}

#[test]
fn wrapped_directory_bands_each_keep_their_heading() {
    let input = LayoutInput {
        nodes: (0..301)
            .map(|i| NodeSpec {
                id: format!("unit{i:03}"),
                directory: if i < 300 { "src" } else { "drivers" }.into(),
                size: egui::vec2(180.0, 48.0),
            })
            .collect(),
        edges: (0..300)
            .map(|i| (format!("unit{i:03}"), "unit300".into()))
            .collect(),
        grouped: true,
        ram: false,
        vertical: false,
    };
    let geometry = compute(&input);
    let src_bands: BTreeSet<_> = input.nodes[..300]
        .iter()
        .map(|node| geometry.cards[&node.id].center().x as i64)
        .collect();
    assert!(src_bands.len() > 1, "directory must wrap across bands");
    for node in &input.nodes {
        let center = geometry.cards[&node.id].center();
        assert!(
            geometry
                .headings
                .iter()
                .any(|(label, position)| label == &node.directory
                    && (position.x - center.x).abs() < 0.01),
            "{} has no directory heading above its band",
            node.id
        );
    }
    assert_eq!(geometry.headings.len(), src_bands.len() + 1);
}

#[test]
fn wrapped_directory_headings_do_not_overlap() {
    let directory = "project/generated/platform/very_long_directory_name";
    let input = LayoutInput {
        nodes: (0..301)
            .map(|i| NodeSpec {
                id: format!("unit{i:03}"),
                directory: directory.into(),
                size: egui::vec2(120.0, 50.0),
            })
            .collect(),
        edges: vec![],
        grouped: true,
        ram: false,
        vertical: false,
    };
    let geometry = compute(&input);
    assert!(geometry.headings.len() > 1);
    let headings: Vec<_> = geometry
        .headings
        .iter()
        .map(|(label, center)| {
            egui::Rect::from_center_size(
                *center,
                egui::vec2(label.chars().count() as f32 * 8.0, 24.0),
            )
        })
        .collect();
    for (i, heading) in headings.iter().enumerate() {
        assert!(geometry.bounds.contains_rect(*heading));
        for other in &headings[i + 1..] {
            assert!(
                !heading.intersects(*other),
                "wrapped directory headings overlap"
            );
        }
    }
}

#[test]
fn adjacent_directory_bands_keep_reciprocal_edges_separately_selectable() {
    let input = LayoutInput {
        nodes: (0..101)
            .map(|i| NodeSpec {
                id: format!("unit{i:03}"),
                directory: if i == 0 { "a" } else { "b" }.into(),
                size: egui::vec2(180.0, 48.0),
            })
            .collect(),
        edges: vec![
            ("unit000".into(), "unit001".into()),
            ("unit001".into(), "unit000".into()),
        ],
        grouped: true,
        ram: false,
        vertical: false,
    };
    let geometry = compute(&input);
    for (edge, other) in [
        (&geometry.edges[0], &geometry.edges[1]),
        (&geometry.edges[1], &geometry.edges[0]),
    ] {
        assert!(route_is_clear(&edge.points, &geometry.cards));
        assert!(
            edge.points
                .iter()
                .any(|point| super::curve_distance(&other.points, *point) > 1.0),
            "reciprocal arrows share their entire selectable path"
        );
    }
}
