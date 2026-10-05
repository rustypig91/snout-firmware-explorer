//! Small independent ELF encodings exercise endian/bitness and invalid layouts.
//! Production parsing is delegated to goblin; this writer exists only in tests.
use firmware_analysis_core::{analyze_bytes, AnalysisOptions};

fn put(data: &mut [u8], offset: usize, value: u64, width: usize, big: bool) {
    for i in 0..width {
        data[offset + i] = (value >> (8 * if big { width - 1 - i } else { i })) as u8;
    }
}
fn fixture(wide: bool, big: bool) -> Vec<u8> {
    let header = if wide { 64 } else { 52 };
    let stride = if wide { 64 } else { 40 };
    let names = b"\0.custom\0.scratch\0.shstrtab\0.debug_info\0";
    let table = header + 4 + names.len();
    let mut data = vec![0; table + 4 * stride];
    data[..7].copy_from_slice(&[
        0x7f,
        b'E',
        b'L',
        b'F',
        if wide { 2 } else { 1 },
        if big { 2 } else { 1 },
        1,
    ]);
    put(&mut data, 16, 2, 2, big);
    put(&mut data, 18, if wide { 183 } else { 40 }, 2, big);
    put(&mut data, 20, 1, 4, big);
    put(
        &mut data,
        if wide { 40 } else { 32 },
        table as u64,
        if wide { 8 } else { 4 },
        big,
    );
    put(&mut data, if wide { 52 } else { 40 }, header as u64, 2, big);
    put(&mut data, if wide { 58 } else { 46 }, stride as u64, 2, big);
    put(&mut data, if wide { 60 } else { 48 }, 4, 2, big);
    put(&mut data, if wide { 62 } else { 50 }, 3, 2, big);
    data[header..header + 4].copy_from_slice(&[1, 2, 3, 4]);
    data[header + 4..table].copy_from_slice(names);
    for (index, name, kind, flags, address, offset, size) in [
        (1, 1, 1, 2, 0x08000000, header, 4),
        (2, 9, 8, 3, 0x20000000, 0, 12),
        (3, 18, 3, 0, 0, header + 4, names.len()),
    ] {
        let at = table + index * stride;
        put(&mut data, at, name, 4, big);
        put(&mut data, at + 4, kind, 4, big);
        if wide {
            for (field, value) in [
                (8, flags),
                (16, address),
                (24, offset as u64),
                (32, size as u64),
                (48, 1),
            ] {
                put(&mut data, at + field, value, 8, big);
            }
        } else {
            for (field, value) in [
                (8, flags),
                (12, address),
                (16, offset as u64),
                (20, size as u64),
                (32, 1),
            ] {
                put(&mut data, at + field, value, 4, big);
            }
        }
    }
    data
}

#[test]
fn endian_and_bitness_do_not_change_accounting() {
    for wide in [false, true] {
        for big in [false, true] {
            let a = analyze_bytes(
                &fixture(wide, big),
                "synthetic",
                &AnalysisOptions::default(),
            )
            .unwrap();
            assert_eq!(a.metadata.bitness, if wide { 64 } else { 32 });
            assert_eq!(a.metadata.endianness, if big { "Big" } else { "Little" });
            assert_eq!(a.totals.flash, 4);
            assert_eq!(a.totals.ram, 12);
            assert!(a.warnings.iter().any(|s| s.contains("no matching PT_LOAD")));
        }
    }
}

#[test]
fn overflowing_64_bit_addresses_are_rejected() {
    let mut data = fixture(true, false);
    let table = u64::from_le_bytes(data[40..48].try_into().unwrap()) as usize;
    put(&mut data, table + 64 + 16, u64::MAX - 1, 8, false);
    assert!(analyze_bytes(&data, "overflow", &Default::default()).is_err());
}

