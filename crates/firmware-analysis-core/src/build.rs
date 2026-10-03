//! Build folder discovery and GNU ld memory configuration import.
use crate::{
    analyze_path, validate_options, Analysis, AnalysisOptions, Error, MemoryKind, MemoryRegion,
};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtifactKind {
    Firmware,
    Map,
    StackUsage,
    LinkerScript,
    MemoryLayout,
}
impl ArtifactKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Firmware => "ELF firmware",
            Self::Map => "Linker map",
            Self::StackUsage => "Stack usage",
            Self::LinkerScript => "Linker script",
            Self::MemoryLayout => "Memory layout",
        }
    }
}
#[derive(Debug, Clone)]
pub struct Artifact {
    pub path: PathBuf,
    pub kind: ArtifactKind,
}
#[derive(Debug, Clone)]
pub struct BuildFolder {
    pub root: PathBuf,
    pub artifacts: Vec<Artifact>,
    pub warnings: Vec<String>,
}

pub fn scan_folder(root: impl AsRef<Path>) -> Result<BuildFolder, Error> {
    let root = root.as_ref();
    if !root.is_dir() {
        return Err(Error::Invalid(
            "Select a build folder, not an individual file".into(),
        ));
    }
    let root = fs::canonicalize(root).map_err(|source| Error::Io {
        path: root.display().to_string(),
        source,
    })?;
    let mut build = BuildFolder {
        root: root.clone(),
        artifacts: Vec::new(),
        warnings: Vec::new(),
    };
    let mut pending = vec![root];
    while let Some(dir) = pending.pop() {
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(e) => {
                build.warnings.push(format!("{}: {e}", dir.display()));
                continue;
            }
        };
        for entry in entries {
            let entry = match entry {
                Ok(e) => e,
                Err(e) => {
                    build.warnings.push(e.to_string());
                    continue;
                }
            };
            let path = entry.path();
            let kind = match entry.file_type() {
                Ok(k) => k,
                Err(e) => {
                    build.warnings.push(format!("{}: {e}", path.display()));
                    continue;
                }
            };
            if kind.is_symlink() {
                continue;
            }
            if kind.is_dir() {
                pending.push(path);
                continue;
            }
            if !kind.is_file() {
                continue;
            }
            let extension = path
                .extension()
                .unwrap_or_default()
                .to_string_lossy()
                .to_ascii_lowercase();
            let artifact_kind = match extension.as_str() {
                "map" => Some(ArtifactKind::Map),
                "su" => Some(ArtifactKind::StackUsage),
                "ld" | "lds" => Some(ArtifactKind::LinkerScript),
                "json" => fs::read(&path)
                    .ok()
                    .and_then(|b| serde_json_layout(&b))
                    .map(|_| ArtifactKind::MemoryLayout),
                _ => {
                    let mut header = [0; 18];
                    match fs::File::open(&path).and_then(|mut f| f.read(&mut header)) {
                        Ok(n) if n >= 18 && &header[..4] == b"\x7fELF" => {
                            let elf_type = if header[5] == 2 {
                                u16::from_be_bytes([header[16], header[17]])
                            } else {
                                u16::from_le_bytes([header[16], header[17]])
                            };
                            matches!(elf_type, 2 | 3).then_some(ArtifactKind::Firmware)
                        }
                        Err(e) => {
                            build.warnings.push(format!("{}: {e}", path.display()));
                            None
                        }
                        _ => None,
                    }
                }
            };
            if let Some(kind) = artifact_kind {
                build.artifacts.push(Artifact { path, kind });
            }
        }
    }
    build.artifacts.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(build)
}

fn serde_json_layout(bytes: &[u8]) -> Option<AnalysisOptions> {
    let options: AnalysisOptions = serde_json::from_slice(bytes).ok()?;
    (!options.regions.is_empty() && validate_options(&options).is_ok()).then_some(options)
}

impl BuildFolder {
    /// Prefer a sibling map with the same stem; otherwise require a unique same-stem map.
    pub fn matching_map(&self, firmware: &Path) -> Option<&Path> {
        let matches: Vec<_> = self
            .artifacts
            .iter()
            .filter(|a| {
                a.kind == ArtifactKind::Map
                    && (a.path.file_stem() == firmware.file_stem()
                        || a.path.file_stem() == firmware.file_name())
            })
            .collect();
        let siblings: Vec<_> = matches
            .iter()
            .filter(|a| a.path.parent() == firmware.parent())
            .collect();
        if siblings.len() == 1 {
            return Some(&siblings[0].path);
        }
        (matches.len() == 1).then(|| matches[0].path.as_path())
    }
}

/// Imports GNU ld's Memory Configuration table. Permission/name-based memory roles
/// remain an inference; the numeric origins and lengths come directly from the map.
pub fn parse_map_regions(text: &str) -> Result<AnalysisOptions, Error> {
    let mut in_table = false;
    let mut options = AnalysisOptions::default();
    for line in text.lines() {
        let line = line.trim();
        if line == "Memory Configuration" {
            in_table = true;
            continue;
        }
        if !in_table {
            continue;
        }
        if line.starts_with("Linker script and memory map") {
            break;
        }
        if line.is_empty() {
            continue;
        }
        let fields: Vec<_> = line.split_whitespace().collect();
        if matches!(fields.first(), Some(&"*default*" | &"Name")) {
            continue;
        }
        if fields.len() < 3 {
            return Err(Error::Configuration(format!(
                "Unsupported memory map row: {line}"
            )));
        }
        let hex = |s: &str| {
            u64::from_str_radix(s.trim_start_matches("0x").trim_start_matches("0X"), 16)
                .map_err(|_| Error::Configuration(format!("Invalid map address/length: {s}")))
        };
        let name = fields[0].to_string();
        let attrs = fields
            .get(3)
            .copied()
            .unwrap_or("")
            .split('!')
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        let kind = if attrs.contains('w') || name.to_ascii_lowercase().contains("ram") {
            MemoryKind::Ram
        } else {
            MemoryKind::Flash
        };
        options.regions.push(MemoryRegion {
            name,
            start: hex(fields[1])?,
            size: hex(fields[2])?,
            kind,
        });
    }
    if options.regions.is_empty() {
        return Err(Error::Configuration(
            "No supported GNU ld Memory Configuration table found".into(),
        ));
    }
    validate_options(&options)?;
    Ok(options)
}

pub fn analyze_build_firmware(
    build: &BuildFolder,
    path: &Path,
    override_options: Option<&AnalysisOptions>,
) -> Result<Analysis, Error> {
    let mut notes = Vec::new();
    let mut options = override_options.cloned().unwrap_or_default();
    if override_options.is_none() {
        if let Some(map) = build.matching_map(path) {
            match fs::read_to_string(map)
                .map_err(|e| e.to_string())
                .and_then(|text| parse_map_regions(&text).map_err(|e| e.to_string()))
            {
                Ok(layout) => {
                    options = layout;
                    notes.push(format!("Memory capacities imported from {} (matched by filename). Flash/RAM roles are inferred from region names and attributes. Verify that the map belongs to this firmware build.", map.display()));
                }
                Err(e) => notes.push(format!("{}: {e}; capacity remains unknown", map.display())),
            }
        } else {
            notes.push("No unique matching map file found. Select a map and apply its memory regions, or load a memory layout.".into());
        }
    }
    let mut analysis = analyze_path(path, &options)?;
    analysis.warnings.extend(notes);
    Ok(analysis)
}
