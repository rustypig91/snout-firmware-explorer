use crate::{
    aggregate, Analysis, AnalysisOptions, Classification, Error, MemoryKind, MemoryRange, Metadata,
    Section, Symbol, Usage,
};
use goblin::elf::{header, program_header::PT_LOAD, section_header::*, sym, Elf};
use std::{fs, path::Path};

pub fn analyze_path(path: impl AsRef<Path>, options: &AnalysisOptions) -> Result<Analysis, Error> {
    let path = path.as_ref();
    let bytes = fs::read(path).map_err(|source| Error::Io {
        path: path.display().to_string(),
        source,
    })?;
    analyze_bytes(&bytes, &path.display().to_string(), options)
}

fn checked_end(start: u64, size: u64) -> Result<u64, Error> {
    start
        .checked_add(size)
        .ok_or_else(|| Error::Invalid("Address or file range overflows".into()))
}

fn contains(start: u64, size: u64, address: u64, len: u64) -> bool {
    address >= start
        && address
            .checked_add(len)
            .zip(start.checked_add(size))
            .is_some_and(|(a, b)| a <= b)
}

fn region_kind(options: &AnalysisOptions, address: u64, size: u64) -> Option<MemoryKind> {
    options
        .regions
        .iter()
        .find(|r| contains(r.start, r.size, address, size))
        .map(|r| r.kind)
}

fn is_mapping_symbol(machine: u16, name: &str, symbol: &sym::Sym) -> bool {
    if symbol.st_type() != sym::STT_NOTYPE
        || symbol.st_bind() != sym::STB_LOCAL
        || symbol.st_size != 0
    {
        return false;
    }
    // Arm ABI mapping symbols use an exact marker or a dot-delimited suffix.
    // A shared prefix alone does not make an ordinary symbol a mapping marker.
    let marker = name.split('.').next().unwrap_or(name);
    match machine {
        header::EM_ARM => matches!(marker, "$a" | "$d" | "$t"),
        header::EM_AARCH64 => matches!(marker, "$x" | "$d"),
        _ => false,
    }
}

/// Validate a physical memory layout before applying it to a report.
pub fn validate_options(options: &AnalysisOptions) -> Result<(), Error> {
    for (i, region) in options.regions.iter().enumerate() {
        let end = region
            .start
            .checked_add(region.size)
            .ok_or_else(|| Error::Configuration("Region address overflows".into()))?;
        if region.size == 0 {
            return Err(Error::Configuration("Region size must be nonzero".into()));
        }
        for other in &options.regions[..i] {
            if region.start < other.start + other.size && other.start < end {
                return Err(Error::Configuration(
                    "Memory regions must not overlap".into(),
                ));
            }
        }
    }
    Ok(())
}

