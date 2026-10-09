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
    let issues = placement_issues(placements, sections);
    if issues.is_empty() {
        Evidence::Matches
    } else if issues.iter().any(|issue| issue.conflict) {
        Evidence::Conflicts
    } else {
        Evidence::Incomplete
    }
}

struct Issue {
    message: String,
    conflict: bool,
}

fn placement_issues(placements: &[Placement], sections: &[Placement]) -> Vec<Issue> {
    let mut issues = Vec::new();
    for section in sections {
        let rows: Vec<_> = placements
            .iter()
            .filter(|row| row.name == section.name)
            .collect();
        let message = match rows.as_slice() {
            [] => Some((
                format!("Section {} is missing from the map.", section.name),
                false,
            )),
            [row] => {
                let mut differences = Vec::new();
                if row.address != section.address {
                    differences.push(format!(
                        "runtime address: ELF {:#x}, map {:#x}",
                        section.address, row.address
                    ));
                }
                if row.size != section.size {
                    differences.push(format!(
                        "size: ELF {} bytes, map {} bytes",
                        section.size, row.size
                    ));
                }
                if let Some((map, elf)) = row.load_address.zip(section.load_address) {
                    if map != elf {
                        differences.push(format!("load address: ELF {elf:#x}, map {map:#x}"));
                    }
                }
                (!differences.is_empty()).then(|| {
                    (
                        format!(
                            "Section {} differs ({})",
                            section.name,
                            differences.join("; ")
                        ),
                        true,
                    )
                })
            }
            _ => Some((
                format!(
                    "Section {} has multiple map entries; its placement is ambiguous.",
                    section.name
                ),
                true,
            )),
        };
        if let Some((message, conflict)) = message {
            issues.push(Issue { message, conflict });
        }
    }
    if sections.len() < 2 {
        issues.push(Issue { message: "The ELF has fewer than two nonempty allocated sections; there is insufficient evidence to confirm a match.".into(), conflict: false });
    }
    issues
}

/// Explain why a map cannot be confirmed against this ELF. Empty means that
/// all nonempty allocated sections agree under the automatic matching rules.
/// Matching placement is evidence, not proof of build ownership.
pub fn map_match_issues(elf_bytes: &[u8], map_text: &str) -> Vec<String> {
    let Some(sections) = elf_sections(elf_bytes) else {
        return vec![
            "ELF section placement could not be read; this map cannot be verified.".into(),
        ];
    };
    match crate::map::parse_map_sections(map_text) {
        Ok(Some(placements)) => placement_issues(&placements, &sections).into_iter().map(|issue| issue.message).collect(),
        Ok(None) => vec!["The map has no supported output-section placement table; its contents cannot be matched to this ELF.".into()],
        Err(error) => vec![format!("The map's section placement could not be read: {error}")],
    }
}
