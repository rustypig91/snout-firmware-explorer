//! Baseline-only rows are presentation data, never input to analysis or snapshots.
use super::{
    snapshots::{stack_key, symbol_key},
    Analysis, Explorer, StackReport,
};
use firmware_analysis_core::{FileTree, Usage};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

pub(super) struct BaselineDisplay {
    pub analysis: Arc<Analysis>,
    pub stack: Option<Arc<StackReport>>,
    removed: HashSet<(String, String)>,
    removed_symbol_indexes: HashMap<usize, usize>,
}

fn extend<T: Clone>(
    current: &mut Vec<T>,
    old: &[T],
    domain: &str,
    identity: impl Fn(&T) -> String,
    zero: impl Fn(&mut T),
    removed: &mut HashSet<(String, String)>,
) {
    let present: HashSet<_> = current.iter().map(&identity).collect();
    for item in old.iter().filter(|item| !present.contains(&identity(item))) {
        removed.insert((domain.into(), identity(item)));
        let mut item = item.clone();
        zero(&mut item);
        current.push(item);
    }
}

fn merge_tree(current: &mut FileTree, old: &FileTree) {
    for child in &old.children {
        if let Some(existing) = current.children.iter_mut().find(|n| n.name == child.name) {
            merge_tree(existing, child);
        } else {
            let mut child = child.clone();
            fn zero(node: &mut FileTree) {
                node.usage = Usage::default();
                for child in &mut node.children {
                    zero(child);
                }
            }
            zero(&mut child);
            current.children.push(child);
        }
    }
}

impl BaselineDisplay {
    pub fn new(
        current: &Analysis,
        old: &Analysis,
        stack: Option<&StackReport>,
        old_stack: Option<&StackReport>,
    ) -> Self {
        let mut a = current.clone();
        let mut removed = HashSet::new();
        extend(
            &mut a.files,
            &old.files,
            "file",
            |f| f.path.clone(),
            |f| {
                f.usage = Usage::default();
                f.symbol_count = 0;
            },
            &mut removed,
        );
        // ELF indexes are local to a build. Give removed sections fresh indexes,
        // then map baseline symbols by their baseline section's identity.
        let mut next_index = a.sections.iter().map(|s| s.index).max().unwrap_or(0) + 1;
        let mut section_indexes = HashMap::new();
        for section in &old.sections {
            let matches: Vec<_> = current
                .sections
                .iter()
                .filter(|s| s.name == section.name)
                .collect();
            let index = match matches.as_slice() {
                [s] => s.index,
                [] => {
                    let mut s = section.clone();
                    s.index = next_index;
                    next_index += 1;
                    s.size = 0;
                    s.load_size = 0;
                    s.runtime_size = 0;
                    s.usage = Usage::default();
                    removed.insert(("section".into(), s.name.clone()));
                    let index = s.index;
                    a.sections.push(s);
                    index
                }
                // Duplicate current section names cannot be assigned a unique
                // drilldown. Keep the removed symbol in the Symbols table.
                _ => usize::MAX,
            };
            section_indexes.insert(section.index, index);
        }
        let present: HashSet<_> = current.symbols.iter().map(symbol_key).collect();
        let mut removed_symbol_indexes = HashMap::new();
        for (old_index, symbol) in old.symbols.iter().enumerate() {
            let id = symbol_key(symbol);
            if present.contains(&id) {
                continue;
            }
            removed.insert(("symbol".into(), id));
            let mut symbol = symbol.clone();
            symbol.size = 0;
            symbol.usage = Usage::default();
            symbol.section_index = section_indexes
                .get(&symbol.section_index)
                .copied()
                .unwrap_or(usize::MAX);
            removed_symbol_indexes.insert(old_index, a.symbols.len());
            a.symbols.push(symbol);
        }
        extend(
            &mut a.memory_map,
            &old.memory_map,
            "range",
            |r| serde_json::to_string(&(&r.name, &r.space)).unwrap(),
            |r| r.size = 0,
            &mut removed,
        );
        extend(
            &mut a.options.regions,
            &old.options.regions,
            "region",
            |r| r.name.clone(),
            |r| r.size = 0,
            &mut removed,
        );
        extend(
            &mut a.dependencies.nodes,
            &old.dependencies.nodes,
            "dependency",
            |n| n.id.clone(),
            |n| n.usage = Some(Usage::default()),
            &mut removed,
        );
        merge_tree(&mut a.tree, &old.tree);
        fn tree_ids(node: &FileTree, path: &str, ids: &mut HashSet<String>) {
            ids.insert(path.into());
            for child in &node.children {
                let path = if path.is_empty() {
                    child.name.clone()
                } else {
                    format!("{path}/{}", child.name)
                };
                tree_ids(child, &path, ids);
            }
        }
        let mut current_tree = HashSet::new();
        let mut old_tree = HashSet::new();
        tree_ids(&current.tree, "", &mut current_tree);
        tree_ids(&old.tree, "", &mut old_tree);
        removed.extend(
            old_tree
                .difference(&current_tree)
                .map(|id| ("tree".into(), id.clone())),
        );
        let current_roles: HashSet<_> = current
            .sections
            .iter()
            .map(super::insights::ram_role)
            .collect();
        removed.extend(
            old.sections
                .iter()
                .map(super::insights::ram_role)
                .filter(|role| !current_roles.contains(role))
                .map(|role| ("ram_role".into(), role.into())),
        );
        if let Some(old_tls) = &old.tls {
            if a.tls.is_none() {
                let mut tls = old_tls.clone();
                tls.initialized_size = 0;
                tls.zero_initialized_size = 0;
                tls.template_size = 0;
                tls.alignment = 0;
                tls.symbols.clear();
                a.tls = Some(tls);
                removed.insert(("tls".into(), String::new()));
            }
            extend(
                &mut a.tls.as_mut().unwrap().symbols,
                &old_tls.symbols,
                "tls_symbol",
                |s| s.name.clone(),
                |s| s.size = 0,
                &mut removed,
            );
        }
        let mut display_stack = stack.cloned();
        if let Some(old_stack) = old_stack {
            let report = display_stack.get_or_insert_with(|| StackReport {
                schema_version: old_stack.schema_version,
                entries: vec![],
                warnings: vec![],
                call_graph: Default::default(),
            });
            extend(
                &mut report.entries,
                &old_stack.entries,
                "stack",
                stack_key,
                |e| {
                    e.local_bytes = 0;
                    e.symbol_candidates.clear();
                },
                &mut removed,
            );
        }
        Self {
            analysis: Arc::new(a),
            stack: display_stack.map(Arc::new),
            removed,
            removed_symbol_indexes,
        }
    }
    pub fn is_removed(&self, domain: &str, id: &str) -> bool {
        self.removed.contains(&(domain.into(), id.into()))
    }
}

