//! Cached sidebar data and virtualized compiler-report tree.
use super::{egui, workspace::StackSelection, Analysis};
use firmware_analysis_core::build::{ArtifactKind, BuildFolder};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::Arc,
};

pub(super) struct BrowserCache {
    pub build: Arc<BuildFolder>,
    pub analysis: Option<Arc<Analysis>>,
    pub revision: u64,
    pub reports: StackSelection,
    pub paths: Vec<PathBuf>,
    pub labels: Vec<String>,
    search_labels: Vec<String>,
    pub artifacts: [Vec<usize>; 3],
    search: Option<String>,
    nodes: Vec<Node>,
    visible: Vec<usize>,
    pub closed: BTreeSet<PathBuf>,
}
struct Node {
    path: PathBuf,
    label: String,
    search: String,
    directory: bool,
    depth: usize,
    parent: Option<usize>,
    end: usize,
    total: usize,
    checked: usize,
}
impl BrowserCache {
    pub fn new(
        build: Arc<BuildFolder>,
        analysis: Option<Arc<Analysis>>,
        revision: u64,
        reports: StackSelection,
    ) -> Self {
        let labels: Vec<_> = build
            .artifacts
            .iter()
            .map(|a| {
                a.path
                    .strip_prefix(&build.root)
                    .unwrap_or(&a.path)
                    .display()
                    .to_string()
            })
            .collect();
        let paths: Vec<_> = build
            .artifacts
            .iter()
            .filter(|a| a.kind == ArtifactKind::StackUsage)
            .map(|a| a.path.clone())
            .collect();
        let mut children = BTreeMap::<PathBuf, BTreeSet<(bool, PathBuf)>>::new();
        for path in &paths {
            let mut child = path.clone();
            let mut file = true;
            while child != build.root {
                let Some(parent) = child.parent().filter(|p| p.starts_with(&build.root)) else {
                    break;
                };
                children
                    .entry(parent.to_owned())
                    .or_default()
                    .insert((file, child.clone()));
                child = parent.to_owned();
                file = false;
            }
        }
        fn append(
            path: PathBuf,
            directory: bool,
            parent: Option<usize>,
            depth: usize,
            children: &BTreeMap<PathBuf, BTreeSet<(bool, PathBuf)>>,
            reports: &StackSelection,
            nodes: &mut Vec<Node>,
        ) {
            let index = nodes.len();
            let checked = !directory && reports.contains(&path);
            nodes.push(Node {
                label: path
                    .file_name()
                    .unwrap_or(path.as_os_str())
                    .to_string_lossy()
                    .into_owned(),
                search: path.to_string_lossy().to_lowercase(),
                path: path.clone(),
                directory,
                depth,
                parent,
                end: 0,
                total: usize::from(!directory),
                checked: usize::from(checked),
            });
            if let Some(children_here) = children.get(&path) {
                for (file, child) in children_here {
                    append(
                        child.clone(),
                        !file,
                        Some(index),
                        depth + 1,
                        children,
                        reports,
                        nodes,
                    );
                }
            }
            nodes[index].end = nodes.len();
            if let Some(parent) = parent {
                nodes[parent].total += nodes[index].total;
                nodes[parent].checked += nodes[index].checked;
            }
        }
        let mut nodes = Vec::new();
        if !paths.is_empty() {
            append(
                build.root.clone(),
                true,
                None,
                0,
                &children,
                &reports,
                &mut nodes,
            );
        }
        Self {
            search_labels: labels.iter().map(|s| s.to_lowercase()).collect(),
            labels,
            build,
            analysis,
            revision,
            reports,
            paths,
            artifacts: Default::default(),
            search: None,
            nodes,
            visible: vec![],
            closed: Default::default(),
        }
    }

    pub fn filter(&mut self, search: &str) {
        let search = search.to_lowercase();
        if self.search.as_ref() == Some(&search) {
            return;
        }
        self.artifacts = Default::default();
        for (i, artifact) in self.build.artifacts.iter().enumerate() {
            if !self.search_labels[i].contains(&search) {
                continue;
            }
            let group = match artifact.kind {
                ArtifactKind::Firmware => 0,
                ArtifactKind::Map => 1,
                ArtifactKind::StackUsage => 2,
            };
            self.artifacts[group].push(i);
        }
        self.search = Some(search);
        self.flatten();
    }

    fn flatten(&mut self) {
        let search = self.search.as_deref().unwrap_or("");
        let mut matching: Vec<_> = self
            .nodes
            .iter()
            .map(|n| !n.directory && n.search.contains(search))
            .collect();
        for i in (0..self.nodes.len()).rev() {
            if matching[i] {
                if let Some(parent) = self.nodes[i].parent {
                    matching[parent] = true;
                }
            }
        }
        self.visible.clear();
        let mut i = 0;
        while i < self.nodes.len() {
            let node = &self.nodes[i];
            if matching[i] {
                self.visible.push(i);
            }
            if !matching[i] || (search.is_empty() && self.closed.contains(&node.path)) {
                i = node.end;
            } else {
                i += 1;
            }
        }
    }