#[test]
fn debug_sections_without_file_payload_do_not_read_arbitrary_offsets() {
    for wide in [true, false] {
        for big in [false, true] {
            let mut data = fixture(wide, big);
            let header = if wide { 64 } else { 52 };
            let stride = if wide { 64 } else { 40 };
            let table = header + 4 + b"\0.custom\0.scratch\0.shstrtab\0.debug_info\0".len();
            let section = table + stride;
            put(&mut data, section, 28, 4, big);
            put(&mut data, section + 4, 8, 4, big); // SHT_NOBITS has no file data.
            put(&mut data, section + 8, 0, if wide { 8 } else { 4 }, big);
            put(
                &mut data,
                section + if wide { 24 } else { 16 },
                if wide { u64::MAX } else { u32::MAX.into() },
                if wide { 8 } else { 4 },
                big,
            );
            let analysis = analyze_bytes(&data, "no debug payload", &Default::default()).unwrap();
            assert!(!analysis.metadata.has_dwarf);
            assert_eq!(analysis.totals.ram, 12);
            assert_eq!(analysis.totals.flash, 0);
        }
    }
}

#[test]
fn overlays_are_explicitly_unsupported() {
    let original = fixture(true, false);
    let table = u64::from_le_bytes(original[40..48].try_into().unwrap()) as usize;
    let mut overlay = original.clone();
    put(&mut overlay, table + 128 + 16, 0x08000002, 8, false);
    assert!(analyze_bytes(&overlay, "overlay", &Default::default())
        .unwrap_err()
        .to_string()
        .contains("Overlapping"));
}

#[test]
fn relocatable_objects_are_not_reported_as_final_firmware() {
    let mut data = fixture(false, false);
    put(&mut data, 16, 1, 2, false);
    assert!(analyze_bytes(&data, "object", &Default::default())
        .unwrap_err()
        .to_string()
        .contains("relocatable"));
}

#[test]
fn overlapping_load_payloads_are_rejected_even_in_configured_ram() {
    use firmware_analysis_core::{MemoryKind, MemoryRegion};
    let mut data = include_bytes!("../../../fixtures/build/cortex-m.elf").to_vec();
    let elf = goblin::elf::Elf::parse(&data).unwrap();
    let second = elf.header.e_phoff as usize + elf.header.e_phentsize as usize;
    // Keep disjoint runtime sections, but overlap the first segment's load image.
    put(&mut data, second + 12, 0x08000000, 4, false);
    for kind in [MemoryKind::Flash, MemoryKind::Ram] {
        let options = AnalysisOptions {
            regions: vec![MemoryRegion {
                name: "load storage".into(),
                start: 0x08000000,
                size: 0x10000,
                kind,
            }],
        };
        assert!(analyze_bytes(&data, "load overlay", &options)
            .unwrap_err()
            .to_string()
            .contains("Overlapping load payloads"));
    }
}

#[test]
fn tls_sections_are_templates_not_physical_runtime_allocations() {
    for wide in [false, true] {
        for big in [false, true] {
            let mut data = fixture(wide, big);
            let header = if wide { 64 } else { 52 };
            let stride = if wide { 64 } else { 40 };
            let table = header + 4 + b"\0.custom\0.scratch\0.shstrtab\0.debug_info\0".len();
            let at = table + 2 * stride;
            put(&mut data, at + 8, 0x403, if wide { 8 } else { 4 }, big);
            // A TLS NOBITS template may overlap ordinary read-only storage.
            put(
                &mut data,
                at + if wide { 16 } else { 12 },
                0x08000000,
                if wide { 8 } else { 4 },
                big,
            );
            put(
                &mut data,
                at + if wide { 48 } else { 32 },
                8,
                if wide { 8 } else { 4 },
                big,
            );
            let report = analyze_bytes(&data, "TLS", &Default::default()).unwrap();
            let tls = report.tls.unwrap();
            assert_eq!(tls.template_size, 12);
            assert_eq!(tls.zero_initialized_size, 12);
            assert_eq!(tls.initialized_size, 0);
            assert_eq!(tls.alignment, 8);
            assert_eq!(tls.total_runtime_ram, None);
            assert_eq!(report.totals.ram, 0);
            assert_eq!(report.totals.flash, 4);
            assert_eq!(report.sections[1].runtime_size, 0);
            assert!(!report.memory_map.iter().any(|r| r.name == ".scratch"));
        }
    }
}