impl Explorer {
    pub(super) fn baseline_display_analysis(&self) -> Option<Arc<Analysis>> {
        self.baseline_display.as_ref().map(|d| d.analysis.clone())
    }
    pub(super) fn snapshot_removed(&self, domain: &str, id: &str) -> bool {
        self.snapshot_label().is_some()
            && self
                .baseline_display
                .as_ref()
                .is_some_and(|d| d.is_removed(domain, id))
    }
    pub(super) fn display_region_usage(
        &self,
        a: &Analysis,
        r: &firmware_analysis_core::MemoryRegion,
    ) -> firmware_analysis_core::regions::RegionUsage {
        let display = self
            .baseline_display
            .as_ref()
            .filter(|d| std::ptr::eq(a, d.analysis.as_ref()));
        let current = display.and(self.analysis.as_deref()).unwrap_or(a);
        let mut usage = if self.snapshot_removed("region", &r.name) {
            firmware_analysis_core::regions::RegionUsage {
                used: 0,
                free: 0,
                symbols: vec![],
            }
        } else {
            firmware_analysis_core::regions::region_usage(current, r)
        };
        if let (Some(display), Some(old)) = (display, self.snapshot_analysis()) {
            if let Some(old_region) = old
                .options
                .regions
                .iter()
                .find(|region| region.name == r.name)
            {
                for entry in firmware_analysis_core::regions::region_usage(old, old_region).symbols
                {
                    if let Some(&index) = display.removed_symbol_indexes.get(&entry.symbol_index) {
                        usage
                            .symbols
                            .push(firmware_analysis_core::regions::RegionSymbol {
                                symbol_index: index,
                                ..entry
                            });
                    }
                }
            }
        }
        usage
    }
}
