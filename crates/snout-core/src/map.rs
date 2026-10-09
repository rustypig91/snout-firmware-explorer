//! Content-based linker-map detection and memory-region import.
use crate::{validate_options, AnalysisOptions, Error, MemoryKind, MemoryRegion};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapFormat {
    GnuLd,
    TexasCgt,
    LlvmLld,
    Unknown,
}

impl MapFormat {
    pub fn label(self) -> &'static str {
        match self {
            Self::GnuLd => "GNU ld",
            Self::TexasCgt => "Texas Instruments CGT",
            Self::LlvmLld => "LLVM lld (ELF)",
            Self::Unknown => "Unknown / unsupported",
        }
    }
}

/// Identify table syntax, independently of whether its contents are valid.
/// Filenames and extensions are deliberately not used as format evidence.
pub fn detect_map_format(text: &str) -> MapFormat {
    let mut format = MapFormat::Unknown;
    for line in text.lines() {
        let line = line.trim().trim_start_matches('\u{feff}');
        let candidate = match line {
            "Memory Configuration" => MapFormat::GnuLd,
            "MEMORY CONFIGURATION" => MapFormat::TexasCgt,
            _ if is_lld_header(line) => MapFormat::LlvmLld,
            _ => continue,
        };
        if format != MapFormat::Unknown && format != candidate {
            return MapFormat::Unknown;
        }
        format = candidate;
    }
    format
}

/// Import region capacities. Usage still comes from the ELF, not map totals.
pub fn parse_map_regions(text: &str) -> Result<AnalysisOptions, Error> {
    let text = text.trim_start_matches('\u{feff}');
    match detect_map_format(text) {
        MapFormat::GnuLd => parse_gnu_regions(text),
        MapFormat::TexasCgt => parse_ti_regions(text),
        MapFormat::LlvmLld => Err(Error::Configuration(
            "LLVM lld maps describe section placement, not physical memory capacities. Physical capacities remain unknown; --cref can supply dependencies.".into(),
        )),
        MapFormat::Unknown => Err(Error::Configuration(
            "Unknown or ambiguous linker map format; recognized formats: GNU ld, Texas Instruments CGT, and LLVM lld (ELF)".into(),
        )),
    }
}

fn hex(value: &str) -> Result<u64, Error> {
    u64::from_str_radix(value.trim_start_matches("0x").trim_start_matches("0X"), 16)
        .map_err(|_| Error::Configuration(format!("Invalid map address/length: {value}")))
}

