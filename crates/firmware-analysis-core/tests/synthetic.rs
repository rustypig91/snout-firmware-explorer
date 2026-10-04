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
fn overlays_and_tls_are_explicitly_unsupported() {
    let original = fixture(true, false);
    let table = u64::from_le_bytes(original[40..48].try_into().unwrap()) as usize;
    let mut overlay = original.clone();
    put(&mut overlay, table + 128 + 16, 0x08000002, 8, false);
    assert!(analyze_bytes(&overlay, "overlay", &Default::default())
        .unwrap_err()
        .to_string()
        .contains("Overlapping"));
    let mut tls = original;
    put(&mut tls, table + 128 + 8, 0x403, 8, false);
    assert!(analyze_bytes(&tls, "tls", &Default::default())
        .unwrap_err()
        .to_string()
        .contains("Thread-local"));
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
