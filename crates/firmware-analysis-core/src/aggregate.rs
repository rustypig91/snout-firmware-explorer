use crate::{FileTree, FileUsage, Section, Symbol, Usage};
use std::collections::BTreeMap;

pub(crate) fn attribute(
    sections: &[Section],
    symbols: &mut [Symbol],
    totals: Usage,
) -> (Vec<FileUsage>, FileTree, Usage, bool) {
    symbols.sort_by(|a, b| {
        (a.section_index, a.normalized_address, &a.name).cmp(&(
            b.section_index,
            b.normalized_address,
            &b.name,
        ))
    });
    let mut files: BTreeMap<(String, String), FileUsage> = BTreeMap::new();
    let mut owned = Usage::default();
    let mut overlaps = false;
    for section in sections {
        let mut claimed_end = section.address;
        for symbol in symbols
            .iter_mut()
            .filter(|s| s.section_index == section.index)
        {
            let end = symbol.normalized_address + symbol.size;
            let unique = end.saturating_sub(claimed_end.max(symbol.normalized_address));
            overlaps |= unique != symbol.size;
            claimed_end = claimed_end.max(end);
            symbol.usage = Usage {
                flash: if section.usage.flash > 0 { unique } else { 0 },
                ram: if section.usage.ram > 0 { unique } else { 0 },
            };
            let path = symbol
                .source_file
                .as_ref()
                .or(symbol.compilation_unit.as_ref());
            if let Some(path) = path {
                let path = path.replace('\\', "/");
                let file = files
                    .entry((path.clone(), symbol.attribution.clone()))
                    .or_insert_with(|| FileUsage {
                        path,
                        attribution: symbol.attribution.clone(),
                        usage: Usage::default(),
                        symbol_count: 0,
                    });
                file.usage.flash += symbol.usage.flash;
                file.usage.ram += symbol.usage.ram;
                file.symbol_count += 1;
                owned.flash += symbol.usage.flash;
                owned.ram += symbol.usage.ram;
            }
        }
    }
    let unattributed = Usage {
        flash: totals.flash - owned.flash,
        ram: totals.ram - owned.ram,
    };
    let mut files: Vec<_> = files.into_values().collect();
    files.push(FileUsage {
        path: "[unattributed]".into(),
        attribution: "Symbols without a file, padding and bytes not covered by symbols".into(),
        usage: unattributed,
        symbol_count: symbols
            .iter()
            .filter(|s| s.source_file.is_none() && s.compilation_unit.is_none())
            .count(),
    });
    files.sort_by(|a, b| {
        b.usage
            .flash
            .cmp(&a.usage.flash)
            .then_with(|| a.path.cmp(&b.path))
    });
    let mut tree = FileTree {
        name: "Project".into(),
        ..Default::default()
    };
    for file in &files {
        let parts: Vec<_> = file
            .path
            .split('/')
            .filter(|p| !p.is_empty() && *p != ".")
            .collect();
        insert(&mut tree, &parts, file.usage);
    }
    (files, tree, unattributed, overlaps)
}

fn insert(node: &mut FileTree, path: &[&str], usage: Usage) {
    node.usage.flash += usage.flash;
    node.usage.ram += usage.ram;
    if let Some((name, rest)) = path.split_first() {
        let index = node
            .children
            .iter()
            .position(|c| c.name == *name)
            .unwrap_or_else(|| {
                node.children.push(FileTree {
                    name: name.to_string(),
                    ..Default::default()
                });
                node.children.len() - 1
            });
        insert(&mut node.children[index], rest, usage);
    }
}