fn parse_ti_regions(text: &str) -> Result<AnalysisOptions, Error> {
    // These toolchains report word addresses; treating their lengths as bytes
    // would silently corrupt capacities in our byte-addressed physical model.
    let banner = text
        .split("MEMORY CONFIGURATION")
        .next()
        .unwrap_or("")
        .to_ascii_uppercase();
    // Only the tool identification before "Linker" describes the target.
    // Output filenames and paths can contain names of unrelated target families.
    if banner.lines().any(|line| {
        line.split_once(" LINKER").is_some_and(|(tool, _)| {
            tool.split_whitespace().any(|word| {
                [
                    "TMS320C28",
                    "TMS320C54",
                    "TMS320C55",
                    "TMS320C2000",
                    "C2000",
                ]
                .iter()
                .any(|target| word.starts_with(target))
            })
        })
    }) {
        return Err(Error::Configuration(
            "TI CGT word-addressed targets are not supported by the byte-based memory model".into(),
        ));
    }
    let mut options = AnalysisOptions::default();
    let mut in_table = false;
    let mut attr_column = None;
    let mut columns = 0;
    for line in text.lines() {
        let line = line.trim();
        if line == "MEMORY CONFIGURATION" {
            in_table = true;
            continue;
        }
        if !in_table {
            continue;
        }
        if line.starts_with("SECTION ALLOCATION MAP")
            || line.starts_with("SEGMENT ALLOCATION MAP")
            || line.starts_with("GLOBAL SYMBOLS")
        {
            break;
        }
        if line.is_empty() || line.chars().all(|c| c == '-' || c.is_whitespace()) {
            continue;
        }
        let fields: Vec<_> = line.split_whitespace().collect();
        if fields.first() == Some(&"name") {
            if fields.get(1) != Some(&"origin") || fields.get(2) != Some(&"length") {
                return Err(Error::Configuration(format!(
                    "Unsupported TI CGT memory table header: {line}"
                )));
            }
            attr_column = fields
                .iter()
                .position(|field| matches!(*field, "attr" | "attributes"));
            columns = attr_column.ok_or_else(|| {
                Error::Configuration("TI CGT memory table has no attributes column".into())
            })?;
            if columns != 3 && !(columns == 5 && fields[3..5] == ["used", "unused"]) {
                return Err(Error::Configuration(format!(
                    "Unsupported TI CGT memory table columns: {line}"
                )));
            }
            continue;
        }
        if line.starts_with("PAGE ") {
            if fields.len() != 2
                || fields[1]
                    .strip_suffix(':')
                    .and_then(|n| n.parse::<u32>().ok())
                    .is_none()
            {
                return Err(Error::Configuration(format!(
                    "Invalid TI CGT memory page: {line}"
                )));
            }
            continue;
        }
        if attr_column.is_none() || fields.len() <= columns {
            return Err(Error::Configuration(format!(
                "Unsupported TI CGT memory row: {line}"
            )));
        }
        let start = hex(fields[1])?;
        let size = hex(fields[2])?;
        // Validate optional used/unused numeric columns without importing them.
        for value in &fields[3..columns] {
            hex(value)?;
        }
        let name = fields[0].to_string();
        let lower = name.to_ascii_lowercase();
        let attrs = fields[columns].to_ascii_lowercase();
        // TI commonly marks FLASH as RWIX too: explicit names take precedence.
        let kind = if lower.contains("flash") || lower.contains("rom") {
            MemoryKind::Flash
        } else if lower.contains("ram") || attrs.contains('w') {
            MemoryKind::Ram
        } else {
            MemoryKind::Flash
        };
        // Empty TI linker ranges reserve no physical memory.
        if size != 0 {
            options.regions.push(MemoryRegion {
                name,
                start,
                size,
                kind,
            });
        }
    }
    if options.regions.is_empty() {
        return Err(Error::Configuration(
            "No nonempty TI CGT memory regions found".into(),
        ));
    }
    validate_options(&options).map_err(|error| Error::Configuration(format!(
        "TI CGT memory layout cannot be imported: {error}. Regions must fit one non-overlapping byte-addressed space; separate overlapping PAGE spaces are unsupported."
    )))?;
    Ok(options)
}

/// Imports GNU ld's Memory Configuration table. Permission/name-based memory roles
/// remain an inference; the numeric origins and lengths come directly from the map.
fn parse_gnu_regions(text: &str) -> Result<AnalysisOptions, Error> {
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

fn is_lld_header(line: &str) -> bool {
    line.split_whitespace()
        .eq(["VMA", "LMA", "Size", "Align", "Out", "In", "Symbol"])
}

/// An output section's placement in an LLVM lld ELF map. These ranges are
/// allocations, never physical region capacities. They do not override ELF/DWARF.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LldOutputSection {
    pub name: String,
    pub vma: u64,
    pub lma: u64,
    pub size: u64,
    pub alignment: u64,
}

/// Read lld's 32/64-bit ELF placement table. Input sections, symbols, and linker
/// script commands are validated but excluded to avoid double-counting ranges.
pub fn parse_lld_sections(text: &str) -> Result<Vec<LldOutputSection>, Error> {
    if detect_map_format(text) != MapFormat::LlvmLld {
        return Err(Error::Configuration(
            "No unambiguous LLVM lld ELF map header found".into(),
        ));
    }
    let mut rows = Vec::new();
    let mut in_table = false;
    for (index, line) in text.lines().enumerate() {
        let line = line.trim_start_matches('\u{feff}');
        if is_lld_header(line) {
            in_table = true;
            continue;
        }
        if !in_table || line.trim().is_empty() {
            continue;
        }
        if line.trim() == "Cross Reference Table" {
            break;
        }
        let invalid = || {
            Error::Configuration(format!(
                "Invalid LLVM lld placement row at line {}",
                index + 1
            ))
        };
        let mut rest = line.trim_start();
        let mut numbers = [0; 4];
        for (column, value) in numbers.iter_mut().enumerate() {
            let end = rest.find(char::is_whitespace).ok_or_else(invalid)?;
            let token = &rest[..end];
            *value = if column == 3 {
                token.parse().map_err(|_| invalid())?
            } else {
                hex(token)?
            };
            rest = &rest[end..];
            if column != 3 {
                rest = rest.trim_start();
            }
        }
        let [vma, lma, size, alignment] = numbers;
        let name = rest.trim();
        if name.is_empty() {
            return Err(invalid());
        }
        // lld writes one space after Align, then 0/8/16 indentation spaces
        // for output sections / inputs / symbols. Never sum nested rows.
        if rest.starts_with(' ')
            && !rest[1..].starts_with(char::is_whitespace)
            && !name.contains('=')
        {
            // Script assignments may encode a negative dot delta as a u64.
            // Only output sections describe allocation ranges to validate.
            vma.checked_add(size).ok_or_else(invalid)?;
            lma.checked_add(size).ok_or_else(invalid)?;
            rows.push(LldOutputSection {
                name: name.into(),
                vma,
                lma,
                size,
                alignment,
            });
        }
    }
    if rows.is_empty() {
        return Err(Error::Configuration(
            "LLVM lld map has no output sections".into(),
        ));
    }
    Ok(rows)
}

