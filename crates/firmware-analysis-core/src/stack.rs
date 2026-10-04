//! Compiler reports are facts about local frames, not call-chain upper bounds.
use crate::{Analysis, Error};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StackReport {
    pub schema_version: u32,
    pub entries: Vec<StackEntry>,
    pub warnings: Vec<String>,
    pub call_graph: CallGraph,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StackEntry {
    pub report_file: String,
    pub source_file: String,
    pub source_line: u32,
    pub function: String,
    pub local_bytes: u64,
    pub qualifier: String,
    pub evidence: String,
    /// Candidates supported by name or source location; multiple results are ambiguous.
    pub symbol_candidates: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CallGraph {
    pub edges: Vec<CallEdge>,
    pub unresolved: Vec<UnresolvedCall>,
    pub complete: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallEdge {
    pub caller: String,
    pub callee: String,
    pub evidence: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnresolvedCall {
    pub function: Option<String>,
    pub reason: Uncertainty,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Uncertainty {
    IndirectCall,
    Recursion,
    InlineAssembly,
    InterruptEntry,
    MissingSymbol,
    MissingStackInformation,
    CallGraphUnavailable,
}

pub fn parse_stack_usage(text: &str, report_file: &str) -> (Vec<StackEntry>, Vec<String>) {
    let mut entries = Vec::new();
    let mut warnings = Vec::new();
    for (index, line) in text
        .lines()
        .enumerate()
        .filter(|(_, l)| !l.trim().is_empty())
    {
        let parsed = (|| {
            let mut fields = line.split('\t');
            let location = fields.next()?;
            let bytes = fields.next()?.trim().parse::<u64>().ok()?;
            let qualifier = fields.next()?.trim();
            if fields.next().is_some()
                || !matches!(qualifier, "static" | "dynamic" | "dynamic,bounded")
            {
                return None;
            }
            // Find :line:column: from the left, allowing Windows drive letters and C++ :: names.
            let parts: Vec<_> = location.split(':').collect();
            let i = (1..parts.len().saturating_sub(2)).find(|&i| {
                parts[i].parse::<u32>().is_ok() && parts[i + 1].parse::<u32>().is_ok()
            })?;
            let function = parts[i + 2..].join(":");
            if function.is_empty() {
                return None;
            }
            Some(StackEntry {
                report_file: report_file.into(),
                source_file: parts[..i].join(":"),
                source_line: parts[i].parse().ok()?,
                function,
                local_bytes: bytes,
                qualifier: qualifier.into(),
                evidence: "Compiler reported local frame; not total call-chain stack".into(),
                symbol_candidates: Vec::new(),
            })
        })();
        match parsed {
            Some(entry) => entries.push(entry),
            None => warnings.push(format!(
                "{report_file}:{}: malformed or unsupported stack-usage record",
                index + 1
            )),
        }
    }
    (entries, warnings)
}

pub fn analyze_stack(analysis: &Analysis, path: impl AsRef<Path>) -> Result<StackReport, Error> {
    let mut paths = Vec::new();
    collect(path.as_ref(), &mut paths)?;
    analyze_stack_files(analysis, paths)
}

/// Analyze the explicit reports discovered in a build folder.
pub fn analyze_stack_files(
    analysis: &Analysis,
    mut paths: Vec<PathBuf>,
) -> Result<StackReport, Error> {
    paths.sort();
    paths.dedup();
    let mut report = StackReport { schema_version: 1, entries: Vec::new(), warnings: vec!["Local stack comes from compiler reports, which must belong to this build. Dynamic frames may be unbounded. Interrupt overhead and call-chain totals are unknown.".into()],
        call_graph: CallGraph { unresolved: vec![UnresolvedCall { function: None, reason: Uncertainty::CallGraphUnavailable }], ..Default::default() } };
    for path in paths {
        let text = fs::read_to_string(&path).map_err(|source| Error::Io {
            path: path.display().to_string(),
            source,
        })?;
        let (mut entries, warnings) = parse_stack_usage(&text, &path.display().to_string());
        for entry in &mut entries {
            let functions: Vec<_> = analysis
                .symbols
                .iter()
                .filter(|s| s.kind == "Function")
                // A known conflicting source path rules out same-name functions in other units.
                .filter(|s| {
                    s.source_file
                        .as_ref()
                        .is_none_or(|p| source_matches(p, &entry.source_file))
                })
                .collect();
            let mut candidates: Vec<_> = functions
                .iter()
                .copied()
                .filter(|s| s.name == entry.function || s.demangled_name == entry.function)
                .collect();
            let mut method = "exact symbol name";
            if candidates.is_empty() {
                candidates = functions
                    .iter()
                    .copied()
                    .filter(|s| {
                        s.source_line == Some(entry.source_line)
                            && s.source_file
                                .as_ref()
                                .is_some_and(|p| source_matches(p, &entry.source_file))
                    })
                    .collect();
                method = "DWARF source file and line";
            }
            if candidates.is_empty() && !entry.function.contains('(') {
                candidates = functions
                    .iter()
                    .copied()
                    .filter(|s| {
                        s.demangled_name
                            .split_once('(')
                            .is_some_and(|(name, _)| name == entry.function)
                    })
                    .collect();
                method = "demangled function name without parameters";
            }
            // Prefer source evidence when duplicate local names or overloads exist.
            let located: Vec<_> = candidates
                .iter()
                .copied()
                .filter(|s| {
                    s.source_line == Some(entry.source_line)
                        && s.source_file
                            .as_ref()
                            .is_some_and(|p| source_matches(p, &entry.source_file))
                })
                .collect();
            if !located.is_empty() {
                candidates = located;
                if method != "DWARF source file and line" {
                    method = "symbol name and DWARF source file/line";
                }
            }
            if !candidates.is_empty() {
                entry
                    .evidence
                    .push_str(&format!("; ELF candidates matched by {method}"));
            }
            entry.symbol_candidates = candidates
                .into_iter()
                .map(|s| format!("{} @ {:#x}", s.name, s.address))
                .collect();
        }
        report.entries.extend(entries);
        report.warnings.extend(warnings);
    }
    if report.entries.is_empty() {
        report.warnings.push("No stack usage information found. Build with -fstack-usage and select the resulting .su files or build directory.".into());
    }
    report
        .entries
        .sort_by_key(|e| std::cmp::Reverse(e.local_bytes));
    Ok(report)
}

/// Debug paths may be absolute on the build machine while .su paths are relative.
/// Compare whole components and separators, never just arbitrary string suffixes.
fn source_matches(a: &str, b: &str) -> bool {
    let a = a.replace('\\', "/");
    let b = b.replace('\\', "/");
    let parts = |p: &str| {
        p.split('/')
            .filter(|c| !c.is_empty() && *c != ".")
            .map(str::to_owned)
            .collect::<Vec<_>>()
    };
    let a = parts(&a);
    let b = parts(&b);
    !a.is_empty() && !b.is_empty() && (a.ends_with(&b) || b.ends_with(&a))
}

fn collect(path: &Path, files: &mut Vec<PathBuf>) -> Result<(), Error> {
    let meta = fs::symlink_metadata(path).map_err(|source| Error::Io {
        path: path.display().to_string(),
        source,
    })?;
    if meta.file_type().is_symlink() {
        return Ok(());
    }
    if meta.is_dir() {
        for entry in fs::read_dir(path).map_err(|source| Error::Io {
            path: path.display().to_string(),
            source,
        })? {
            let entry = entry.map_err(|source| Error::Io {
                path: path.display().to_string(),
                source,
            })?;
            collect(&entry.path(), files)?;
        }
    } else if path.extension().is_some_and(|s| s == "su") {
        files.push(path.to_owned());
    }
    Ok(())
}