#[test]
fn initialized_tls_payload_is_retained_in_flash_not_static_ram() {
    let mut data = fixture(true, false);
    let table = u64::from_le_bytes(data[40..48].try_into().unwrap()) as usize;
    put(&mut data, table + 64 + 8, 0x403, 8, false);
    let report = analyze_bytes(&data, "initialized TLS", &Default::default()).unwrap();
    assert_eq!(report.totals.flash, 4);
    assert_eq!(report.totals.ram, 12);
    let tls_section = report
        .sections
        .iter()
        .find(|s| s.name == ".custom")
        .unwrap();
    let mut physical = report.clone();
    physical
        .sections
        .iter_mut()
        .find(|s| s.index == tls_section.index)
        .unwrap()
        .load_address = Some(tls_section.address);
    let region = firmware_analysis_core::MemoryRegion {
        name: "template image".into(),
        start: tls_section.address,
        size: 4,
        kind: firmware_analysis_core::MemoryKind::Flash,
    };
    assert_eq!(
        firmware_analysis_core::regions::region_usage(&physical, &region).used,
        4
    );
    let tls = report.tls.unwrap();
    assert_eq!(tls.template_size, 4);
    assert_eq!(tls.initialized_size, 4);
    assert_eq!(tls.zero_initialized_size, 0);
}

fn tls_segment_fixture(wide: bool, big: bool) -> (Vec<u8>, usize) {
    let mut data = fixture(wide, big);
    let header = if wide { 64 } else { 52 };
    let stride = if wide { 64 } else { 40 };
    let table = header + 4 + b"\0.custom\0.scratch\0.shstrtab\0.debug_info\0".len();
    // Initialized template followed by alignment padding and zero-init storage.
    put(
        &mut data,
        table + stride + 8,
        0x403,
        if wide { 8 } else { 4 },
        big,
    );
    put(
        &mut data,
        table + 2 * stride + 8,
        0x403,
        if wide { 8 } else { 4 },
        big,
    );
    put(
        &mut data,
        table + 2 * stride + if wide { 16 } else { 12 },
        0x08000008,
        if wide { 8 } else { 4 },
        big,
    );
    let ph = data.len();
    let ph_size = if wide { 56 } else { 32 };
    data.resize(ph + ph_size, 0);
    put(
        &mut data,
        if wide { 32 } else { 28 },
        ph as u64,
        if wide { 8 } else { 4 },
        big,
    );
    put(
        &mut data,
        if wide { 54 } else { 42 },
        ph_size as u64,
        2,
        big,
    );
    put(&mut data, if wide { 56 } else { 44 }, 1, 2, big);
    put(&mut data, ph, 7, 4, big); // PT_TLS
    let fields = if wide {
        vec![
            (8, header as u64),
            (16, 0x08000000),
            (24, 0x08000000),
            (32, 4),
            (40, 24),
            (48, 8),
        ]
    } else {
        vec![
            (4, header as u64),
            (8, 0x08000000),
            (12, 0x08000000),
            (16, 4),
            (20, 24),
            (28, 8),
        ]
    };
    for (offset, value) in fields {
        put(&mut data, ph + offset, value, if wide { 8 } else { 4 }, big);
    }
    (data, ph)
}

#[test]
fn pt_tls_includes_padding_and_validates_file_and_memory_ranges() {
    for wide in [false, true] {
        for big in [false, true] {
            let (data, ph) = tls_segment_fixture(wide, big);
            let report = analyze_bytes(&data, "PT_TLS", &Default::default()).unwrap();
            let tls = report.tls.unwrap();
            assert_eq!(tls.source, "PT_TLS");
            assert_eq!(tls.initialized_size, 4);
            assert_eq!(tls.template_size, 24); // trailing padding is authoritative
            assert_eq!(tls.zero_initialized_size, 20);
            assert_eq!(tls.alignment, 8);
            assert_eq!(report.totals.ram, 0);
            assert_eq!(report.totals.flash, 4);
            for (offset, value) in [
                (if wide { 32 } else { 16 }, 25),
                (if wide { 40 } else { 20 }, 2),
                (if wide { 48 } else { 28 }, 3),
            ] {
                let mut invalid = data.clone();
                put(
                    &mut invalid,
                    ph + offset,
                    value,
                    if wide { 8 } else { 4 },
                    big,
                );
                assert!(analyze_bytes(&invalid, "invalid TLS", &Default::default()).is_err());
            }
        }
    }
}

