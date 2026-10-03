//! Conservative source ownership: match definitions by address and symbol name.
use crate::Symbol;
use gimli::{AttributeValue, Dwarf, Reader, Unit};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone)]
struct Definition {
    name: String,
    unit: usize,
    unit_path: String,
    source: String,
    line: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn symbol(name: &str, address: u64) -> Symbol {
        Symbol {
            name: name.into(),
            demangled_name: name.into(),
            address,
            normalized_address: address,
            size: 4,
            section_index: 1,
            section: ".data".into(),
            kind: "Global".into(),
            weak: false,
            source_file: None,
            source_line: None,
            compilation_unit: Some("main.c".into()),
            attribution: "ELF compilation-unit label (not an object path)".into(),
            usage: Default::default(),
        }
    }

    fn definition(name: &str, unit: usize, path: &str) -> Definition {
        Definition {
            name: name.into(),
            unit,
            unit_path: path.into(),
            source: path.into(),
            line: Some(1),
        }
    }

    #[test]
    fn same_basename_units_link_only_through_matching_symbols() {
        let mut index = SourceIndex::default();
        index
            .definitions
            .insert((100, false), vec![definition("anchor", 1, "/one/main.c")]);
        index
            .definitions
            .insert((200, false), vec![definition("anchor", 2, "/two/main.c")]);
        let mut symbols = vec![
            symbol("anchor", 100),
            symbol("other", 104),
            symbol("anchor", 200),
            symbol("other", 204),
            symbol("unknown", 300),
            symbol("wrong_name", 100),
        ];
        index.apply(
            &mut symbols,
            &[Some(1), Some(1), Some(2), Some(2), Some(3), Some(4)],
        );
        assert_eq!(symbols[1].source_file.as_deref(), Some("/one/main.c"));
        assert_eq!(symbols[3].source_file.as_deref(), Some("/two/main.c"));
        assert!(symbols[4].source_file.is_none());
        assert!(symbols[5].source_file.is_none());
    }

    #[test]
    fn ambiguous_or_conflicting_unit_links_are_not_used() {
        let mut index = SourceIndex::default();
        index.definitions.insert(
            (100, false),
            vec![
                definition("anchor", 1, "/one/main.c"),
                definition("anchor", 2, "/two/main.c"),
            ],
        );
        let mut symbols = vec![symbol("anchor", 100), symbol("unknown", 104)];
        index.apply(&mut symbols, &[Some(1), Some(1)]);
        assert!(symbols.iter().all(|s| s.source_file.is_none()));
        index
            .definitions
            .insert((100, false), vec![definition("anchor", 1, "/one/main.c")]);
        index
            .definitions
            .insert((200, false), vec![definition("second", 2, "/two/main.c")]);
        let mut symbols = vec![
            symbol("anchor", 100),
            symbol("second", 200),
            symbol("unknown", 300),
        ];
        index.apply(&mut symbols, &[Some(1), Some(1), Some(1)]);
        assert!(symbols[2].source_file.is_none());
    }

    #[test]
    fn definition_does_not_inherit_a_line_from_another_source_file() {
        let mut index = SourceIndex::default();
        let mut owner = definition("function", 1, "/project/main.c");
        owner.line = None;
        index.definitions.insert((100, true), vec![owner]);
        for (source, expected_line) in [("/project/header.h", None), ("/project/main.c", Some(42))]
        {
            let mut function = symbol("function", 100);
            function.kind = "Function".into();
            function.source_file = Some(source.into());
            function.source_line = Some(42);
            index.apply(std::slice::from_mut(&mut function), &[None]);
            assert_eq!(function.source_file.as_deref(), Some("/project/main.c"));
            assert_eq!(function.source_line, expected_line);
        }
    }

    #[test]
    fn paths_are_resolved_independently_of_the_host_os() {
        assert_eq!(join("C:\\project", "src/main.c"), "C:/project/src/main.c");
        assert_eq!(join("/project", "./src/main.c"), "/project/src/main.c");
        assert_eq!(join("/project", "/other/main.c"), "/other/main.c");
    }
}

#[derive(Debug, Default)]
pub(crate) struct SourceIndex {
    definitions: BTreeMap<(u64, bool), Vec<Definition>>,
}

fn string<R: Reader>(dwarf: &Dwarf<R>, unit: &Unit<R>, value: AttributeValue<R>) -> Option<String> {
    dwarf
        .attr_string(unit, value)
        .ok()?
        .to_string_lossy()
        .ok()
        .map(|s| s.into_owned())
}

fn join(base: &str, path: &str) -> String {
    let path = path.replace('\\', "/");
    let absolute = path.starts_with('/') || path.as_bytes().get(1) == Some(&b':');
    let joined = if absolute || base.is_empty() {
        path
    } else {
        format!("{}/{path}", base.replace('\\', "/").trim_end_matches('/'))
    };
    // Normalize dot components without consulting the host filesystem.
    joined
        .split('/')
        .filter(|part| *part != ".")
        .collect::<Vec<_>>()
        .join("/")
}

