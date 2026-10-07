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
    changed: HashSet<(String, String)>,
    removed_symbol_indexes: HashMap<usize, usize>,
}

// Compare stable identities once when the baseline or report changes. ELF
// indexes are build-local bookkeeping rather than changes to an entry.
fn changed_entries(
    current: &Analysis,
    old: &Analysis,
    stack: Option<&StackReport>,
    old_stack: Option<&StackReport>,
) -> HashSet<(String, String)> {
    fn entries(
        a: &Analysis,
        stack: Option<&StackReport>,
    ) -> HashMap<(String, String), Vec<serde_json::Value>> {
        fn add<T: serde::Serialize>(
            map: &mut HashMap<(String, String), Vec<serde_json::Value>>,
            domain: &str,
            id: String,
            entry: &T,
        ) {
            let mut value = serde_json::to_value(entry).unwrap();
            if let Some(object) = value.as_object_mut() {
                object.remove("index");
                object.remove("section_index");
            }
            map.entry((domain.into(), id)).or_default().push(value);
        }
        let mut map = HashMap::new();
        for f in &a.files {
            add(&mut map, "file", f.path.clone(), f);
        }
        for s in &a.sections {
            add(&mut map, "section", s.name.clone(), s);
        }
        for s in &a.symbols {
            add(&mut map, "symbol", symbol_key(s), s);
        }
        for range in &a.memory_map {
            add(
                &mut map,
                "range",
                serde_json::to_string(&(&range.name, &range.space)).unwrap(),
                range,
            );
        }
        for region in &a.options.regions {
            let usage = firmware_analysis_core::regions::region_usage(a, region);
            let mut symbols: Vec<_> = usage
                .symbols
                .iter()
                .map(|entry| {
                    let symbol = &a.symbols[entry.symbol_index];
                    (
                        symbol_key(symbol),
                        entry.placement,
                        entry.address,
                        symbol.size,
                    )
                })
                .collect();
            symbols.sort();
            add(
                &mut map,
                "region",
                region.name.clone(),
                &(region, usage.used, usage.free, symbols),
            );
        }
        let mut connections: HashMap<&str, Vec<String>> = HashMap::new();
        for edge in &a.dependencies.edges {
            let value = serde_json::to_string(edge).unwrap();
            connections
                .entry(&edge.from)
                .or_default()
                .push(value.clone());
            connections.entry(&edge.to).or_default().push(value);
        }
        for node in &a.dependencies.nodes {
            let mut edges = connections.remove(node.id.as_str()).unwrap_or_default();
            edges.sort();
            add(&mut map, "dependency", node.id.clone(), &(node, edges));
        }
        if let Some(tls) = &a.tls {
            for s in &tls.symbols {
                add(&mut map, "tls_symbol", s.name.clone(), s);
            }
        }
        if let Some(stack) = stack {
            for e in &stack.entries {
                add(&mut map, "stack", stack_key(e), e);
            }
        }
        fn tree(
            map: &mut HashMap<(String, String), Vec<serde_json::Value>>,
            node: &FileTree,
            path: String,
        ) {
            add(map, "tree", path.clone(), &node.usage);
            for child in &node.children {
                let child_path = if path.is_empty() {
                    child.name.clone()
                } else {
                    format!("{path}/{}", child.name)
                };
                tree(map, child, child_path);
            }
        }
        tree(&mut map, &a.tree, String::new());
        // Duplicate identities have no unique match; preserve them for review.
        for values in map.values_mut() {
            values.sort_by_cached_key(|v| v.to_string());
        }
        map
    }
    let current = entries(current, stack);
    let old = entries(old, old_stack);
    let mut changed: HashSet<_> = current
        .keys()
        .chain(old.keys())
        .filter(|id| current.get(*id) != old.get(*id))
        .cloned()
        .collect();
    // Keep directory ancestors visible even when sibling changes cancel out.
    let paths: Vec<_> = changed
        .iter()
        .filter(|(domain, _)| domain == "tree" || domain == "file")
        .map(|(_, path)| path.trim_start_matches('/').to_owned())
        .collect();
    for mut path in paths {
        loop {
            changed.insert(("tree".into(), path.clone()));
            match path.rsplit_once('/') {
                Some((parent, _)) => path = parent.to_owned(),
                None => break,
            }
        }
        changed.insert(("tree".into(), String::new()));
    }
    changed
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
            changed: changed_entries(current, old, stack, old_stack),
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
    pub(super) fn diffs_active(&self) -> bool {
        self.snapshot_label().is_some()
    }
    pub(super) fn diff_visible(&self, domain: &str, id: &str) -> bool {
        !self.diffs_active()
            || self
                .baseline_display
                .as_ref()
                .is_some_and(|d| d.changed.contains(&(domain.into(), id.into())))
    }

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

#[cfg(test)]
mod tests {
    use super::*;
    use firmware_analysis_core::dependencies::DependencyEdge;

    fn fixture() -> Analysis {
        firmware_analysis_core::analyze_bytes(
            include_bytes!("../../../fixtures/build/cortex-m.elf"),
            "fixture.elf",
            &Default::default(),
        )
        .unwrap()
    }

    #[test]
    fn diff_matching_ignores_build_local_indexes_and_row_order() {
        let old = fixture();
        let mut current = old.clone();
        for section in &mut current.sections {
            section.index += 100;
        }
        for symbol in &mut current.symbols {
            symbol.section_index += 100;
        }
        current.sections.reverse();
        current.symbols.reverse();
        current.files.reverse();
        assert!(changed_entries(&current, &old, None, None).is_empty());
    }

    #[test]
    fn dependency_connection_changes_include_both_endpoints() {
        let old = fixture();
        assert!(old.dependencies.nodes.len() >= 2);
        let mut current = old.clone();
        let from = current.dependencies.nodes[0].id.clone();
        let to = current.dependencies.nodes[1].id.clone();
        current.dependencies.edges.push(DependencyEdge {
            from: from.clone(),
            to: to.clone(),
            symbols: vec!["new_reference".into()],
        });
        let changed = changed_entries(&current, &old, None, None);
        assert!(changed.contains(&("dependency".into(), from)));
        assert!(changed.contains(&("dependency".into(), to)));
        assert!(changed.iter().all(|(domain, _)| domain == "dependency"));
    }
}
