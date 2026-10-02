use serde::{Deserialize, Serialize};

/// Versioned, frontend-independent report. All sizes are bytes; addresses are unsigned.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Analysis {
    pub schema_version: u32,
    pub path: String,
    pub metadata: Metadata,
    pub options: AnalysisOptions,
    pub totals: Usage,
    pub unattributed: Usage,
    pub sections: Vec<Section>,
    pub symbols: Vec<Symbol>,
    pub files: Vec<FileUsage>,
    pub tree: FileTree,
    pub memory_map: Vec<MemoryRange>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Metadata {
    pub architecture: String,
    pub machine: u16,
    pub bitness: u8,
    pub endianness: String,
    pub entry_point: u64,
    pub file_size: u64,
    /// PT_LOAD file bytes, including segment padding/headers. Not the Flash total.
    pub segment_file_bytes: u64,
    pub has_dwarf: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    pub flash: u64,
    pub ram: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Classification {
    ReadOnly,
    InitializedRam,
    NoLoadRam,
    NonAllocated,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Section {
    pub index: usize,
    pub name: String,
    pub address: u64,
    pub load_address: Option<u64>,
    pub size: u64,
    pub load_size: u64,
    pub runtime_size: u64,
    pub alignment: u64,
    pub flags: u64,
    pub executable: bool,
    pub writable: bool,
    pub allocated: bool,
    pub classification: Classification,
    pub usage: Usage,
    pub evidence: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Symbol {
    pub name: String,
    pub demangled_name: String,
    /// Raw ELF symbol value (may contain the ARM Thumb bit).
    pub address: u64,
    pub normalized_address: u64,
    pub size: u64,
    pub section_index: usize,
    pub section: String,
    pub kind: String,
    pub weak: bool,
    pub source_file: Option<String>,
    pub source_line: Option<u32>,
    /// STT_FILE is a compilation-unit label, not proof of an object-file path.
    pub compilation_unit: Option<String>,
    pub attribution: String,
    /// Unique owned bytes; aliases/overlapping symbols do not double count.
    pub usage: Usage,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileUsage {
    pub path: String,
    pub attribution: String,
    pub usage: Usage,
    pub symbol_count: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FileTree {
    pub name: String,
    pub usage: Usage,
    pub children: Vec<FileTree>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryRange {
    pub name: String,
    pub address: u64,
    pub size: u64,
    pub space: String,
    pub evidence: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnalysisOptions {
    /// Optional physical memory description for unusual layouts.
    pub regions: Vec<MemoryRegion>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryRegion {
    pub name: String,
    pub start: u64,
    pub size: u64,
    pub kind: MemoryKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryKind {
    Flash,
    Ram,
}
