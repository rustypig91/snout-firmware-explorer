# Rusty's Snout — Firmware Explorer

A standalone Rust desktop application and CLI for understanding embedded firmware memory usage. The first milestone targets **ARM Cortex-M and GCC**, with generic ELF models that leave room for other architectures.

## Run it

Install a current stable Rust toolchain. Windows builds need the Visual Studio C++ Build Tools and Windows SDK. Linux desktop builds need a working X11 or Wayland session and development packages for Wayland, xkbcommon and OpenGL (on Ubuntu: `libwayland-dev libxkbcommon-dev libgl1-mesa-dev`). The CLI and core do not require desktop libraries.

```sh
cargo build --workspace --locked
cargo run -p firmware-gui -- fixtures --elf cortex-m.elf
```

Pass `--elf FILE` to select firmware automatically after the folder scan. Relative file paths are resolved inside the supplied build folder. You can also open an ELF directly: `firmware-gui fixtures/cortex-m.elf`. With no path, Snout opens the last build folder and restores the saved workspace. Opening the same build folder without an explicit ELF selection restores the last selected firmware if it is still present. An explicit ELF selection takes priority; a missing saved ELF leaves the folder open for selection.

Select **Open build folder**, press Ctrl+O, or drag a folder into the window. The application recursively scans for linked ELF images (including `.elf`, `.axf`, `.out` and extensionless images), linker maps (`.map`), stack reports (`.su`), and valid memory-layout JSON files. The **Build files** sidebar lists the discovered artifacts with relative paths and a search field. Select a firmware image to analyze it. Selecting a supporting file opens its contents in **Overview** and keeps the selected firmware active. Other tabs continue to show the selected firmware. Rescan the folder after rebuilding.