#[test]
fn tls_symbol_values_are_offsets_not_runtime_addresses() {
    use goblin::elf::{section_header::SHT_SYMTAB, sym, Elf};
    let mut data = include_bytes!("../../../fixtures/build/cortex-m.elf").to_vec();
    let elf = Elf::parse(&data).unwrap();
    let (symbol_index, raw) = elf
        .syms
        .iter()
        .enumerate()
        .find(|(_, s)| s.st_type() == sym::STT_OBJECT && s.st_size > 0)
        .unwrap();
    let section_index = raw.st_shndx;
    let section = &elf.section_headers[section_index];
    let offset = raw.st_value - section.sh_addr;
    let name = elf.strtab.get_at(raw.st_name).unwrap().to_owned();
    let expected_size = raw.st_size;
    let symtab = elf
        .section_headers
        .iter()
        .find(|s| s.sh_type == SHT_SYMTAB)
        .unwrap();
    let symbol_at = symtab.sh_offset as usize + symbol_index * symtab.sh_entsize as usize;
    let section_at = elf.header.e_shoff as usize + section_index * elf.header.e_shentsize as usize;
    let flags = section.sh_flags | 0x400;
    let info = (raw.st_info & 0xf0) | sym::STT_TLS;
    put(&mut data, section_at + 8, flags, 4, false);
    put(&mut data, symbol_at + 4, offset, 4, false);
    put(&mut data, symbol_at + 12, info.into(), 1, false);
    let report = analyze_bytes(&data, "TLS symbol", &Default::default()).unwrap();
    assert!(!report.symbols.iter().any(|s| s.name == name));
    let tls = report.tls.unwrap();
    let variable = tls.symbols.iter().find(|s| s.name == name).unwrap();
    assert_eq!(variable.offset, offset);
    assert_eq!(variable.size, expected_size);
}

#[test]
fn tls_rejects_zero_fill_inside_initialization_image_and_incompatible_alignment() {
    for wide in [false, true] {
        for big in [false, true] {
            let (data, ph) = tls_segment_fixture(wide, big);
            let header = if wide { 64 } else { 52 };
            let stride = if wide { 64 } else { 40 };
            let table = header + 4 + b"\0.custom\0.scratch\0.shstrtab\0.debug_info\0".len();
            let width = if wide { 8 } else { 4 };
            // No section overlap: the initialized segment extends into .scratch.
            let mut invalid = data.clone();
            put(
                &mut invalid,
                ph + if wide { 32 } else { 16 },
                12,
                width,
                big,
            );
            assert!(
                analyze_bytes(&invalid, "TLS zero fill", &Default::default())
                    .unwrap_err()
                    .to_string()
                    .contains("Zero-initialized TLS section")
            );
            for alignment in [3, 16] {
                let mut invalid = data.clone();
                put(
                    &mut invalid,
                    table + 2 * stride + if wide { 48 } else { 32 },
                    alignment,
                    width,
                    big,
                );
                assert!(
                    analyze_bytes(&invalid, "TLS alignment", &Default::default())
                        .unwrap_err()
                        .to_string()
                        .contains("TLS section alignment")
                );
            }
            // Without PT_TLS, putting zero fill before initialized data cannot
            // be represented by an initialized prefix followed by a zero tail.
            let mut invalid = data;
            put(&mut invalid, if wide { 56 } else { 44 }, 0, 2, big);
            put(
                &mut invalid,
                table + 2 * stride + if wide { 16 } else { 12 },
                0x07fffff0,
                width,
                big,
            );
            assert!(
                analyze_bytes(&invalid, "inferred TLS zero fill", &Default::default())
                    .unwrap_err()
                    .to_string()
                    .contains("Zero-initialized TLS section")
            );
        }
    }
}
