//! Build folder discovery and automatic linker-map memory configuration import.
mod map_match;

pub use map_match::map_match_issues;

pub use crate::map::{detect_map_format, parse_map_regions, MapFormat};
use crate::{analyze_path, Analysis, AnalysisOptions, Error};
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
}
impl ArtifactKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Firmware => "ELF firmware",
            Self::Map => "Linker map",
            Self::StackUsage => "Stack usage",
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
                "ld" | "lds" => None,
                "json" => None,
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

impl BuildFolder {
    /// Match section placement first, then use unambiguous filename evidence
    /// only for maps without usable placement data. Conflicting maps are excluded.
    pub fn matching_map(&self, firmware: &Path) -> Option<&Path> {
        self.matching_map_with_reason(firmware)
            .map(|(path, _)| path)
    }

    fn matching_map_with_reason(&self, firmware: &Path) -> Option<(&Path, &'static str)> {
        let sections = fs::read(firmware)
            .ok()
            .and_then(|bytes| map_match::elf_sections(&bytes));
        let mut content = Vec::new();
        let mut fallback = Vec::new();
        for artifact in self
            .artifacts
            .iter()
            .filter(|artifact| artifact.kind == ArtifactKind::Map)
        {
            let evidence = sections
                .as_ref()
                .and_then(|sections| {
                    fs::read_to_string(&artifact.path).ok().map(|text| {
                        match crate::map::parse_map_sections(&text) {
                            Ok(placements) => map_match::evidence(placements.as_deref(), sections),
                            Err(_) => map_match::Evidence::Conflicts,
                        }
                    })
                })
                .unwrap_or(map_match::Evidence::Unavailable);
            match evidence {
                map_match::Evidence::Matches => content.push(artifact.path.as_path()),
                map_match::Evidence::Unavailable if filename_matches(&artifact.path, firmware) => {
                    fallback.push(artifact.path.as_path())
                }
                _ => {}
            }
        }
        if !content.is_empty() {
            if content.len() == 1 {
                return Some((
                    content[0],
                    "matched by ELF section names, addresses and sizes",
                ));
            }
            // Identical layouts can belong to different targets. Filename and
            // sibling evidence may disambiguate these, but proximity alone cannot.
            let named: Vec<_> = content
                .into_iter()
                .filter(|path| filename_matches(path, firmware))
                .collect();
            return unique_named(&named, firmware)
                .map(|path| (path, "matched by ELF sections and filename"));
        }
        unique_named(&fallback, firmware)
            .map(|path| (path, "matched by filename; section evidence unavailable"))
    }
}

fn filename_matches(map: &Path, firmware: &Path) -> bool {
    map.file_stem() == firmware.file_stem() || map.file_stem() == firmware.file_name()
}

fn unique_named<'a>(matches: &[&'a Path], firmware: &Path) -> Option<&'a Path> {
    let siblings: Vec<_> = matches
        .iter()
        .filter(|path| path.parent() == firmware.parent())
        .collect();
    if siblings.len() == 1 {
        return Some(siblings[0]);
    }
    (matches.len() == 1).then(|| matches[0])
}

pub fn analyze_build_firmware(
    build: &BuildFolder,
    path: &Path,
    override_options: Option<&AnalysisOptions>,
) -> Result<Analysis, Error> {
    let selected_map = build.matching_map_with_reason(path);
    let mut notes = Vec::new();
    let mut options = override_options.cloned().unwrap_or_default();
    if override_options.is_none() {
        if let Some((map, reason)) = selected_map {
            match fs::read_to_string(map)
                .map_err(|e| e.to_string())
                .and_then(|text| {
                    parse_map_regions(&text)
                        .map(|layout| (detect_map_format(&text), layout))
                        .map_err(|e| e.to_string())
                }) {
                Ok((format, layout)) => {
                    options = layout;
                    notes.push(format!("Memory capacities imported from {} ({}; {reason}). Flash/RAM roles are inferred from region names and attributes. Verify that the map belongs to this firmware build.", map.display(), format.label()));
                }
                Err(e) => notes.push(format!("{}: {e}; capacity remains unknown", map.display())),
            }
        } else {
            notes.push(
                "No unique matching map file found. Select a map and apply its memory regions."
                    .into(),
            );
        }
    }
    let mut analysis = analyze_path(path, &options)?;
    if let Some((map, _)) = selected_map {
        match fs::read_to_string(map) {
            Ok(text) => {
                crate::dependencies::import_map(&mut analysis, &text, &map.display().to_string())
            }
            Err(e) => analysis
                .dependencies
                .notes
                .push(format!("{}: {e}", map.display())),
        }
    }
    analysis.warnings.extend(notes);
    Ok(analysis)
}
