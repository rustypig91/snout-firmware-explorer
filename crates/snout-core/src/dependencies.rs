//! Compilation-unit graph backed by GNU ld / LLVM lld cross references, not inferred calls.
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
    /// Guidance for a map that could not supply connections; absent for ELF-only reports.
    #[serde(default)]
    pub connection_hint: Option<String>,
    /// Only supplied when a recognized linker supports these options.
    #[serde(default)]
    pub cross_reference_flags: Option<String>,
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

/// A weak alias can inherit a unit only when its exact function range has one
/// known owner. Distinct owners at a folded address remain ambiguous.
fn resolved_owners(analysis: &Analysis) -> Vec<Option<(String, String, &'static str)>> {
    let mut ranges: BTreeMap<_, BTreeSet<_>> = BTreeMap::new();
    for symbol in &analysis.symbols {
        if symbol.kind == "Function" && symbol.size > 0 {
            if let Some(unit) = owner(symbol) {
                ranges
                    .entry((symbol.section_index, symbol.normalized_address, symbol.size))
                    .or_default()
                    .insert(unit);
            }
        }
    }
    analysis
        .symbols
        .iter()
        .map(|symbol| {
            owner(symbol).or_else(|| {
                if !symbol.weak || symbol.kind != "Function" || symbol.size == 0 {
                    return None;
                }
                ranges
                    .get(&(symbol.section_index, symbol.normalized_address, symbol.size))
                    .filter(|units| units.len() == 1)
                    .and_then(|units| units.first())
                    .cloned()
            })
        })
        .collect()
}

/// Source locations in headers are deliberately not used as compilation-unit identities.
pub fn units(analysis: &Analysis) -> DependencyGraph {
    let mut nodes = BTreeMap::new();
    for (symbol, owner) in analysis.symbols.iter().zip(resolved_owners(analysis)) {
        let Some((id, label, evidence)) = owner else {
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
        notes: vec!["No linker cross references loaded. Load a GNU ld or LLVM lld ELF map with cross references from the same firmware build. Units without connections are not proven independent.".into()],
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
        return Err("No Cross Reference Table found.".into());
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
            // Raw symbols contain no whitespace. Split at their first separator,
            // since long symbols use a single space and paths may contain repeated spaces.
            let split = line
                .find(char::is_whitespace)
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
    if crate::map::detect_map_format(text) == crate::map::MapFormat::TexasCgt {
        return Err("TI CGT memory regions are supported, but TI cross-reference import is not yet supported.".into());
    }
    let format = crate::map::detect_map_format(text);
    let references = references(text).map_err(|error| {
        match format {
            crate::map::MapFormat::GnuLd | crate::map::MapFormat::LlvmLld =>
                format!("{} map: {error} Rebuild with --cref --no-demangle to generate symbol cross references.", format.label()),
            _ => format!("Unrecognized map format: {error} Cross-reference import supports GNU ld and LLVM lld ELF tables; select a supported map from the same build."),
        }
    })?;
    let mut graph = units(analysis);
    graph.notes.clear();
    if crate::map::detect_map_format(text) == crate::map::MapFormat::LlvmLld {
        graph.notes.push("Cross references imported from LLVM lld (ELF). ELF/DWARF symbol ownership remains authoritative.".into());
    }
    graph.map_path = Some(path.into());
    graph.notes.push("Arrows mean linker symbol references, including data and function addresses; they are not a function call graph. Cross references may include discarded code. Object-only arrows may also include unresolved weak references: GNU ld lists their users without distinguishing a defining file. Map matching is not proof of build ownership.".into());
    let mut definitions: BTreeMap<&str, BTreeSet<Option<String>>> = BTreeMap::new();
    for (symbol, owner) in analysis.symbols.iter().zip(resolved_owners(analysis)) {
        // --cref reports global symbols only. Static symbols with the same
        // name belong to a different namespace and cannot establish ownership.
        if symbol.local {
            continue;
        }
        definitions
            .entry(&symbol.name)
            .or_default()
            .insert(owner.map(|unit| unit.0));
    }
    let mut candidates: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut objects = BTreeSet::new();
    for reference in &references {
        objects.insert(reference.definition.clone());
        objects.extend(reference.users.iter().cloned());
        if let Some(owners) = definitions.get(reference.symbol.as_str()) {
            // An unknown or ambiguous definition prevents association of this object.
            let entry = candidates.entry(reference.definition.clone()).or_default();
            if owners.len() == 1 && !owners.contains(&None) {
                entry.insert(owners.first().unwrap().as_ref().unwrap().clone());
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
    let mut omitted = 0;
    for reference in references {
        let to = &object_ids[&reference.definition];
        // A source-unit arrow requires evidence that this symbol is defined in
        // the ELF. For undefined weak references GNU ld puts a caller first;
        // other symbols missing from the ELF may belong to discarded code.
        // Neither establishes a dependency on the current source unit.
        if !to.starts_with("object:") && !definitions.contains_key(reference.symbol.as_str()) {
            omitted += 1;
            continue;
        }
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
    if omitted > 0 {
        graph.notes.push(format!("{omitted} symbols without a global ELF definition were omitted from source-unit connections; they may be unresolved or discarded."));
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
    // A successful replacement map clears a warning from a previous failed import.
    analysis
        .warnings
        .retain(|warning| !warning.starts_with("Dependency graph: "));
    analysis.dependencies = from_map(analysis, text, path).unwrap_or_else(|error| {
        analysis.warnings.push(format!(
            "Dependency graph: {path}: {error} File boxes do not establish dependencies; connections are unavailable until a map with cross references is loaded."
        ));
        let mut graph = units(analysis);
        graph.notes.clear();
        graph.connection_hint = Some(error.clone());
        if matches!(crate::map::detect_map_format(text), crate::map::MapFormat::GnuLd | crate::map::MapFormat::LlvmLld) {
            graph.cross_reference_flags = Some("-Wl,--cref,--no-demangle".into());
        }
        graph.notes.push(format!("{path}: {error}"));
        graph
    });
}