impl SourceIndex {
    pub(crate) fn read<R: Reader>(dwarf: &Dwarf<R>, arm: bool) -> Result<Self, gimli::Error> {
        let mut result = Self::default();
        let mut headers = dwarf.units();
        let mut unit_id = 0;
        while let Some(header) = headers.next()? {
            let unit = dwarf.unit(header)?;
            let base = unit
                .comp_dir
                .as_ref()
                .and_then(|s| s.to_string_lossy().ok())
                .map(|s| s.into_owned())
                .unwrap_or_default();
            let Some(name) = unit.name.as_ref().and_then(|s| s.to_string_lossy().ok()) else {
                continue;
            };
            let unit_path = join(&base, &name);
            unit_id += 1;
            let mut entries = unit.entries();
            while let Some((_, entry)) = entries.next_dfs()? {
                let function = entry.tag() == gimli::DW_TAG_subprogram;
                if !function && entry.tag() != gimli::DW_TAG_variable {
                    continue;
                }
                // Declarations, frame-relative locals, location lists, TLS and
                // complex expressions are deliberately not guessed.
                let address = if function {
                    match entry.attr_value(gimli::DW_AT_low_pc)? {
                        Some(AttributeValue::Addr(a)) => Some(if arm { a & !1 } else { a }),
                        _ => None,
                    }
                } else {
                    match entry.attr_value(gimli::DW_AT_location)? {
                        Some(AttributeValue::Exprloc(expr)) => {
                            let mut ops = expr.operations(unit.encoding());
                            match (ops.next()?, ops.next()?) {
                                (Some(gimli::Operation::Address { address }), None) => {
                                    Some(address)
                                }
                                _ => None,
                            }
                        }
                        _ => None,
                    }
                };
                let Some(address) = address else {
                    continue;
                };
                let name = entry
                    .attr_value(gimli::DW_AT_linkage_name)?
                    .or(entry.attr_value(gimli::DW_AT_MIPS_linkage_name)?)
                    .or(entry.attr_value(gimli::DW_AT_name)?);
                let Some(name) = name.and_then(|v| string(dwarf, &unit, v)) else {
                    continue;
                };
                let mut source = unit_path.clone();
                if let Some(AttributeValue::FileIndex(index)) =
                    entry.attr_value(gimli::DW_AT_decl_file)?
                {
                    if let Some(program) = &unit.line_program {
                        let header = program.header();
                        if let Some(file) = header.file(index) {
                            if let Some(path) = string(dwarf, &unit, file.path_name()) {
                                let directory = file
                                    .directory(header)
                                    .and_then(|v| string(dwarf, &unit, v))
                                    .unwrap_or_default();
                                source = join(&join(&base, &directory), &path);
                            }
                        }
                    }
                }
                let line = entry
                    .attr_value(gimli::DW_AT_decl_line)?
                    .and_then(|v| v.udata_value())
                    .and_then(|v| v.try_into().ok());
                let definition = Definition {
                    name,
                    unit: unit_id,
                    unit_path: unit_path.clone(),
                    source,
                    line,
                };
                result
                    .definitions
                    .entry((address, function))
                    .or_default()
                    .push(definition);
            }
        }
        Ok(result)
    }

    /// `groups` are separate STT_FILE occurrences, never basename matches.
    pub(crate) fn apply(&self, symbols: &mut [Symbol], groups: &[Option<usize>]) {
        let mut links: BTreeMap<usize, BTreeSet<(usize, String)>> = BTreeMap::new();
        for (symbol, group) in symbols.iter_mut().zip(groups) {
            let function = symbol.kind == "Function";
            let candidates: Vec<_> = self
                .definitions
                .get(&(symbol.normalized_address, function))
                .into_iter()
                .flatten()
                .filter(|d| d.name == symbol.name)
                .collect();
            if let Some(group) = group {
                for d in &candidates {
                    links
                        .entry(*group)
                        .or_default()
                        .insert((d.unit, d.unit_path.clone()));
                }
            }
            // Identical addresses can be aliases or folded code. Require one definition.
            if let [definition] = candidates.as_slice() {
                // An address lookup may refer to a different file (for example,
                // inlined code). Its line is only usable with that same file.
                symbol.source_line = definition.line.or_else(|| {
                    (symbol.source_file.as_deref() == Some(definition.source.as_str()))
                        .then_some(symbol.source_line)
                        .flatten()
                });
                symbol.source_file = Some(definition.source.clone());
                symbol.attribution = if function {
                    "DWARF function definition"
                } else {
                    "DWARF variable definition"
                }
                .into();
            } else if candidates.len() > 1 {
                symbol.source_file = None;
                symbol.source_line = None;
                symbol.attribution =
                    "Ambiguous DWARF ownership; ELF compilation-unit fallback where available"
                        .into();
            }
        }
        for (symbol, group) in symbols.iter_mut().zip(groups) {
            if symbol.source_file.is_some() {
                continue;
            }
            let Some(link) = group
                .and_then(|g| links.get(&g))
                .filter(|set| set.len() == 1)
                .and_then(|set| set.first())
            else {
                continue;
            };
            symbol.source_file = Some(link.1.clone());
            symbol.attribution = "DWARF compilation unit linked by symbol name and address".into();
        }
    }
}
