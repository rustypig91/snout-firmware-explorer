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

type Placement = MapOutputSection;

pub(super) fn elf_sections(bytes: &[u8]) -> Option<Vec<Placement>> {
    let elf = Elf::parse(bytes).ok()?;
    elf.section_headers
        .iter()
        .filter(|section| section.sh_flags & u64::from(SHF_ALLOC) != 0 && section.sh_size != 0)
        .map(|section| {
            let name = elf.shdr_strtab.get_at(section.sh_name)?.to_owned();
            let load_address = crate::elf::section_load_address(&elf, section, &name).ok()?;
            Some(Placement {
                name,
                address: section.sh_addr,
                size: section.sh_size,
                load_address,
            })
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
    for section in sections {
        let rows: Vec<_> = placements
            .iter()
            .filter(|row| row.name == section.name)
            .collect();
        if rows.is_empty() {
            continue;
        }
        if rows.len() != 1
            || rows[0].address != section.address
            || rows[0].size != section.size
            || rows[0]
                .load_address
                .zip(section.load_address)
                .is_some_and(|(map, elf)| map != elf)
        {
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
