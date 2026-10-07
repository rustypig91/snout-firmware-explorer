# Rusty's Snout — Firmware Explorer

A standalone Rust desktop application and CLI for understanding embedded firmware memory usage. The first milestone targets **ARM Cortex-M and GCC**, with generic ELF models that leave room for other architectures.

## Run it

Install a current stable Rust toolchain. Windows builds need the Visual Studio C++ Build Tools and Windows SDK. Linux desktop builds need a working X11 or Wayland session and development packages for Wayland, xkbcommon and OpenGL (on Ubuntu: `libwayland-dev libxkbcommon-dev libgl1-mesa-dev`). The CLI and core do not require desktop libraries.

```sh
cargo build --workspace --locked
cargo run -p firmware-gui -- fixtures/build --elf cortex-m.elf
```

Pass `--elf FILE` to select firmware automatically after the folder scan. Relative file paths are resolved inside the supplied build folder. You can also open an ELF directly: `firmware-gui fixtures/build/cortex-m.elf`. With no path, Snout opens the last build folder and restores the saved workspace. Opening the same build folder without an explicit ELF selection restores the last selected firmware if it is still present. An explicit ELF selection takes priority; a missing saved ELF leaves the folder open for selection.

Select **Open build folder**, press Ctrl+O, or drag a folder into the window. **Menu > Open recent build folder** remembers the five most recently opened folders across restarts. Reopening a folder moves it to the top without adding a duplicate. The application recursively scans for linked ELF images (including `.elf`, `.axf`, `.out` and extensionless images), linker maps (`.map`) and stack reports (`.su`). The **Build files** sidebar lists the discovered artifacts with relative paths and a search field. Select a firmware image to analyze it. Selecting a supporting file opens its contents in **Overview** and keeps the selected firmware active. Other tabs continue to show the selected firmware. Select **Menu > Refresh** or press F5 after rebuilding. Menu shortcut hints also show **Ctrl+O** to open a build folder and **F1** for About.