/// Common output-section evidence. Addresses and sizes are byte counts; input
/// sections and symbols never appear here. A missing load address is unknown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapOutputSection {
    pub name: String,
    pub address: u64,
    pub size: u64,
    pub load_address: Option<u64>,
}

/// Parse output sections independently of format. None means the format/table
/// is unavailable; Some(empty) means a recognized empty table. Malformed
/// supported tables return errors rather than allowing weaker filename matching.
pub fn parse_map_sections(text: &str) -> Result<Option<Vec<MapOutputSection>>, Error> {
    let text = text.trim_start_matches('\u{feff}');
    match detect_map_format(text) {
        MapFormat::GnuLd => parse_gnu_sections(text),
        MapFormat::TexasCgt => parse_ti_sections(text),
        MapFormat::LlvmLld => Ok(Some(
            parse_lld_sections(text)?
                .into_iter()
                .map(|row| MapOutputSection {
                    name: row.name,
                    address: row.vma,
                    size: row.size,
                    load_address: Some(row.lma),
                })
                .collect(),
        )),
        MapFormat::Unknown => Ok(None),
    }
}

fn output_section(
    name: &str,
    address: u64,
    size: u64,
    load_address: Option<u64>,
) -> Result<MapOutputSection, Error> {
    if address.checked_add(size).is_none()
        || load_address.is_some_and(|load| load.checked_add(size).is_none())
    {
        return Err(Error::Configuration(format!(
            "Map section range overflows: {name}"
        )));
    }
    Ok(MapOutputSection {
        name: name.into(),
        address,
        size,
        load_address,
    })
}

fn gnu_section_row(name: &str, fields: &[&str]) -> Result<MapOutputSection, Error> {
    if fields.len() < 2 {
        return Err(Error::Configuration(format!(
            "Incomplete GNU ld output section: {name}"
        )));
    }
    let load_address = fields
        .iter()
        .position(|field| *field == "load")
        .map(|index| {
            if fields.get(index + 1) != Some(&"address") {
                return Err(Error::Configuration(format!(
                    "Invalid GNU ld load address: {name}"
                )));
            }
            hex(fields.get(index + 2).ok_or_else(|| {
                Error::Configuration(format!("Missing GNU ld load address: {name}"))
            })?)
        })
        .transpose()?;
    let address = hex(fields[0])?;
    // GNU ld prints a load-address annotation when LMA differs from VMA.
    // Its absence therefore establishes equal placement, not unknown LMA.
    output_section(
        name,
        address,
        hex(fields[1])?,
        Some(load_address.unwrap_or(address)),
    )
}

