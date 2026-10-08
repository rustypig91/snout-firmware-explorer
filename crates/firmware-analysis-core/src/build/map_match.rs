//! Conservative section-placement evidence for automatic map selection.
use crate::map::MapOutputSection;
use goblin::elf::{section_header::SHF_ALLOC, Elf};

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Evidence {
    Unavailable,
    Matches,
    Conflicts,
    Incomplete,
}

type Placement = (String, u64, u64);

pub(super) fn elf_sections(bytes: &[u8]) -> Option<Vec<Placement>> {
    let elf = Elf::parse(bytes).ok()?;
    elf.section_headers
        .iter()
        .filter(|section| section.sh_flags & u64::from(SHF_ALLOC) != 0 && section.sh_size != 0)
        .map(|section| {
            Some((
                elf.shdr_strtab.get_at(section.sh_name)?.to_owned(),
                section.sh_addr,
                section.sh_size,
            ))
        })
        .collect()
}

pub(super) fn evidence(
    placements: Option<&[MapOutputSection]>,
    sections: &[Placement],
) -> Evidence {
    let Some(placements) = placements else {
        return Evidence::Unavailable;
    };
    // Never choose a stale map just because its filename matches. Duplicate
    // output names are also insufficient to identify a unique ELF section.
    let mut matched = 0;
    for (name, address, size) in sections {
        let rows: Vec<_> = placements.iter().filter(|row| &row.name == name).collect();
        if rows.is_empty() {
            continue;
        }
        if rows.len() != 1 || rows[0].address != *address || rows[0].size != *size {
            return Evidence::Conflicts;
        }
        matched += 1;
    }
    if matched >= 2 && matched == sections.len() {
        Evidence::Matches
    } else {
        Evidence::Incomplete
    }
}
