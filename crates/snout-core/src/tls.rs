//! Runtime-independent ELF TLS template inspection.
use crate::{Error, TlsReport, TlsSymbol};
use goblin::elf::{program_header::PT_TLS, section_header::*, sym, Elf};

fn end(start: u64, size: u64) -> Result<u64, Error> {
    start
        .checked_add(size)
        .ok_or_else(|| Error::Invalid("TLS range overflows".into()))
}

pub(crate) fn analyze(elf: &Elf, file_len: u64) -> Result<Option<TlsReport>, Error> {
    let sections: Vec<_> = elf
        .section_headers
        .iter()
        .enumerate()
        .filter(|(_, s)| {
            s.sh_flags & u64::from(SHF_ALLOC | SHF_TLS) == u64::from(SHF_ALLOC | SHF_TLS)
        })
        .collect();
    let segments: Vec<_> = elf
        .program_headers
        .iter()
        .filter(|p| p.p_type == PT_TLS)
        .collect();
    if segments.len() > 1 {
        return Err(Error::Unsupported(
            "Multiple PT_TLS templates are not supported".into(),
        ));
    }
    if sections.is_empty() && segments.is_empty() {
        return Ok(None);
    }
    let (base, initialized, size, alignment, source) = if let Some(ph) = segments.first() {
        if ph.p_filesz > ph.p_memsz || end(ph.p_offset, ph.p_filesz)? > file_len {
            return Err(Error::Invalid("Invalid TLS segment file range".into()));
        }
        end(ph.p_vaddr, ph.p_memsz)?;
        (
            ph.p_vaddr,
            ph.p_filesz,
            ph.p_memsz,
            ph.p_align.max(1),
            "PT_TLS",
        )
    } else {
        // Some bare-metal linkers emit SHF_TLS sections without a PT_TLS header.
        let base = sections.iter().map(|(_, s)| s.sh_addr).min().unwrap();
        let mut stop = base;
        let mut file_stop = base;
        let mut alignment = 1;
        for (_, s) in &sections {
            stop = stop.max(end(s.sh_addr, s.sh_size)?);
            if s.sh_type != SHT_NOBITS {
                file_stop = file_stop.max(end(s.sh_addr, s.sh_size)?);
            }
            alignment = alignment.max(s.sh_addralign);
        }
        (
            base,
            file_stop - base,
            stop - base,
            alignment,
            "SHF_TLS sections (inferred template)",
        )
    };
    if !alignment.is_power_of_two() {
        return Err(Error::Invalid(
            "TLS alignment must be a power of two".into(),
        ));
    }
    let mut ranges = Vec::new();
    for (_, s) in &sections {
        if !s.sh_addralign.max(1).is_power_of_two() || s.sh_addralign > alignment {
            return Err(Error::Invalid(
                "TLS section alignment is incompatible with the TLS template".into(),
            ));
        }
        if s.sh_addr < base || end(s.sh_addr, s.sh_size)? > end(base, size)? {
            return Err(Error::Invalid(
                "TLS section lies outside the TLS template".into(),
            ));
        }
        if s.sh_type == SHT_NOBITS {
            if s.sh_size > 0 && s.sh_addr < end(base, initialized)? {
                return Err(Error::Invalid(
                    "Zero-initialized TLS section overlaps the TLS initialization image".into(),
                ));
            }
        } else {
            if end(s.sh_addr, s.sh_size)? > end(base, initialized)? {
                return Err(Error::Invalid(
                    "Initialized TLS section lies outside the TLS initialization image".into(),
                ));
            }
            if let Some(ph) = segments.first() {
                if s.sh_offset < ph.p_offset || s.sh_offset - ph.p_offset != s.sh_addr - base {
                    return Err(Error::Invalid(
                        "TLS section file offset disagrees with PT_TLS".into(),
                    ));
                }
            }
        }
        if s.sh_size > 0 {
            ranges.push((s.sh_addr, end(s.sh_addr, s.sh_size)?));
        }
    }
    ranges.sort_unstable();
    if ranges.windows(2).any(|p| p[0].1 > p[1].0) {
        return Err(Error::Unsupported(
            "Overlapping TLS sections require a template layout policy".into(),
        ));
    }
    let mut symbols = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for (raw, strtab) in elf
        .syms
        .iter()
        .map(|s| (s, &elf.strtab))
        .chain(elf.dynsyms.iter().map(|s| (s, &elf.dynstrtab)))
        .filter(|(s, _)| s.st_type() == sym::STT_TLS && s.st_shndx != 0)
    {
        let Some((_, section)) = sections.iter().find(|(i, _)| *i == raw.st_shndx) else {
            continue;
        };
        let name = strtab.get_at(raw.st_name).unwrap_or("");
        if name.is_empty() || raw.st_size == 0 {
            continue;
        }
        if !seen.insert((name.to_owned(), raw.st_value, raw.st_size, raw.st_shndx)) {
            continue;
        }
        // STT_TLS values are template offsets, never virtual addresses.
        let start = section.sh_addr - base;
        if raw.st_value < start || end(raw.st_value, raw.st_size)? > end(start, section.sh_size)? {
            return Err(Error::Invalid(format!(
                "TLS symbol {name} lies outside its template section"
            )));
        }
        symbols.push(TlsSymbol {
            name: name.into(),
            offset: raw.st_value,
            size: raw.st_size,
            section: elf
                .shdr_strtab
                .get_at(section.sh_name)
                .unwrap_or("<unnamed>")
                .into(),
        });
    }
    symbols.sort_by(|a, b| (a.offset, &a.name).cmp(&(b.offset, &b.name)));
    Ok(Some(TlsReport {
        source: source.into(),
        initialized_size: initialized,
        zero_initialized_size: size - initialized,
        template_size: size,
        alignment,
        total_runtime_ram: None,
        symbols,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dynamic_tls_symbols_use_the_dynamic_string_table() {
        let bytes = include_bytes!("../../../fixtures/build/gcc/cortex-m.elf");
        let mut elf = Elf::parse(bytes).unwrap();
        let raw = elf
            .syms
            .iter()
            .find(|s| s.st_type() == sym::STT_OBJECT && s.st_size > 0)
            .unwrap();
        let name = elf.strtab.get_at(raw.st_name).unwrap().to_owned();
        let section = &mut elf.section_headers[raw.st_shndx];
        section.sh_flags |= u64::from(SHF_TLS);
        let offset = raw.st_value - section.sh_addr;
        let size = raw.st_size;
        let mut symbol = raw;
        symbol.st_value = offset;
        symbol.st_info = (symbol.st_info & 0xf0) | sym::STT_TLS;
        // Model a stripped dynamic ELF without a static symbol/string table.
        // Symtab parses an actual encoded symbol to keep this test independent
        // of the string-table selection in the implementation.
        let mut encoded = [0u8; 16];
        encoded[0..4].copy_from_slice(&(symbol.st_name as u32).to_le_bytes());
        encoded[4..8].copy_from_slice(&(offset as u32).to_le_bytes());
        encoded[8..12].copy_from_slice(&(size as u32).to_le_bytes());
        encoded[12] = symbol.st_info;
        encoded[14..16].copy_from_slice(&(symbol.st_shndx as u16).to_le_bytes());
        elf.dynsyms = goblin::elf::sym::Symtab::parse(
            &encoded,
            0,
            1,
            goblin::container::Ctx::new(
                goblin::container::Container::Little,
                goblin::container::Endian::Little,
            ),
        )
        .unwrap();
        elf.dynstrtab = elf.strtab;
        elf.strtab = Default::default();
        elf.syms = Default::default();
        let tls = analyze(&elf, bytes.len() as u64).unwrap().unwrap();
        assert_eq!(tls.symbols.len(), 1);
        assert_eq!(tls.symbols[0].name, name);
        assert_eq!(tls.symbols[0].offset, offset);
        assert_eq!(tls.symbols[0].size, size);
    }
}
