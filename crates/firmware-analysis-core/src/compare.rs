//! Stable, signed differences suitable for future CI budgets.
use crate::{Analysis, Usage};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Comparison {
    pub schema_version: u32,
    pub old_path: String,
    pub new_path: String,
    pub old: Usage,
    pub new: Usage,
    pub flash_delta: i128,
    pub ram_delta: i128,
    #[serde(default)]
    pub sections: Vec<Change>,
    pub files: Vec<Change>,
    pub symbols: Vec<Change>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Change {
    pub identity: String,
    pub status: String,
    pub old: Usage,
    pub new: Usage,
    pub flash_delta: i128,
    pub ram_delta: i128,
}

pub fn compare(old: &Analysis, new: &Analysis) -> Comparison {
    let sections = |a: &Analysis| {
        let mut map = BTreeMap::new();
        for s in a.sections.iter().filter(|s| s.allocated) {
            add(&mut map, s.name.clone(), s.usage);
        }
        map
    };
    let files = |a: &Analysis| {
        let mut map = BTreeMap::new();
        for f in &a.files {
            add(&mut map, f.path.clone(), f.usage);
        }
        map
    };
    let symbols = |a: &Analysis| {
        let mut map = BTreeMap::new();
        for s in &a.symbols {
            // Addresses intentionally excluded: normal relinking moves symbols.
            let owner = s
                .source_file
                .as_ref()
                .or(s.compilation_unit.as_ref())
                .map(String::as_str)
                .unwrap_or("?");
            add(
                &mut map,
                format!("{} | {} | {}", owner.replace('\\', "/"), s.section, s.name),
                s.usage,
            );
        }
        map
    };
    let mut warnings = vec!["Matching uses file label, section and mangled symbol name. Duplicate identities are aggregated; renamed/moved files appear as additions and removals. Attribution changes can affect file deltas.".into()];
    if old.metadata.machine != new.metadata.machine || old.metadata.bitness != new.metadata.bitness
    {
        warnings
            .push("The builds target different architectures; interpret deltas with care.".into());
    }
    if old.options != new.options {
        warnings.push("The builds were analyzed with different memory configurations; classification changes may affect totals.".into());
    }
    if old.metadata.has_dwarf != new.metadata.has_dwarf
        || old.symbols.is_empty() != new.symbols.is_empty()
    {
        warnings.push("Symbol/debug availability differs between builds; file and symbol deltas may reflect attribution changes rather than code growth.".into());
    }
    Comparison {
        schema_version: 1,
        old_path: old.path.clone(),
        new_path: new.path.clone(),
        old: old.totals,
        new: new.totals,
        flash_delta: i128::from(new.totals.flash) - i128::from(old.totals.flash),
        ram_delta: i128::from(new.totals.ram) - i128::from(old.totals.ram),
        sections: changes(sections(old), sections(new)),
        files: changes(files(old), files(new)),
        symbols: changes(symbols(old), symbols(new)),
        warnings,
    }
}

fn add(map: &mut BTreeMap<String, Usage>, key: String, usage: Usage) {
    let entry = map.entry(key).or_default();
    entry.flash += usage.flash;
    entry.ram += usage.ram;
}

fn changes(old: BTreeMap<String, Usage>, new: BTreeMap<String, Usage>) -> Vec<Change> {
    let keys: BTreeSet<_> = old.keys().chain(new.keys()).cloned().collect();
    let mut result = Vec::new();
    for identity in keys {
        let before = old.get(&identity).copied().unwrap_or_default();
        let after = new.get(&identity).copied().unwrap_or_default();
        if before == after && old.contains_key(&identity) == new.contains_key(&identity) {
            continue;
        }
        result.push(Change {
            status: if !old.contains_key(&identity) {
                "added"
            } else if !new.contains_key(&identity) {
                "removed"
            } else {
                "changed"
            }
            .into(),
            identity,
            old: before,
            new: after,
            flash_delta: i128::from(after.flash) - i128::from(before.flash),
            ram_delta: i128::from(after.ram) - i128::from(before.ram),
        });
    }
    result.sort_by_key(|c| std::cmp::Reverse(c.flash_delta.abs() + c.ram_delta.abs()));
    result
}