Selecting firmware automatically imports memory-region capacities from a unique matching GNU ld or Texas Instruments CGT map (`app.map` or `app.elf.map` for `app.elf`), preferring a sibling file. Ambiguous or unsupported maps leave capacity unknown and produce analysis notes. Map matching is filename-based, not proof that artifacts came from the same build. Use a map’s radio button in the sidebar to select its memory regions. Manual map choices are remembered for each firmware image in each build folder across restarts. Chosen maps are read again when reopening or refreshing firmware after a rebuild. The map currently supplying memory regions has its radio button selected in the sidebar. **Select linker map** starts in the build folder; **Menu > Reset settings for this build folder** clears that folder’s saved choices and rediscovers the current firmware’s matching map. Linker scripts are not listed or evaluated. Memory roles are inferred from map region names and attributes. See the [GNU linker documentation](https://sourceware.org/binutils/docs/ld/Options.html) for generating a link map.

Map previews automatically identify GNU ld, Texas Instruments CGT, and LLVM lld ELF formats from their contents. TI CGT import supports old `attributes` tables and newer `used / unused / attr` tables, hexadecimal values, and non-overlapping `PAGE` groups. Capacities come from region lengths; usage remains calculated from the ELF. Explicit FLASH/ROM names take precedence over TI's often-permissive `RWIX` attributes. Empty TI ranges are skipped. Overlapping page address spaces and recognized word-addressed C28x/C54x/C55x targets are rejected because the current memory model requires one byte-addressed physical space. TI cross references and section/object attribution are not imported. Unknown formats are identified as unsupported. See [TI's linker-map example](https://software-dl.ti.com/codegen/docs/tiarmclang/rel3_1_0_STS/compiler_manual/linker_description/12_linker_example/linker-example-stdz0759199.html).

LLVM lld ELF maps (`VMA LMA Size Align Out In Symbol`) support a parsed **LLVM output section placement** preview with runtime/load addresses and sizes, plus dependency import from `--cref` tables. Generate these with `-Wl,-Map,app.map,--cref,--no-demangle` when linking with lld. Automatic matching and manual **Use cross references from this map** both work. LLVM maps do not report physical region capacities: physical capacities remain unknown. The section preview does not alter ELF analysis. LLVM's COFF, Mach-O, and WebAssembly map dialects are not supported. See [LLVM's ELF map writer](https://github.com/llvm/llvm-project/blob/main/lld/ELF/MapFile.cpp).

ELF/DWARF information always takes precedence over map information for symbol and source attribution. Maps supplement the ELF with region capacities and supported linker cross references; they never overwrite DWARF ownership or symbol sizes/addresses.

When firmware is selected, Snout guesses `.su` reports from linker-map object paths, matching target directories, or unambiguous ELF function matches. Review or change the selection with file and recursive directory checkboxes under **Stack usage** in the sidebar. Choices are saved per ELF immediately and restored on refresh and restart, including an empty selection. Selected directories include future reports; selecting a file includes future siblings while preserving existing unchecked files. Reports can span several targets or configurations, so verify their build ownership. The Stack view defaults to entries with candidates in the selected ELF and preserves report paths and matching evidence. Gray bars behind Local frame byte counts compare each row with the largest visible frame: a full cell is 100%, and half the bytes fills half the cell. Enable **Show unresolved reports** to inspect entries that could not be associated. Functions without matching reports are counted with unknown local stack size. No call-chain totals are invented.

Compact tabs switch between Overview, Files, Symbols, Sections, Memory map, Dependencies and Stack. Interface text has a 12 pt minimum; labels inside the zoomable dependency diagram scale with zoom. Overview shows separate Flash, RAM and all-section bar breakdowns, firmware identity, configured capacity, largest contributors and analysis status. Click a breakdown row to drill down from section to compilation unit, then to functions and data symbols. Other / unconnected contains symbols without a known unit and uncovered bytes. Units use DWARF ownership where available, falling back to ELF compilation-unit labels. Use Back or the mouse Back button to move up one level. Function arguments do not have separately measured section sizes. Symbol bars count unique bytes, with aliases and zero-sized labels retained in the legend. Tables support search and sorting, with right-aligned numeric columns. Columns fit their content when space allows and share spare width in smaller windows. Widths update when the window or table data changes; manual resizing remains available. Gray bars in the Size/ELF size, Flash and RAM columns of Sections, Symbols and Files compare each value with its own column’s largest visible value. Click a row to expand its details directly underneath; click it again to collapse. Expanded file rows include a button to inspect their symbols. Enable Directories for an optional source tree sidebar. The footer shows memory totals, row counts and expandable analysis notes; Escape collapses the expanded row and notes. Firmware stays local. The CLI continues to accept individual ELF paths.


The **Dependencies** tab shows a hierarchical compilation-unit graph with clickable boxes and curved arrows, pan/zoom, directory grouping, search, and a focused neighborhood view. It flows top-to-bottom in tall canvases and left-to-right in wide canvases. Box area is proportional to the selected Flash/RAM contribution above a minimum size measured from the labels; the largest box uses 2.5 times that minimum area. Each box also displays its byte count. Filtering shows matching units and their directly connected dependencies and dependents, within the selected-unit focus when enabled. Switching memory modes or filtering recalculates and fits the layout. Arrows run from a unit using a symbol to the unit defining it; selecting an arrow lists the connecting symbols. Sizes count attributed symbol bytes, excluding padding and unknown ownership. DWARF compilation-unit ownership takes priority over ELF unit labels; source locations in headers do not create units.

To supply connections, generate a GNU ld or LLVM lld ELF linker map with `-Wl,-Map,app.map,--cref,--no-demangle`. A unique matching map loads automatically with firmware. Alternatively, select a map in **Build files** and choose **Use cross references from this map**. Explicit dependency-map choices are saved per firmware independently of memory layouts and reread on F5 and restart. Failed explicit imports preserve the current report; an unavailable saved map leaves source nodes visible with a note and no stale connections.

Cross references include functions, global data, and function addresses, and may include discarded code; they do not prove runtime calls. Source-unit connections require a global ELF definition of the connecting symbol; symbols missing from the ELF are omitted because they may be unresolved or discarded. Object-only graphs cannot distinguish unresolved weak references from definitions in GNU ld cross-reference tables, so their arrows need verification. Object/archive paths associate with source units only through exact ELF symbol definitions with one consistent unit owner. Ambiguous or unknown objects remain separate nodes with unknown memory contribution. LTO and stripped builds can limit source association. Without cross references, the graph shows known units and explains that connections are unavailable; an isolated node is not proof of independence. The committed Cortex-M fixtures include six source units and sixteen connections, including reciprocal dependencies; open `fixtures/build/cortex-m.elf` from the build folder to explore them. Weak aliases can inherit ownership from an exact function range with one known unit; conflicting owners remain ambiguous. ELF-only CLI JSON includes unit nodes in the additive `dependencies` field; automatic map import applies to build-folder analysis.


```sh
cargo run -p firmware-cli -- analyze fixtures/build/cortex-m.elf
cargo run -p firmware-cli -- files fixtures/build/cortex-m.elf
cargo run -p firmware-cli -- symbols fixtures/build/cortex-m.elf
cargo run -p firmware-cli -- diff fixtures/build/cortex-m.elf fixtures/build/cortex-m-grown.elf
cargo run -p firmware-cli -- stack fixtures/build/cortex-m.elf --stack-usage fixtures/build/CMakeFiles/cortex-m-objects.dir/src
cargo run -p firmware-cli -- analyze fixtures/build/cortex-m.elf --format json
cargo run -p firmware-cli -- analyze fixtures/build/cortex-m.elf --map fixtures/build/cortex-m.map --format json
```

CLI `analyze`, `files`, `symbols`, and `stack` accept `--map FILE`. Maps supply physical memory regions (GNU ld/TI CGT) and dependency cross references (GNU ld/LLVM lld with `--cref`). JSON reports retain the imported map path, source/object evidence, connecting symbols, and limitations. LLVM maps provide no physical capacities; these remain unknown. JSON memory-layout files and `--config` are no longer supported.

The CLI uses ELF inference when `--map` is omitted and never automatically chooses a map. Pass maps explicitly for comparisons using `diff OLD.elf NEW.elf --old-map OLD.map --new-map NEW.map`. Verify that each map belongs to its ELF. Missing/unreadable maps, unsupported formats, malformed memory regions, or malformed cross-reference tables exit with status 1 and a diagnostic on stderr. Valid maps without cross references still report memory usage and explain how to generate `--cref` connections. Argument errors (including removed `--config`) exit with status 2. Desktop automatic matching requires a unique same-name map, preferring a unique sibling; ambiguous matches never select an arbitrary map.

For optimized standalone executables:

```sh
cargo build --workspace --release --locked
```

Executables are `target/release/firmware-gui` and `target/release/firmware-explorer` (with `.exe` on Windows). Neither requires a Rust installation on the destination machine. Desktop platform libraries still apply.

## Releases and updates

The [build workflow](.github/workflows/build.yml) follows Pigtail's Windows/Linux release workflow. Run it manually on a branch, or label a PR `build`, `build-linux`, or `build-windows` to produce downloadable artifacts. Pushing `v<workspace-version>` publishes a GitHub release; the workflow rejects tags that do not match `Cargo.toml`.

Release assets include portable archives containing the GUI and CLI, standalone GUI executables for the updater, Windows MSI and Setup installers, a Debian package, and an AppImage. Linux builds also capture `snout-screenshot.png` using the regular app with `fixtures/build/cortex-m.elf` selected. There is no demo build. The screenshot runs in an isolated Xvfb session with fresh preferences and update checks disabled.

Build packages locally using `bash scripts/build-release.sh` on x86_64 Debian/Ubuntu or `scripts\build-release.cmd` from a Windows developer shell. Output goes to `target/release-assets/<target>/`. The Linux script needs the desktop build packages listed above, plus `curl`, `pkg-config`, and `dpkg-dev`; packaging downloads linuxdeploy and installs cargo-deb if needed. Windows requires Rust, the C++ Build Tools and Windows SDK; the script downloads portable WiX and Inno Setup. To capture a screenshot locally, install `xvfb xauth xdotool imagemagick`, then run:

```sh
bash scripts/capture-screenshot.sh target/release/firmware-gui target/snout-screenshot.png fixtures/build/cortex-m.elf
```

Snout checks GitHub releases at startup. **Menu → Check for updates** provides a manual check. **Support developer** opens Buy Me a Coffee, and **About** shows the app version and repository link. Startup checks stay quiet on network errors, when up to date, or for a skipped version. Download and installation begin only when you press **Update**, and the download's published size and SHA-256 digest are verified before installation. Workspace preferences are saved before installing and restored after restarting. Portable binaries update in place, AppImages replace the original AppImage, and Windows Setup installations reuse their existing installation scope. Debian installations use the package manager instead. MSI installations are identified by this product’s Windows Installer UpgradeCode and the registered executable component, and offer a **Download MSI installer** button instead of in-app replacement. Run that MSI to upgrade the registered installation. Portable copies remain eligible for self-updates even when an MSI is installed elsewhere. If Windows Installer ownership cannot be checked, in-app replacement is disabled and the downloads page remains available. `--no-update-check` suppresses the startup request for one launch.

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

Use **Menu > Snapshot** to save, compare to, overwrite, or delete named firmware snapshots. **Delete all snapshots** removes the snapshots listed for the current firmware after confirmation. Each saved snapshot shows its ELF path and capture date/time in the computer’s local timezone. The centered manager dims and blocks the background; saving shows a progress screen, and overwriting or deleting requires confirmation. The default tabs display changed sizes as `170.00 KiB (+1.00 KiB)` and changed addresses as `0x20000400 (+0x400)`. Unchanged values keep their normal display; new entries are marked `(new)`. Percentages show differences in percentage points. Snapshots are saved as individual JSON files under `snout-firmware-explorer/snapshots/<build-folder-id>/<firmware-id>/<snapshot-id>.json` in the user configuration directory. Saved snapshots survive refreshes and restarts. Baseline selection lasts only for the current session; the app starts with no baseline selected. At startup, snapshot folders are removed when their build folder is no longer listed in the saved build settings, and empty snapshot directories are pruned. Deleting a snapshot also removes any empty firmware, build, and snapshot storage directories. Folder and file names use compact stable IDs so identical names from different builds remain separate and paths stay short on Windows. Display names are stored in the JSON files. Nothing is saved to the build folder. **Stop comparing** returns to the usual display without deleting snapshots. Resetting build-folder settings preserves snapshots. Selecting a baseline automatically shows only added, removed, and changed entries in every tab. A highlighted banner identifies the active baseline. Click the **Baseline** hint in the footer to select a saved snapshot, create and select a new baseline, or choose **No baseline — show all entries** to return to the full view. Overview applies the filter to its memory breakdown; firmware totals remain the full current totals.

Source files use consistent short paths across the desktop views, keeping at least the parent directory and filename and adding parents to distinguish duplicates across the whole report. Hover shows the full absolute source path; relative compiler paths use a unique matching DWARF source or the build folder when available. ELF compilation-unit labels without a proven file location remain labels. Full paths remain searchable in file tables. The directory tree keeps its folder hierarchy and shows full source paths on hover. The desktop overview shows static RAM composition and an expandable explanation of unattributed bytes by section. BSS and reservation categories use section naming conventions; no-load storage alone does not establish how startup initializes it. Stack summaries show the largest uniquely matched local frame and explain why total stack is unknown. Reports without ELF matches remain visible when no matches exist, including stripped firmware. A loaded comparison highlights the largest Flash/RAM increases and links to the Sections, Files and Symbols comparison tables. F5 keeps the baseline snapshot while reloading current firmware and rereading selected map/JSON layouts. You can drop a build folder or an ELF into the window.

### Parsing and UI choices

- [goblin](https://docs.rs/goblin/0.10.7/goblin/elf/struct.Elf.html) exposes ELF section and program headers, including physical load addresses, directly. This avoids writing a production binary parser.
- [gimli and addr2line](https://docs.rs/addr2line/0.25.1/addr2line/) provide DWARF line lookup. `cpp_demangle` and `rustc-demangle` preserve raw symbol names alongside readable names.
- [egui/eframe](https://docs.rs/eframe/0.30.0/eframe/) provides a native Rust desktop frontend with virtualized table rows, contextual help and no web frontend build step. Versions are pinned by `Cargo.lock`; deliberate dependency upgrades should run the full checks.
- [layout-rs](https://docs.rs/layout-rs/0.1.3/layout/) arranges dependency boxes and routes curves in-process, with cached geometry drawn interactively in egui. No Graphviz installation is required. Disassembly dependencies remain deferred until there is a concrete call-graph implementation to validate.

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

ELF metadata does not conclusively identify physical memory technology. Writable, copied, and no-payload allocated sections are inferred as RAM under the bare-metal model. Memory regions imported from linker maps classify matching load/runtime ranges. A region must contain the entire range. Uncovered ranges produce warnings and retain inferred classification. In Memory map, each configured region shows its address bounds, capacity, used/free bytes and percentage used. Select a region to browse and search its symbols, including initial load images of data or code copied to RAM. Import regions through Memory map > Load memory regions by selecting a `.map` file. Occupancy counts the union of allocated load and runtime section ranges intersecting each physical region, including padding and reservations, without counting the same addresses twice. Boundary-crossing sections count only their intersection; analysis notes still flag incomplete coverage. Free space means capacity minus static ELF occupancy, not guaranteed runtime headroom or a contiguous allocation. Without map-provided regions, capacity and free space remain unknown. Stripped files retain section usage but may have no symbols.

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

1. **Attribution:** extend DWARF definition support and add GNU linker MAP object/archive ownership. Map import reads region capacities and GNU ld / LLVM lld symbol cross references for the dependency graph; section/object memory accounting is not yet imported. Variable attribution supports direct address expressions; location lists, TLS, complex expressions and definitions requiring reference resolution remain unsupported. Global variables without a matching supported definition remain unattributed. Compressed and split/external DWARF are not supported.
2. **Memory layouts:** add saved target profiles, explicit unknown memory roles, overlay policies and segment-only fallback. Relocatable objects, overlapping ordinary allocated/load ranges and sectionless ELFs currently return clear unsupported errors. HEX/BIN analysis and standalone MAP symbol/section analysis are not supported. Dynamic-symbol-only attribution is not yet implemented.
3. **Comparisons:** normalize source roots across build machines, improve duplicate/renamed symbol matching and introduce simple CI budgets. Matching currently uses file label, section and mangled symbol name, with duplicate identities aggregated. Differences in attribution or debug availability can affect per-file/symbol deltas.
4. **Stack:** import evidenced call graphs and represent recursion, indirect calls, assembly, interrupts and missing data before estimating call chains. The model reserves these uncertainty categories. CFA/disassembly, RTOS task stacks and runtime high-water marks are later inputs.
5. **Desktop delivery:** validate on representative user firmware and Linux desktops, refine large-report performance, add installers, accessibility/keyboard review and preferences. Source tables currently rebuild their display rows each frame, although visible rows are virtualized. Region occupancy and largest-contributor rankings are cached per analysis. Workspace preferences remember the folder, firmware, layout, view and overview memory selection. Refresh (F5) rescans and reloads the selected firmware while preserving the last successful report on failure. Preferences are stored in `snout-firmware-explorer/workspace.json` under the user configuration directory (APPDATA on Windows, XDG_CONFIG_HOME or ~/.config elsewhere). Use **Menu > Open configuration folder** to open Snout’s settings and snapshot directory in your system file explorer. No export dialog is included yet; use CLI JSON for export.

No VS Code dependency is present. A future extension or CI service can consume core reports or the CLI without the desktop frontend.

### Thread-local storage

Generic TLS analysis uses standard ELF `PT_TLS`, `SHF_TLS`, and `STT_TLS` metadata. Reports show the initialized and zero-initialized template sizes, alignment, and TLS variables with template offsets. When `PT_TLS` is absent, the template extent is inferred from allocated TLS sections, including internal gaps. Initialized section payload remains in Flash accounting; TLS templates are excluded from static RAM and physical runtime ranges. ELF template addresses may overlap ordinary runtime sections.

The template size is a per-thread requirement, not a complete allocation size. Total TLS RAM remains unknown: the ELF does not establish live thread count, ABI/runtime overhead, or whether TLS occupies already-counted stack reservations. No RTOS-specific assumptions are applied. TLS variables are listed separately; DWARF source attribution for TLS remains unsupported.