Selecting firmware automatically imports memory-region capacities from a unique matching GNU ld map (`app.map` or `app.elf.map` for `app.elf`), preferring a sibling file. Ambiguous or unsupported maps leave capacity unknown and produce analysis notes. Map matching is filename-based, not proof that artifacts came from the same build. You can select another map and choose **Use memory regions from this map**, or apply a memory-layout JSON. Manual map and memory-layout choices are remembered for each firmware image in each build folder across restarts. Chosen maps are read again when reopening or refreshing firmware after a rebuild. The map currently supplying memory regions is marked **(in use)** in the sidebar. **Configure memory regions** starts in the build folder; **Menu > Reset settings for this build folder** clears that folder’s saved choices and rediscovers the current firmware’s matching map. Linker scripts are not listed or evaluated. Memory roles are inferred from map region names and attributes. See the [GNU linker documentation](https://sourceware.org/binutils/docs/ld/Options.html) for generating a link map.

All discovered `.su` reports load automatically when firmware is selected. Reports can span several targets or configurations if the selected folder does; the Stack view defaults to entries with candidates in the selected ELF and preserves report paths and matching evidence. Enable **Show unresolved reports** to inspect entries that could not be associated. Functions without matching reports are counted with unknown local stack size. Select an individual `.su` file to inspect only that report, or use **Load all build reports** to restore the complete set. No build ownership or call-chain totals are invented.

Compact tabs switch between Overview, Files, Symbols, Sections, Memory map, Stack and Compare. Overview shows separate Flash, RAM and all-section bar breakdowns, firmware identity, configured capacity, largest contributors and analysis status. Click a breakdown row to drill down from section to compilation unit, then to functions and data symbols. Other / unconnected contains symbols without a known unit and uncovered bytes. Units use DWARF ownership where available, falling back to ELF compilation-unit labels. Use Back or the mouse Back button to move up one level. Function arguments do not have separately measured section sizes. Symbol bars count unique bytes, with aliases and zero-sized labels retained in the legend. Tables support search and sorting, with right-aligned numeric columns. Click a row to expand its details directly underneath; click it again to collapse. Expanded file rows include a button to inspect their symbols. Enable Directories for an optional source tree sidebar. The footer shows memory totals, row counts and expandable analysis notes; Escape collapses the expanded row and notes. Firmware stays local. The CLI continues to accept individual ELF paths.


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

## Releases and updates

The [build workflow](.github/workflows/build.yml) follows Pigtail's Windows/Linux release workflow. Run it manually on a branch, or label a PR `build`, `build-linux`, or `build-windows` to produce downloadable artifacts. Pushing `v<workspace-version>` publishes a GitHub release; the workflow rejects tags that do not match `Cargo.toml`.

Release assets include portable archives containing the GUI and CLI, standalone GUI executables for the updater, Windows MSI and Setup installers, a Debian package, and an AppImage. Linux builds also capture `snout-screenshot.png` using the regular app with `fixtures/cortex-m.elf` selected. There is no demo build. The screenshot runs in an isolated Xvfb session with fresh preferences and update checks disabled.

Build packages locally using `bash scripts/build-release.sh` on x86_64 Debian/Ubuntu or `scripts\build-release.cmd` from a Windows developer shell. Output goes to `target/release-assets/<target>/`. The Linux script needs the desktop build packages listed above, plus `curl`, `pkg-config`, and `dpkg-dev`; packaging downloads linuxdeploy and installs cargo-deb if needed. Windows requires Rust, the C++ Build Tools and Windows SDK; the script downloads portable WiX and Inno Setup. To capture a screenshot locally, install `xvfb xauth xdotool imagemagick`, then run:

```sh
bash scripts/capture-screenshot.sh target/release/firmware-gui target/snout-screenshot.png fixtures/cortex-m.elf
```

Snout checks GitHub releases at startup. **Menu → Check for updates** provides a manual check. **Support developer** opens Buy Me a Coffee, and **About** shows the app version and repository link. Startup checks stay quiet on network errors, when up to date, or for a skipped version. Download and installation begin only when you press **Update**, and the download's published size and SHA-256 digest are verified before installation. Workspace preferences are saved before installing and restored after restarting. Portable binaries update in place, AppImages replace the original AppImage, and Windows Setup installations reuse their existing installation scope. Debian installations use the package manager instead. `--no-update-check` suppresses the startup request for one launch.

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

The CLI's `analyze`, `files` and `symbols` JSON modes return the same complete report envelope, including `schema_version`, options, warnings, sections, symbols, file totals and address ranges. `diff` and `stack` have their own versioned envelopes. Comparison reports include section, file and symbol deltas; same-name sections are aggregated. Sizes and addresses are exact integers; signed comparison deltas are bytes. Exit status is zero for successful analysis (including documented missing information), nonzero for invalid input or command usage. No CI policy engine is included yet.

The desktop overview shows static RAM composition, shortened source paths with full paths on hover, and an expandable explanation of unattributed bytes by section. BSS and reservation categories use section naming conventions; no-load storage alone does not establish how startup initializes it. Stack summaries show the largest uniquely matched local frame and explain why total stack is unknown. Reports without ELF matches remain visible when no matches exist, including stripped firmware. A loaded comparison highlights the largest Flash/RAM increases and links to the Sections, Files and Symbols comparison tables. F5 keeps the baseline snapshot while reloading current firmware and rereading selected map/JSON layouts. You can drop a build folder or an ELF into the window.

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

ELF metadata does not conclusively identify physical memory technology. Writable, copied, and no-payload allocated sections are inferred as RAM under the bare-metal model. An optional JSON memory layout overrides matching load/runtime ranges. See `examples/cortex-m-memory.json`; addresses are decimal JSON integers. A region must contain the entire range. Uncovered ranges produce warnings and retain inferred classification. In Memory map, each configured region shows its address bounds, capacity, used/free bytes and percentage used. Select a region to browse and search its symbols, including initial load images of data or code copied to RAM. Load a layout through Memory map > Load memory regions (for example `examples/cortex-m-memory.json`). Occupancy counts the union of allocated load and runtime section ranges intersecting each physical region, including padding and reservations, without counting the same addresses twice. Boundary-crossing sections count only their intersection; analysis notes still flag incomplete coverage. Free space means capacity minus static ELF occupancy, not guaranteed runtime headroom or a contiguous allocation. Without a layout, capacity and free space remain unknown. Stripped files retain section usage but may have no symbols.

### Attribution and uncertainty

- Function source paths and lines come from DWARF address lookup, refined by definitions matching the symbol name and address. Supported variable definitions also supply source ownership. This is symbol-level attribution, not byte-by-byte ownership of inlined code.
- Local symbols can fall back to ELF `STT_FILE` compilation-unit labels. These are **not proven object-file paths**. Global symbols are not assigned to whichever file label happened to precede them.
- File totals include an explicit **unattributed** bucket for unknown owners, padding, reservations and uncovered bytes. File/tree totals reconcile with the overview.
- Symbol sizes remain the ELF values. Zero-sized labels do not acquire guessed sizes. Overlapping symbols/weak aliases share unique memory contributions, assigned once in section/address/name order. File attribution can consequently depend on which alias owns shared bytes.
- Stack entries retain compiler qualifiers (`static`, `dynamic`, `dynamic,bounded`) and ELF candidates matched by exact symbol name, DWARF source file/line, or a demangled function name without parameters. Known conflicting source paths are excluded; source locations disambiguate overloads when available. Zero matches are unresolved; multiple matches are ambiguous. Reports must belong to the same build. No call targets or call-chain stack totals are invented.

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

1. **Attribution:** extend DWARF definition support and add GNU linker MAP object/archive ownership. Map import currently reads region capacities only. Variable attribution supports direct address expressions; location lists, TLS, complex expressions and definitions requiring reference resolution remain unsupported. Global variables without a matching supported definition remain unattributed. Compressed and split/external DWARF are not supported.
2. **Memory layouts:** add saved target profiles, explicit unknown memory roles, overlay policies and segment-only fallback. Relocatable objects, overlapping allocated/load ranges, TLS and sectionless ELFs currently return clear unsupported errors. HEX/BIN analysis and standalone MAP symbol/section analysis are not supported. Dynamic-symbol-only attribution is not yet implemented.
3. **Comparisons:** normalize source roots across build machines, improve duplicate/renamed symbol matching and introduce simple CI budgets. Matching currently uses file label, section and mangled symbol name, with duplicate identities aggregated. Differences in attribution or debug availability can affect per-file/symbol deltas.
4. **Stack:** import evidenced call graphs and represent recursion, indirect calls, assembly, interrupts and missing data before estimating call chains. The model reserves these uncertainty categories. CFA/disassembly, RTOS task stacks and runtime high-water marks are later inputs.
5. **Desktop delivery:** validate on representative user firmware and Linux desktops, refine large-report performance, add installers, accessibility/keyboard review and preferences. Source tables currently rebuild their display rows each frame, although visible rows are virtualized. Region occupancy and largest-contributor rankings are cached per analysis. Workspace preferences remember the folder, firmware, layout, view and overview memory selection. Refresh (F5) rescans and reloads the selected firmware while preserving the last successful report on failure. Preferences are stored in `snout-firmware-explorer/workspace.json` under the user configuration directory (APPDATA on Windows, XDG_CONFIG_HOME or ~/.config elsewhere). No export dialog is included yet; use CLI JSON for export.

No VS Code dependency is present. A future extension or CI service can consume core reports or the CLI without the desktop frontend.