fn parse_gnu_sections(text: &str) -> Result<Option<Vec<MapOutputSection>>, Error> {
    let mut rows = Vec::new();
    let mut in_layout = false;
    let mut pending: Option<&str> = None;
    for line in text.lines() {
        if line.trim() == "Linker script and memory map" {
            in_layout = true;
            continue;
        }
        if !in_layout {
            continue;
        }
        if line.trim() == "Cross Reference Table" {
            break;
        }
        let fields: Vec<_> = line.split_whitespace().collect();
        if fields.is_empty() {
            continue;
        }
        if let Some(name) = pending.take() {
            // Empty linker-script output sections can have only a name and
            // wildcard rules, with no placement row (e.g. .ARM.exidx).
            if fields[0].starts_with("0x") || fields[0].starts_with("0X") {
                rows.push(gnu_section_row(name, &fields)?);
                continue;
            }
        }
        // GNU input rows and symbols are indented; output headers are not.
        if line.starts_with(char::is_whitespace) {
            continue;
        }
        let name = fields[0];
        if fields
            .get(1)
            .is_some_and(|field| field.starts_with("0x") || field.starts_with("0X"))
        {
            rows.push(gnu_section_row(name, &fields[1..])?);
        } else if fields.len() == 1 && name != "/DISCARD/" && !name.contains('(') {
            pending = Some(name);
        } else if name.starts_with('.') && fields.get(1) != Some(&"=") {
            return Err(Error::Configuration(format!(
                "Invalid GNU ld output section: {line}"
            )));
        }
    }
    if pending.is_some() {
        return Err(Error::Configuration(
            "Missing GNU ld output section placement".into(),
        ));
    }
    // Region-only maps provide no placement evidence.
    Ok((!rows.is_empty()).then_some(rows))
}

fn ti_section_row(name: &str, fields: &[&str]) -> Result<MapOutputSection, Error> {
    // TI uses a leading '*' on continuation placement rows for wrapped output
    // names. It is a formatting marker, not the page number.
    let fields = fields.strip_prefix(&["*"]).unwrap_or(fields);
    if fields.len() < 3 || fields[0].parse::<u32>().is_err() {
        return Err(Error::Configuration(format!(
            "Invalid TI CGT output section: {name}"
        )));
    }
    // PAGE is retained only as syntax evidence. Memory import already rejects
    // overlapping PAGE spaces and word-addressed targets.
    let origin = hex(fields[1])?;
    let run = fields
        .iter()
        .position(|field| *field == "RUN")
        .map(|index| {
            if fields.get(index + 1) != Some(&"ADDR") || fields.get(index + 2) != Some(&"=") {
                return Err(Error::Configuration(format!(
                    "Invalid TI CGT run address: {name}"
                )));
            }
            hex(fields.get(index + 3).ok_or_else(|| {
                Error::Configuration(format!("Missing TI CGT run address: {name}"))
            })?)
        })
        .transpose()?;
    output_section(
        name,
        run.unwrap_or(origin),
        hex(fields[2])?,
        // Origin is the load placement; RUN overrides only runtime placement.
        // Uninitialized sections have no file payload to load.
        (!fields.contains(&"UNINITIALIZED")).then_some(origin),
    )
}

fn parse_ti_sections(text: &str) -> Result<Option<Vec<MapOutputSection>>, Error> {
    // Keep unsupported word-addressed and overlapping PAGE targets out of the
    // common byte-addressed interface, including automatic matching.
    parse_ti_regions(text)?;
    let mut in_layout = false;
    let mut header = false;
    let mut pending: Option<&str> = None;
    let mut rows = Vec::new();
    for line in text.lines() {
        let fields: Vec<_> = line.split_whitespace().collect();
        if line.trim() == "SECTION ALLOCATION MAP" {
            in_layout = true;
            continue;
        }
        if !in_layout {
            continue;
        }
        if line.trim().starts_with("GLOBAL SYMBOLS")
            || line.trim().starts_with("SEGMENT ALLOCATION MAP")
            || line.trim() == "MODULE SUMMARY"
        {
            break;
        }
        if fields.starts_with(&["section", "page", "origin", "length"]) {
            header = true;
            continue;
        }
        if !header
            || fields.is_empty()
            || line.trim().chars().all(|c| c == '-' || c.is_whitespace())
        {
            continue;
        }
        if let Some(name) = pending.take() {
            rows.push(ti_section_row(name, &fields)?);
            continue;
        }
        if fields.len() == 1 {
            pending = Some(fields[0]);
            continue;
        }
        // Names such as "abc" are valid hexadecimal strings too. An output
        // row has a page/origin/length triple, unlike a nested input row.
        let output_header = fields.len() >= 4
            && fields[1].parse::<u32>().is_ok()
            && hex(fields[2]).is_ok()
            && hex(fields[3]).is_ok();
        if !output_header && hex(fields[0]).is_ok() {
            continue;
        }
        rows.push(ti_section_row(fields[0], &fields[1..])?);
    }
    if pending.is_some() {
        return Err(Error::Configuration(
            "Missing TI CGT output section placement".into(),
        ));
    }
    Ok(header.then_some(rows))
}
