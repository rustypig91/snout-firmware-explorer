//! Sparse firmware contents. Gaps and reservations are unknown, never invented filler.
use crate::{MemoryKind, Section};
use goblin::elf::Elf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryByte {
    File(u8),
    /// Conventional BSS startup behavior; the ELF does not prove startup code clears it.
    InferredZero,
    Unknown,
}
impl MemoryByte {
    pub fn value(self) -> Option<u8> {
        match self {
            Self::File(value) => Some(value),
            Self::InferredZero => Some(0),
            Self::Unknown => None,
        }
    }
}
#[derive(Debug, Clone)]
pub struct MemoryBlock {
    pub address: u64,
    pub size: u64,
    pub kind: MemoryKind,
    pub section_index: usize,
    pub data: Vec<u8>,
    pub inferred_zero: bool,
}
#[derive(Debug, Clone, Default)]
pub struct MemoryImage {
    /// Nonoverlapping within each space, sorted by space (Flash, RAM) then address.
    pub blocks: Vec<MemoryBlock>,
}
impl MemoryImage {
    pub(crate) fn from_elf(elf: &Elf<'_>, bytes: &[u8], sections: &[Section]) -> Self {
        let mut blocks = Vec::new();
        for section in sections.iter().filter(|s| s.allocated && s.size > 0) {
            let raw = &elf.section_headers[section.index];
            let data = if section.load_size > 0 {
                bytes[raw.sh_offset as usize..(raw.sh_offset + raw.sh_size) as usize].to_vec()
            } else {
                Vec::new()
            };
            if section.usage.flash > 0 {
                // Without a matching load segment we cannot locate the physical payload.
                if let Some(address) = section.load_address {
                    blocks.push(MemoryBlock {
                        address,
                        size: section.size,
                        kind: MemoryKind::Flash,
                        section_index: section.index,
                        data: data.clone(),
                        inferred_zero: false,
                    });
                }
            }
            if section.usage.ram > 0 {
                let bss = [".bss", ".sbss", ".dynbss"].iter().any(|name| {
                    section.name == *name || section.name.starts_with(&format!("{name}."))
                });
                blocks.push(MemoryBlock {
                    address: section.address,
                    size: section.size,
                    kind: MemoryKind::Ram,
                    section_index: section.index,
                    data,
                    inferred_zero: section.load_size == 0 && bss,
                });
            }
        }
        blocks.sort_by_key(|b| (b.kind == MemoryKind::Ram, b.address));
        Self { blocks }
    }
    pub fn byte(&self, kind: MemoryKind, address: u64) -> MemoryByte {
        let key = (kind == MemoryKind::Ram, address);
        let end = self
            .blocks
            .partition_point(|b| (b.kind == MemoryKind::Ram, b.address) <= key);
        if let Some(block) = end
            .checked_sub(1)
            .and_then(|i| self.blocks.get(i))
            .filter(|b| b.kind == kind)
        {
            let offset = address - block.address;
            if offset < block.size {
                return block
                    .data
                    .get(offset as usize)
                    .copied()
                    .map(MemoryByte::File)
                    .unwrap_or(if block.inferred_zero {
                        MemoryByte::InferredZero
                    } else {
                        MemoryByte::Unknown
                    });
            }
        }
        MemoryByte::Unknown
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn startup_contents_keep_copied_code_data_bss_and_reservations_distinct() {
        let report = crate::analyze_bytes(
            include_bytes!("../../../fixtures/build/gcc/cortex-m.elf"),
            "fixture",
            &Default::default(),
        )
        .unwrap();
        let image = report.memory_image.as_ref().unwrap();
        for name in [".data", ".ram_code"] {
            let section = report.sections.iter().find(|s| s.name == name).unwrap();
            for offset in 0..section.size {
                assert!(matches!(
                    image.byte(MemoryKind::Ram, section.address + offset),
                    MemoryByte::File(_)
                ));
                assert_eq!(
                    image.byte(MemoryKind::Ram, section.address + offset),
                    image.byte(MemoryKind::Flash, section.load_address.unwrap() + offset)
                );
            }
        }
        let bss = report.sections.iter().find(|s| s.name == ".bss").unwrap();
        assert_eq!(
            image.byte(MemoryKind::Ram, bss.address),
            MemoryByte::InferredZero
        );
        let reserved = report
            .sections
            .iter()
            .find(|s| s.name == ".reserved")
            .unwrap();
        assert_eq!(
            image.byte(MemoryKind::Ram, reserved.address),
            MemoryByte::Unknown
        );
        assert_eq!(
            image.byte(MemoryKind::Ram, reserved.address + reserved.size),
            MemoryByte::Unknown
        );
        assert_eq!(image.byte(MemoryKind::Flash, 0), MemoryByte::Unknown);
        let json = serde_json::to_string(&report).unwrap();
        assert!(!json.contains("memory_image"));
        assert!(serde_json::from_str::<crate::Analysis>(&json)
            .unwrap()
            .memory_image
            .is_none());
    }
}