    pub fn report_ui(&mut self, ui: &mut egui::Ui) -> Option<(PathBuf, bool)> {
        let height = ui
            .spacing()
            .interact_size
            .y
            .max(ui.text_style_height(&egui::TextStyle::Body));
        let mut selection = None;
        let mut toggle = None;
        egui::ScrollArea::vertical()
            .id_salt("stack_report_rows")
            .max_height(ui.available_height().max(120.0))
            .show_rows(ui, height, self.visible.len(), |ui, range| {
                for row in range {
                    let node = &self.nodes[self.visible[row]];
                    ui.push_id(&node.path, |ui| {
                        ui.horizontal(|ui| {
                            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
                            ui.add_space((node.depth as f32 * 8.0).min(80.0));
                            if node.directory {
                                let open = !self.closed.contains(&node.path)
                                    || self.search.as_ref().is_some_and(|s| !s.is_empty());
                                if ui
                                    .small_button(if open { "\u{25bc}" } else { "\u{25b6}" })
                                    .clicked()
                                {
                                    toggle = Some(node.path.clone());
                                }
                            } else {
                                ui.add_space(18.0);
                            }
                            let mut checked = node.total > 0 && node.total == node.checked;
                            let label = if node.directory {
                                format!("{} ({}/{})", node.label, node.checked, node.total)
                            } else {
                                node.label.clone()
                            };
                            if ui
                                .add(
                                    egui::Checkbox::new(&mut checked, label).indeterminate(
                                        node.checked > 0 && node.checked < node.total,
                                    ),
                                )
                                .on_hover_text(super::display::display_path(
                                    &node.path.to_string_lossy(),
                                ))
                                .changed()
                            {
                                selection = Some((node.path.clone(), checked));
                            }
                        });
                    });
                }
            });
        if let Some(path) = toggle {
            if !self.closed.remove(&path) {
                self.closed.insert(path);
            }
            self.flatten();
        }
        selection
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use firmware_analysis_core::build::Artifact;

    fn cache(count: usize) -> BrowserCache {
        let root = std::env::temp_dir().join("snout-browser-test/build");
        let paths: Vec<_> = (0..count)
            .map(|i| root.join(format!("subsys/module_{:03}/file_{i:04}.su", i / 10)))
            .collect();
        let build = Arc::new(BuildFolder {
            root: root.clone(),
            artifacts: paths
                .iter()
                .map(|path| Artifact {
                    path: path.clone(),
                    kind: ArtifactKind::StackUsage,
                })
                .collect(),
            warnings: vec![],
        });
        BrowserCache::new(
            build,
            None,
            0,
            StackSelection {
                paths: vec![root],
                excluded: paths.first().cloned().into_iter().collect(),
                auto_directories: vec![],
            },
        )
    }

    #[test]
    fn filtering_and_collapsing_keep_ancestors_and_full_folder_counts() {
        let mut cache = cache(2000);
        cache.filter("");
        assert_eq!((cache.nodes[0].checked, cache.nodes[0].total), (1999, 2000));
        let root = cache.build.root.clone();
        cache.closed.insert(root.clone());
        cache.flatten();
        assert_eq!(cache.visible, [0]);
        cache.filter("FILE_1999");
        assert_eq!(cache.artifacts[2].len(), 1);
        assert!(cache
            .visible
            .iter()
            .any(|&i| cache.nodes[i].label == "file_1999.su"));
        assert_eq!(cache.visible.len(), 4);
        assert_eq!(cache.nodes[0].total, 2000);
        // Filtering never changes which files a directory checkbox selects.
        cache.reports.set(&root, false, &cache.paths);
        assert!(cache.paths.iter().all(|p| !cache.reports.contains(p)));
        cache.filter("");
        assert_eq!(cache.visible, [0]);
    }

    #[test]
    fn large_tree_draws_only_visible_rows_and_scrolls_to_the_last_file() {
        let mut cache = cache(2000);
        cache.filter("");
        let ctx = egui::Context::default();
        let mut frame = |scroll: bool| {
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(300.0, 500.0),
                    )),
                    events: if scroll {
                        vec![
                            egui::Event::PointerMoved(egui::pos2(150.0, 250.0)),
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
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        cache.report_ui(ui);
                    });
                },
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
        let initial = labels(&frame(false));
        assert!(initial.iter().any(|s| s == "file_0000.su"));
        assert!(initial.len() < 100, "offscreen widgets were constructed");
        let mut last = Vec::new();
        for _ in 0..20 {
            last = labels(&frame(true));
        }
        assert!(
            last.iter().any(|s| s == "file_1999.su"),
            "last file must remain reachable: {last:?}"
        );
        assert!(last.len() < 100);
    }
}
