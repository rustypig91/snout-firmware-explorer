//! Physical region occupancy, including load images and runtime reservations.
use crate::{Analysis, MemoryRegion};

#[derive(Debug)]
pub struct RegionUsage {
    pub used: u64,
    pub free: u64,
    pub symbols: Vec<RegionSymbol>,
}

#[derive(Debug)]
pub struct RegionSymbol {
    pub symbol_index: usize,
    pub address: u64,
    pub placement: &'static str,
}

/// Counts the union of allocated load/runtime bytes intersecting a region.
/// Free bytes exclude static ELF occupancy only, not future heap/stack demand.
pub fn region_usage(analysis: &Analysis, region: &MemoryRegion) -> RegionUsage {
    let end = region.start.saturating_add(region.size);
    let mut ranges = Vec::new();
    let mut symbols = Vec::new();
    for section in analysis.sections.iter().filter(|s| s.allocated) {
        let mut placements = vec![(section.address, section.runtime_size, "Runtime")];
        if let Some(load) = section.load_address.filter(|_| section.load_size > 0) {
            if load == section.address {
                placements[0].2 = "Load / runtime";
            } else {
                placements.push((load, section.load_size, "Load image"));
            }
        }
        for (address, size, placement) in placements {
            let start = address.max(region.start);
            let stop = address.saturating_add(size).min(end);
            if start >= stop {
                continue;
            }
            ranges.push((start, stop));
            for (symbol_index, symbol) in analysis.symbols.iter().enumerate() {
                if symbol.section_index != section.index {
                    continue;
                }
                let Some(offset) = symbol.normalized_address.checked_sub(section.address) else {
                    continue;
                };
                if offset >= size {
                    continue;
                }
                let Some(symbol_address) = address.checked_add(offset) else {
                    continue;
                };
                let symbol_end = symbol_address.saturating_add(symbol.size.min(size - offset));
                if (symbol.size == 0 && symbol_address >= start && symbol_address < stop)
                    || (symbol.size > 0 && symbol_address < stop && symbol_end > start)
                {
                    symbols.push(RegionSymbol {
                        symbol_index,
                        address: symbol_address,
                        placement,
                    });
                }
            }
        }
    }
    ranges.sort_unstable();
    let mut cursor = region.start;
    let mut used = 0;
    for (start, stop) in ranges {
        used += stop.saturating_sub(start.max(cursor));
        cursor = cursor.max(stop);
    }
    RegionUsage {
        used,
        free: region.size.saturating_sub(used),
        symbols,
    }
}
