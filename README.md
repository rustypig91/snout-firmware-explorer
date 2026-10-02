# Rusty's Snout — Firmware Explorer

A standalone Rust desktop application and CLI for understanding embedded firmware memory usage. The first milestone targets **ARM Cortex-M and GCC**, with generic ELF models that leave room for other architectures.

## Run it

Install a current stable Rust toolchain. Windows builds need the Visual Studio C++ Build Tools and Windows SDK. Linux desktop builds need a working X11 or Wayland session and development packages for Wayland, xkbcommon and OpenGL (on Ubuntu: `libwayland-dev libxkbcommon-dev libgl1-mesa-dev`). The CLI and core do not require desktop libraries.

```sh
cargo build --workspace --locked
cargo run -p firmware-gui -- fixtures/cortex-m.elf
```

Open your own `.elf`, `.axf`, or ELF-format `.out` file using the toolbar or drag-and-drop. The GUI provides Overview, Files, Symbols, Sections, Memory map, Stack and Compare views. Tables support search and sorting; click a file to inspect its symbols. Hover headings and rows for explanations, exact bytes and metadata. File-dialog choices are local; the application does not upload firmware.

```sh
cargo run -p firmware-cli -- analyze fixtures/cortex-m.elf
cargo run -p firmware-cli -- files fixtures/cortex-m.elf
cargo run -p firmware-cli -- symbols fixtures/cortex-m.elf
cargo run -p firmware-cli -- diff fixtures/cortex-m.elf fixtures/cortex-m-grown.elf
cargo run -p firmware-cli -- stack fixtures/cortex-m.elf --stack-usage fixtures/
cargo run -p firmware-cli -- analyze fixtures/cortex-m.elf --format json
cargo run -p firmware-cli -- analyze fixtures/cortex-m.elf --config examples/cortex-m-memory.json
```

For optimized standalone executables:

```sh
cargo build --workspace --release --locked
```

Executables are `target/release/firmware-gui` and `target/release/firmware-explorer` (with `.exe` on Windows). Neither requires a Rust installation on the destination machine. Desktop platform libraries still apply.

## Architecture

```text
firmware-gui (egui/eframe) ─┐
                          ├── firmware-analysis-core
firmware-cli (clap) ───────┘       ├── ELF adapter: goblin
                                  ├── DWARF: gimli + addr2line
                                  ├── accounting / attribution / tree
                                  ├── comparison
                                  └── compiler stack reports / call-graph model
```

The core accepts a path or byte slice plus analysis options and returns a serializable `Analysis`. Parser types stay internal. Frontends do not perform accounting. GUI analysis runs on a worker thread, and failed loads preserve the last successful report. Memory configuration changes are validated and applied together with a successful reanalysis.

The CLI's `analyze`, `files` and `symbols` JSON modes return the same complete report envelope, including `schema_version`, options, warnings, sections, symbols, file totals and address ranges. `diff` and `stack` have their own versioned envelopes. Sizes and addresses are exact integers; signed comparison deltas are bytes. Exit status is zero for successful analysis (including documented missing information), nonzero for invalid input or command usage. No CI policy engine is included yet.

### Parsing and UI choices

