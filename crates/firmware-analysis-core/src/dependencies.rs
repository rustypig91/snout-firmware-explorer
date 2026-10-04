//! Compilation-unit graph backed by GNU ld symbol cross references, not inferred calls.
use crate::{Analysis, Symbol, Usage};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DependencyGraph {
    pub nodes: Vec<DependencyNode>,
    /// Direction: referencing unit -> defining unit. Internal references are omitted.
    pub edges: Vec<DependencyEdge>,
    pub map_path: Option<String>,
    pub notes: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DependencyNode {
    pub id: String,
    pub label: String,
    pub evidence: String,
    /// None for objects that cannot be associated with an ELF compilation unit.
    pub usage: Option<Usage>,
    pub objects: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DependencyEdge {
    pub from: String,
    pub to: String,
    pub symbols: Vec<String>,
}

fn owner(symbol: &Symbol) -> Option<(String, String, &'static str)> {
    if let Some(path) = &symbol.dwarf_compilation_unit {
        let path = path.replace('\\', "/");
        Some((format!("dwarf:{path}"), path, "DWARF compilation unit"))
    } else {
        symbol.compilation_unit.as_ref().map(|path| {
            let path = path.replace('\\', "/");
            (format!("elf:{path}"), path, "ELF compilation-unit label")
        })
    }
}

/// Source locations in headers are deliberately not used as compilation-unit identities.
pub fn units(analysis: &Analysis) -> DependencyGraph {
    let mut nodes = BTreeMap::new();
    for symbol in &analysis.symbols {
        let Some((id, label, evidence)) = owner(symbol) else {
            continue;
        };
        let node = nodes.entry(id.clone()).or_insert_with(|| DependencyNode {
            id,
            label,
            evidence: evidence.into(),
            usage: Some(Usage::default()),
            objects: vec![],
        });
        let usage = node.usage.as_mut().unwrap();
        usage.flash += symbol.usage.flash;
        usage.ram += symbol.usage.ram;
    }
    DependencyGraph {
        nodes: nodes.into_values().collect(),
        notes: vec!["Connections unavailable. Generate a GNU ld map with -Wl,-Map,app.map,--cref,--no-demangle and use it from the same firmware build. Units without connections are not proven independent.".into()],
        ..Default::default()
    }
}

struct Reference {
    symbol: String,
    definition: String,
    users: BTreeSet<String>,
}

fn references(text: &str) -> Result<Vec<Reference>, String> {
    let mut lines = text
        .lines()
        .skip_while(|line| line.trim() != "Cross Reference Table");
    if lines.next().is_none() {
        return Err("No GNU ld Cross Reference Table found; rebuild with --cref.".into());
    }
    let header = lines
        .find(|line| !line.trim().is_empty())
        .ok_or("Missing cross-reference header")?;
    if header.split_whitespace().collect::<Vec<_>>() != ["Symbol", "File"] {
        return Err("Unsupported cross-reference header".into());
    }
    let mut result: Vec<Reference> = vec![];
    for line in lines.filter(|line| !line.trim().is_empty()) {
        if line.starts_with(char::is_whitespace) {
            let entry = result
                .last_mut()
                .ok_or("Reference without a defining symbol")?;
            entry.users.insert(line.trim().replace('\\', "/"));
        } else {
            // GNU ld pads symbols to a minimum width. Long raw symbols have one separator.
            let split = line
                .find("  ")
                .or_else(|| line.find(char::is_whitespace))
                .ok_or("Cross-reference symbol has no defining file")?;
            let symbol = line[..split].trim();
            let definition = line[split..].trim();
            if symbol.is_empty() || definition.is_empty() {
                return Err("Incomplete cross-reference row".into());
            }
            result.push(Reference {
                symbol: symbol.into(),
                definition: definition.replace('\\', "/"),
                users: BTreeSet::new(),
            });
        }
    }
    Ok(result)
}

/// Object -> source-unit association requires exact symbol names and one consistent owner
/// across all matching ELF definitions. Basenames are never used to guess ownership.
pub fn from_map(analysis: &Analysis, text: &str, path: &str) -> Result<DependencyGraph, String> {
    let references = references(text)?;
    let mut graph = units(analysis);
    graph.notes.clear();
    graph.map_path = Some(path.into());
    graph.notes.push("Arrows mean linker symbol references, including data and function addresses; they are not a function call graph. Cross references may include discarded code. Map matching is not proof of build ownership.".into());
    let mut definitions: BTreeMap<&str, Vec<&Symbol>> = BTreeMap::new();
    for symbol in &analysis.symbols {
        definitions.entry(&symbol.name).or_default().push(symbol);
    }
    let mut candidates: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut objects = BTreeSet::new();
    for reference in &references {
        objects.insert(reference.definition.clone());
        objects.extend(reference.users.iter().cloned());
        if let Some(symbols) = definitions.get(reference.symbol.as_str()) {
            let owners: BTreeSet<_> = symbols.iter().map(|s| owner(s).map(|o| o.0)).collect();
            // An unknown or ambiguous definition prevents association of this object.
            let entry = candidates.entry(reference.definition.clone()).or_default();
            if owners.len() == 1 && !owners.contains(&None) {
                entry.insert(owners.into_iter().next().flatten().unwrap());
            } else {
                entry.insert(String::new());
            }
        }
    }
    let mut nodes: BTreeMap<_, _> = graph.nodes.into_iter().map(|n| (n.id.clone(), n)).collect();
    let mut object_ids = BTreeMap::new();
    let mut unresolved = 0;
    for object in objects {
        let associated = candidates
            .get(&object)
            .filter(|ids| ids.len() == 1 && !ids.contains(""))
            .and_then(|ids| ids.first())
            .cloned();
        let id = associated.unwrap_or_else(|| {
            unresolved += 1;
            let id = format!("object:{object}");
            nodes.insert(
                id.clone(),
                DependencyNode {
                    id: id.clone(),
                    label: object.clone(),
                    evidence: "Linker object; source compilation unit unresolved".into(),
                    usage: None,
                    objects: vec![],
                },
            );
            id
        });
        nodes.get_mut(&id).unwrap().objects.push(object.clone());
        object_ids.insert(object, id);
    }
    let mut edges: BTreeMap<(String, String), BTreeSet<String>> = BTreeMap::new();
    for reference in references {
        let to = &object_ids[&reference.definition];
        for user in reference.users {
            let from = &object_ids[&user];
            if from != to {
                edges
                    .entry((from.clone(), to.clone()))
                    .or_default()
                    .insert(reference.symbol.clone());
            }
        }
    }
    if unresolved > 0 {
        graph.notes.push(format!("{unresolved} objects have no unambiguous source-unit association. They remain separate nodes with unknown memory contribution."));
    }
    graph.nodes = nodes.into_values().collect();
    graph.edges = edges
        .into_iter()
        .map(|((from, to), symbols)| DependencyEdge {
            from,
            to,
            symbols: symbols.into_iter().collect(),
        })
        .collect();
    Ok(graph)
}

/// Failed imports preserve the ELF-only nodes and explain why connections are unavailable.
pub fn import_map(analysis: &mut Analysis, text: &str, path: &str) {
    analysis.dependencies = from_map(analysis, text, path).unwrap_or_else(|error| {
        let mut graph = units(analysis);
        graph.notes.push(format!("{path}: {error}"));
        graph
    });
}