/// Analyze a linked ELF. Classification is a bare-metal inference unless regions are supplied.
pub fn analyze_bytes(
    bytes: &[u8],
    path: &str,
    options: &AnalysisOptions,
) -> Result<Analysis, Error> {
    validate_options(options)?;
    if !bytes.starts_with(b"\x7fELF") {
        return Err(Error::Invalid(
            "Expected an ELF file containing linked firmware; linker maps describe memory regions but cannot replace firmware input".into(),
        ));
    }
    let elf = Elf::parse(bytes).map_err(|e| Error::Invalid(e.to_string()))?;
    if !matches!(elf.header.e_type, header::ET_EXEC | header::ET_DYN) {
        return Err(Error::Unsupported(
            "Use a linked executable ELF; relocatable objects do not define a final memory layout"
                .into(),
        ));
    }
    let mut warnings = vec!["Flash/RAM classification uses a bare-metal memory model. ELF permissions do not prove physical memory type; configure memory regions for unusual layouts.".into(),
        "RAM is statically allocated storage, including reservations present in the ELF. Runtime heap growth and unreserved stacks are not included.".into(),
        "Flash counts allocated section payload, excluding gaps, segment padding and programmer-specific image overhead.".into()];
    if elf.header.e_machine != header::EM_ARM {
        warnings.push("This architecture has not been validated as extensively as ARM Cortex-M; generic ELF accounting is used.".into());
    }
    if elf.header.e_type == header::ET_DYN {
        warnings.push("Dynamically linked/position-independent ELF: runtime relocation and operating-system allocations are not modeled.".into());
    }
    let mut segment_file_bytes = 0u64;
    for ph in elf.program_headers.iter().filter(|p| p.p_type == PT_LOAD) {
        if ph.p_filesz > ph.p_memsz || checked_end(ph.p_offset, ph.p_filesz)? > bytes.len() as u64 {
            return Err(Error::Invalid("Invalid load segment file range".into()));
        }
        checked_end(ph.p_vaddr, ph.p_memsz)?;
        checked_end(ph.p_paddr, ph.p_memsz)?;
        segment_file_bytes = checked_end(segment_file_bytes, ph.p_filesz)?;
    }
    let mut sections = Vec::new();
    for (index, sh) in elf.section_headers.iter().enumerate() {
        if sh.sh_type == SHT_NULL {
            continue;
        }
        checked_end(sh.sh_addr, sh.sh_size)?;
        if sh.sh_type != SHT_NOBITS && checked_end(sh.sh_offset, sh.sh_size)? > bytes.len() as u64 {
            return Err(Error::Invalid(format!(
                "Section {index} extends beyond the file"
            )));
        }
        let allocated = sh.sh_flags & u64::from(SHF_ALLOC) != 0;
        let writable = sh.sh_flags & u64::from(SHF_WRITE) != 0;
        let executable = sh.sh_flags & u64::from(SHF_EXECINSTR) != 0;
        let name = elf
            .shdr_strtab
            .get_at(sh.sh_name)
            .unwrap_or("<unnamed>")
            .to_owned();
        if allocated && sh.sh_flags & u64::from(SHF_TLS) != 0 {
            return Err(Error::Unsupported(
                "Thread-local storage requires a per-thread allocation model".into(),
            ));
        }
        let runtime_size = if allocated { sh.sh_size } else { 0 };
        let load_size = if allocated && sh.sh_type != SHT_NOBITS {
            sh.sh_size
        } else {
            0
        };
        let mut load_address = None;
        if load_size != 0 {
            for ph in elf.program_headers.iter().filter(|p| p.p_type == PT_LOAD) {
                if contains(ph.p_vaddr, ph.p_memsz, sh.sh_addr, sh.sh_size)
                    && contains(ph.p_offset, ph.p_filesz, sh.sh_offset, sh.sh_size)
                    && sh.sh_addr - ph.p_vaddr == sh.sh_offset - ph.p_offset
                {
                    let address = checked_end(ph.p_paddr, sh.sh_offset - ph.p_offset)?;
                    if load_address.is_some_and(|old| old != address) {
                        return Err(Error::Unsupported(format!(
                            "Ambiguous load addresses for {name}"
                        )));
                    }
                    load_address = Some(address);
                }
            }
            if load_address.is_none() {
                warnings.push(format!("{name}: no matching PT_LOAD segment; load address is unavailable and payload classification is inferred from section flags."));
            }
        }
        let run_region = region_kind(options, sh.sh_addr, runtime_size);
        let load_region = load_address.and_then(|a| region_kind(options, a, load_size));
        if allocated && sh.sh_size > 0 && !options.regions.is_empty() {
            if run_region.is_none() {
                warnings.push(format!("{name}: runtime range is not fully covered by configured memory regions; falling back to ELF inference. Check region boundaries and capacity."));
            }
            if load_size > 0 && load_region.is_none() {
                warnings.push(format!("{name}: load range is not fully covered by configured memory regions; Flash classification remains inferred."));
            }
        }
        let copied = load_address.is_some_and(|a| a != sh.sh_addr);
        let ram = allocated
            && match run_region {
                Some(MemoryKind::Ram) => true,
                Some(MemoryKind::Flash) => false,
                None => writable || copied || sh.sh_type == SHT_NOBITS,
            };
        let flash = load_size > 0 && load_region != Some(MemoryKind::Ram);
        let classification = if !allocated {
            Classification::NonAllocated
        } else if ram && load_size == 0 {
            Classification::NoLoadRam
        } else if ram {
            Classification::InitializedRam
        } else {
            Classification::ReadOnly
        };
        let evidence = if run_region.is_some() || load_region.is_some() {
            "Configured region(s), with ELF inference for any unmatched address"
        } else if copied {
            "ELF load address differs from runtime address; inferred copied-to-RAM section"
        } else {
            "Inferred from ELF allocation/write flags and file payload"
        }
        .to_string();
        sections.push(Section {
            index,
            name,
            address: sh.sh_addr,
            load_address,
            size: sh.sh_size,
            load_size,
            runtime_size,
            alignment: sh.sh_addralign,
            flags: sh.sh_flags,
            executable,
            writable,
            allocated,
            classification,
            usage: Usage {
                flash: if flash { load_size } else { 0 },
                ram: if ram { runtime_size } else { 0 },
            },
            evidence,
        });
    }
    if !sections.iter().any(|s| s.allocated && s.size > 0) {
        return Err(Error::Unsupported(
            "No allocated sections found. Sectionless ELF analysis is not yet supported".into(),
        ));
    }
    // Overlays need an explicit policy. Refuse misleading additive totals for now.
    let mut runtime_ranges: Vec<_> = sections
        .iter()
        .filter(|s| s.allocated && s.size > 0)
        .collect();
    runtime_ranges.sort_by_key(|s| s.address);
    for pair in runtime_ranges.windows(2) {
        if pair[0].address + pair[0].size > pair[1].address {
            return Err(Error::Unsupported(
                "Overlapping allocated sections require an overlay memory policy".into(),
            ));
        }
    }
    let mut load_ranges: Vec<_> = sections
        .iter()
        .filter(|s| s.usage.flash > 0)
        .filter_map(|s| s.load_address.map(|a| (a, s.load_size)))
        .collect();
    load_ranges.sort_unstable();
    for pair in load_ranges.windows(2) {
        if checked_end(pair[0].0, pair[0].1)? > pair[1].0 {
            return Err(Error::Unsupported(
                "Overlapping load payloads require an overlay memory policy".into(),
            ));
        }
    }
    let has_dwarf = sections
        .iter()
        .any(|s| s.name == ".debug_info" || s.name == ".zdebug_info");
    let endian = if elf.little_endian {
        gimli::RunTimeEndian::Little
    } else {
        gimli::RunTimeEndian::Big
    };
    let compressed = sections.iter().any(|s| {
        (s.name.starts_with(".debug") && s.flags & u64::from(SHF_COMPRESSED) != 0)
            || s.name.starts_with(".zdebug")
    });
    let mut source_index = crate::dwarf::SourceIndex::default();
    let dwarf_context = if has_dwarf && !compressed {
        let dwarf = gimli::Dwarf::load(|id| -> Result<_, gimli::Error> {
            let data = elf
                .section_headers
                .iter()
                .find(|s| elf.shdr_strtab.get_at(s.sh_name) == Some(id.name()))
                .and_then(|s| bytes.get(s.sh_offset as usize..(s.sh_offset + s.sh_size) as usize))
                .unwrap_or(&[]);
            Ok(gimli::EndianSlice::new(data, endian))
        });
        let dwarf = dwarf.inspect(|dwarf| {
            match crate::dwarf::SourceIndex::read(dwarf, elf.header.e_machine == header::EM_ARM) {
                Ok(index) => source_index = index,
                Err(e) => warnings.push(format!("DWARF source ownership unavailable: {e}")),
            }
        });
        match dwarf.and_then(addr2line::Context::from_dwarf) {
            Ok(context) => Some(context),
            Err(e) => {
                warnings.push(format!("Debug information could not be read: {e}"));
                None
            }
        }
    } else {
        None
    };
    if !has_dwarf {
        warnings.push("Debug information not present; source-level attribution is unavailable. Build with -g to improve attribution.".into());
    }
    if compressed {
        warnings.push(
            "Compressed DWARF is not supported yet; memory and symbol analysis remain available."
                .into(),
        );
    }
    let mut symbols = Vec::new();
    let mut compilation_unit = None;
    let mut group = 0usize;
    let mut groups = Vec::new();
    for raw in elf.syms.iter() {
        if raw.st_type() == sym::STT_FILE {
            group += 1;
            compilation_unit = elf.strtab.get_at(raw.st_name).map(str::to_owned);
            continue;
        }
        if raw.st_shndx == 0
            || !matches!(
                raw.st_type(),
                sym::STT_FUNC | sym::STT_OBJECT | sym::STT_NOTYPE
            )
        {
            continue;
        }
        let Some(section) = sections
            .iter()
            .find(|s| s.index == raw.st_shndx && s.allocated)
        else {
            continue;
        };
        let name = elf.strtab.get_at(raw.st_name).unwrap_or("");
        if name.is_empty() || is_mapping_symbol(elf.header.e_machine, name, &raw) {
            continue;
        }
        let address = if elf.header.e_machine == header::EM_ARM && raw.st_type() == sym::STT_FUNC {
            raw.st_value & !1
        } else {
            raw.st_value
        };
        if !contains(section.address, section.size, address, raw.st_size) {
            warnings.push(format!(
                "Symbol {name} extends outside its section; excluded from attribution."
            ));
            continue;
        }
        let mut source_file = None;
        let mut source_line = None;
        if raw.st_type() == sym::STT_FUNC {
            if let Some(context) = &dwarf_context {
                match context.find_location(address) {
                    Ok(Some(location)) => {
                        source_file = location.file.map(str::to_owned);
                        source_line = location.line;
                    }
                    Err(e) => warnings.push(format!("DWARF lookup failed for {name}: {e}")),
                    _ => {}
                }
            }
        }
        let unit = if raw.st_bind() == sym::STB_LOCAL {
            compilation_unit.clone()
        } else {
            None
        };
        let attribution = if source_file.is_some() {
            "DWARF function location"
        } else if unit.is_some() {
            "ELF compilation-unit label (not an object path)"
        } else {
            "Unattributed"
        }
        .to_string();
        let demangled_name = cpp_demangle::Symbol::new(name)
            .ok()
            .and_then(|s| s.demangle(&Default::default()).ok())
            .or_else(|| {
                rustc_demangle::try_demangle(name)
                    .ok()
                    .map(|s| format!("{s:#}"))
            })
            .unwrap_or_else(|| name.into());
        groups.push(
            if raw.st_bind() == sym::STB_LOCAL && compilation_unit.is_some() {
                Some(group)
            } else {
                None
            },
        );
        symbols.push(Symbol {
            name: name.into(),
            demangled_name,
            address: raw.st_value,
            normalized_address: address,
            size: raw.st_size,
            section_index: section.index,
            section: section.name.clone(),
            kind: match raw.st_type() {
                sym::STT_FUNC => "Function",
                sym::STT_OBJECT if section.writable => "Global",
                sym::STT_OBJECT => "Constant",
                _ => "Label",
            }
            .into(),
            weak: raw.st_bind() == sym::STB_WEAK,
            source_file,
            source_line,
            compilation_unit: unit,
            dwarf_compilation_unit: None,
            attribution,
            usage: Usage::default(),
        });
    }
    source_index.apply(&mut symbols, &groups);
    // Repeated STT_FILE labels do not establish a shared source identity.
    // Keep unresolved occurrences distinct, including in the GUI's file filter.
    let mut label_counts = std::collections::BTreeMap::new();
    for raw in elf.syms.iter().filter(|s| s.st_type() == sym::STT_FILE) {
        if let Some(label) = elf.strtab.get_at(raw.st_name) {
            *label_counts.entry(label).or_insert(0usize) += 1;
        }
    }
    for (symbol, group) in symbols.iter_mut().zip(&groups) {
        if symbol.source_file.is_none() {
            if let (Some(label), Some(group)) = (&symbol.compilation_unit, group) {
                if label_counts
                    .get(label.as_str())
                    .is_some_and(|count| *count > 1)
                {
                    symbol.compilation_unit = Some(format!("{label} [ELF unit {group}]"));
                }
            }
        }
    }
    if symbols.is_empty() {
        warnings.push("No defined symbols found; the ELF may be stripped. Section accounting remains available.".into());
    }
    let mut totals = Usage::default();
    let mut memory_map = Vec::new();
    for section in &sections {
        totals.flash = checked_end(totals.flash, section.usage.flash)?;
        totals.ram = checked_end(totals.ram, section.usage.ram)?;
        if section.allocated && section.size > 0 {
            memory_map.push(MemoryRange {
                name: section.name.clone(),
                address: section.address,
                size: section.runtime_size,
                space: if section.usage.ram > 0 {
                    "RAM runtime"
                } else {
                    "Read-only runtime"
                }
                .into(),
                evidence: section.evidence.clone(),
            });
            if let Some(address) = section.load_address.filter(|_| section.load_size > 0) {
                memory_map.push(MemoryRange {
                    name: format!("{} load image", section.name),
                    address,
                    size: section.load_size,
                    space: if section.usage.flash > 0 {
                        "Flash load"
                    } else {
                        "RAM load"
                    }
                    .into(),
                    evidence: "ELF PT_LOAD physical address + file offset".into(),
                });
            }
        }
    }
    memory_map.sort_by(|a, b| (&a.space, a.address).cmp(&(&b.space, b.address)));
    let (files, tree, unattributed, overlaps) =
        aggregate::attribute(&sections, &mut symbols, totals);
    if overlaps {
        warnings.push("Overlapping symbols/aliases share storage. Bytes are assigned once in deterministic address/name order; symbol sizes remain the original ELF facts.".into());
    }
    warnings.sort();
    warnings.dedup();
    Ok(Analysis {
        schema_version: 1,
        path: path.into(),
        options: options.clone(),
        metadata: Metadata {
            architecture: match elf.header.e_machine {
                header::EM_ARM => "ARM",
                header::EM_AARCH64 => "AArch64",
                header::EM_RISCV => "RISC-V",
                header::EM_X86_64 => "x86-64",
                header::EM_386 => "x86",
                _ => "Other ELF architecture",
            }
            .into(),
            machine: elf.header.e_machine,
            bitness: if elf.is_64 { 64 } else { 32 },
            endianness: if elf.little_endian { "Little" } else { "Big" }.into(),
            entry_point: elf.entry,
            file_size: bytes.len() as u64,
            segment_file_bytes,
            has_dwarf,
        },
        totals,
        unattributed,
        sections,
        symbols,
        files,
        tree,
        memory_map,
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinary_dollar_prefixed_symbols_survive_firmware_analysis() {
        let mut data = include_bytes!("../../../fixtures/cortex-m.elf").to_vec();
        let original = analyze_bytes(&data, "fixture", &AnalysisOptions::default()).unwrap();
        let original_name = "_Z12cpp_functionj";
        let name_offset = {
            let elf = Elf::parse(&data).unwrap();
            let symbol = elf
                .syms
                .iter()
                .find(|symbol| elf.strtab.get_at(symbol.st_name) == Some(original_name))
                .unwrap();
            let table = elf
                .section_headers
                .iter()
                .find(|section| section.sh_type == SHT_SYMTAB)
                .unwrap();
            elf.section_headers[table.sh_link as usize].sh_offset as usize + symbol.st_name
        };
        data[name_offset..name_offset + original_name.len()].fill(0);
        data[name_offset..name_offset + 5].copy_from_slice(b"$data");
        let report = analyze_bytes(&data, "fixture", &AnalysisOptions::default()).unwrap();
        let symbol = report
            .symbols
            .iter()
            .find(|symbol| symbol.name == "$data")
            .unwrap();
        assert!(symbol.size > 0);
        assert_eq!(report.symbols.len(), original.symbols.len());
        assert_eq!(report.totals, original.totals);
    }

    #[test]
    fn mapping_markers_are_specific_to_their_architecture_and_symbol_metadata() {
        let mapping = sym::Sym::default();
        for name in ["$a", "$a.1", "$d", "$d.pool", "$t", "$t.2"] {
            assert!(is_mapping_symbol(header::EM_ARM, name, &mapping));
            assert!(!is_mapping_symbol(header::EM_X86_64, name, &mapping));
        }
        for name in ["$x", "$x.1", "$d", "$d.pool"] {
            assert!(is_mapping_symbol(header::EM_AARCH64, name, &mapping));
        }
        for name in ["$data", "$task", "$a_function", "$xylophone"] {
            assert!(!is_mapping_symbol(header::EM_ARM, name, &mapping));
            assert!(!is_mapping_symbol(header::EM_AARCH64, name, &mapping));
        }
        assert!(!is_mapping_symbol(header::EM_ARM, "$x", &mapping));
        assert!(!is_mapping_symbol(header::EM_AARCH64, "$t", &mapping));
        for ordinary in [
            sym::Sym {
                st_info: sym::STB_GLOBAL << 4,
                ..mapping
            },
            sym::Sym {
                st_info: sym::STT_FUNC,
                ..mapping
            },
            sym::Sym {
                st_size: 4,
                ..mapping
            },
        ] {
            assert!(!is_mapping_symbol(header::EM_ARM, "$d", &ordinary));
        }
    }
}