- [goblin](https://docs.rs/goblin/0.10.7/goblin/elf/struct.Elf.html) exposes ELF section and program headers, including physical load addresses, directly. This avoids writing a production binary parser.
- [gimli and addr2line](https://docs.rs/addr2line/0.25.1/addr2line/) provide DWARF line lookup. `cpp_demangle` and `rustc-demangle` preserve raw symbol names alongside readable names.
- [egui/eframe](https://docs.rs/eframe/0.30.0/eframe/) provides a native Rust desktop frontend with virtualized table rows, contextual help and no web frontend build step. Versions are pinned by `Cargo.lock`; deliberate dependency upgrades should run the full checks.
- Disassembly and graph-algorithm dependencies are deferred until there is a concrete call-graph implementation to validate.

## What the numbers mean

**Flash payload** is the sum of allocated, file-backed section bytes inferred to be stored in Flash. Non-allocated debug data, ELF headers, segment padding, gaps between addresses and programmer-specific image overhead are excluded. `metadata.segment_file_bytes` separately reports the sum of `PT_LOAD` file sizes; do not add it to Flash usage.

**RAM at runtime (static)** counts sections inferred to occupy RAM, including explicit heap/stack reservations represented as allocated sections. It is **not** a prediction of all memory needed during execution. Runtime allocation, unreserved stacks, interrupt overhead and OS allocations remain unknown.

| Storage | Flash payload | Static RAM |
|---|---:|---:|
| Read-only code and constants | yes | normally no |
| Initialized writable data | yes | yes |
| Code copied to RAM, identified by different load/run addresses | yes | yes |
| Allocated `SHT_NOBITS` storage such as BSS or stack reservations | no | normally yes |
| Debug information | no | no |

Classification uses allocation/write flags, section type, and matching `PT_LOAD` mappings, **not section-name lists**. For file-backed sections, the load address is `p_paddr + (sh_offset - p_offset)` after checking both file and virtual containment and their relative offsets. ARM function addresses are normalized for the Thumb bit during attribution; the raw value is retained. `SHT_NOBITS` means no file payload; it does not prove how startup initializes that memory.

ELF metadata does not conclusively identify physical memory technology. Writable, copied, and no-payload allocated sections are inferred as RAM under the bare-metal model. An optional JSON memory layout overrides matching load/runtime ranges. See `examples/cortex-m-memory.json`; addresses are decimal JSON integers. A region must contain the entire range. Uncovered ranges produce warnings and retain inferred classification. Capacity bars cover fully contained sections; they are not a placement or free-space allocator.

### Attribution and uncertainty

- Function source paths and lines come from DWARF address lookup. This is the function's location, not byte-by-byte ownership of inlined code.
- Local symbols can fall back to ELF `STT_FILE` compilation-unit labels. These are **not proven object-file paths**. Global symbols are not assigned to whichever file label happened to precede them.
- File totals include an explicit **unattributed** bucket for unknown owners, padding, reservations and uncovered bytes. File/tree totals reconcile with the overview.
- Symbol sizes remain the ELF values. Zero-sized labels do not acquire guessed sizes. Overlapping symbols/weak aliases share unique memory contributions, assigned once in section/address/name order. File attribution can consequently depend on which alias owns shared bytes.
- Stack entries retain compiler qualifiers (`static`, `dynamic`, `dynamic,bounded`) and exact-name ELF candidates. Zero matches are unresolved; multiple matches are ambiguous. Reports must belong to the same build. No call targets or call-chain stack totals are invented.

## Validation

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

Tests cover real GNU Arm GCC Cortex-M ELF files, DWARF function locations, local/global symbols, aliases, stripped files, initialized data, reservations, RAM code, unusual sections, comparisons, malformed input, 32/64-bit and little/big-endian synthetic ELF, CLI JSON/errors, compiler stack reports, and headless rendering of every data view. The checked-in fixtures allow tests without an ARM toolchain. Their C sources and linker script are in `fixtures/src`; regeneration instructions are in `fixtures/README.md`.

Windows tests and builds were run locally. CI is configured for Windows and Linux; Linux desktop execution and native visual/interaction review still need validation. Headless egui tests do not validate native file dialogs or graphics drivers.

## Current boundaries and next milestones

This is the first implementation milestone, not a claim of universal firmware support.

1. **Attribution:** add DWARF variable DIE attribution and GNU linker MAP input for reliable object/archive ownership. Currently global variables without a local compilation-unit label remain unattributed. Compressed and split/external DWARF are not supported.
2. **Memory layouts:** add saved target profiles, explicit unknown memory roles, overlay policies and segment-only fallback. Relocatable objects, overlapping allocated/load ranges, TLS and sectionless ELFs currently return clear unsupported errors. HEX/BIN/MAP-only input is not parsed. Dynamic-symbol-only attribution is not yet implemented.
3. **Comparisons:** normalize source roots across build machines, improve duplicate/renamed symbol matching, add GUI filtering for large comparisons and introduce simple CI budgets. Matching currently uses file label, section and mangled symbol name, with duplicate identities aggregated. Differences in attribution or debug availability can affect per-file/symbol deltas.
4. **Stack:** import evidenced call graphs and represent recursion, indirect calls, assembly, interrupts and missing data before estimating call chains. The model reserves these uncertainty categories. CFA/disassembly, RTOS task stacks and runtime high-water marks are later inputs.
5. **Desktop delivery:** validate on representative user firmware and Linux desktops, refine large-report performance, add installers, accessibility/keyboard review and preferences. Source tables currently rebuild their display rows each frame, although visible rows are virtualized. No session persistence or export dialog is included yet; use CLI JSON for export.

No VS Code dependency is present. A future extension or CI service can consume core reports or the CLI without the desktop frontend.
