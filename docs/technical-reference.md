# Firmware Explorer technical reference

Detailed behavior, accounting rules, limitations and development workflows for contributors and coding agents. Start with the [README](../README.md) for a user introduction. Use the source and tests to verify behavior when changing the application; this document describes the current implementation, not a list of promised features.

## Reference index

- [Opening firmware and selecting supporting files](#opening-firmware-and-selecting-supporting-files): discovery, matching, imports, views and CLI usage.
- [Releases and updates](#releases-and-updates): builds, packaging and installation behavior.
- [Architecture](#architecture): crate boundaries, workspace persistence and snapshots.
- [What the numbers mean](#what-the-numbers-mean): Flash, static RAM, regions and hex contents.
- [Attribution and uncertainty](#attribution-and-uncertainty): ownership, aliases and stack evidence.
- [Validation](#validation): checks and fixture coverage.
- [Current boundaries and next milestones](#current-boundaries-and-next-milestones): unsupported inputs and planned work.
- [Thread-local storage](#thread-local-storage): template accounting and unknown runtime allocation.

A standalone Rust desktop application and CLI for understanding embedded firmware memory usage. The first milestone targets **ARM Cortex-M and GCC**, with generic ELF models that leave room for other architectures.

## Opening firmware and selecting supporting files

Install a current stable Rust toolchain (Rust 1.95 or newer for the desktop application). Windows builds need the Visual Studio C++ Build Tools and Windows SDK. Linux desktop builds need a working X11 or Wayland session and development packages for Wayland and xkbcommon (on Ubuntu: `libwayland-dev libxkbcommon-dev`). The desktop uses wgpu, with Vulkan or OpenGL on Linux and DirectX 12 by default on Windows. Windows excludes Vulkan before adapter enumeration to avoid crashes in older Intel Vulkan drivers, following Pigtail’s Windows compatibility fix. Explicit `WGPU_BACKEND` overrides remain available for diagnostics; `WGPU_POWER_PREF` and other upstream settings are preserved. There is no automatic Vulkan fallback on Windows. The renderer allows two queued frames and uses automatic presentation without vsync on Linux to avoid compositor waits blocking the UI thread; it retains automatic vsync on Windows. Redrawing remains driven by input, animations and background updates. Startup logs identify the selected graphics backend, adapter and driver. Linux Vulkan rendering needs a Vulkan loader and a suitable GPU driver (on Ubuntu: `libvulkan1` and the vendor driver or `mesa-vulkan-drivers`). The CLI and core do not require desktop libraries.

```sh
cargo build --workspace --locked
cargo run -p snout -- fixtures/build/gcc --elf cortex-m.elf
```

Pass `--elf FILE` to select firmware automatically after the folder scan. Relative file paths are resolved inside the supplied build folder. You can also open an ELF directly: `snout fixtures/build/gcc/cortex-m.elf`. With no path, Snout opens the last build folder and restores the saved workspace. Opening the same build folder without an explicit ELF selection restores the last selected firmware if it is still present. An explicit ELF selection takes priority; a missing saved ELF leaves the folder open for selection.

**Menu** is anchored at the bottom-left of the sidebar and contains folder, configuration, update, analysis-note, support and About actions. Select **Open build folder**, press Ctrl+O, or drag a folder into the window. **Menu > Open recent build folder** remembers the five most recently opened folders across restarts. Recent-folder entries show complete paths on one line. The submenu sizes itself to the longest path within the space beside the parent menu; paths wider than that space can be scrolled horizontally. Reopening a folder moves it to the top without adding a duplicate. The application recursively scans for linked ELF images (including `.elf`, `.axf`, `.out` and extensionless images), linker maps (`.map`) and stack reports (`.su`). The header’s ELF dropdown lists each firmware filename above its full path in smaller muted text, matching the selected firmware in the header. Long paths in the header and dropdown are truncated at the start with an ellipsis, preserving the filename and final folders; hovering shows the complete path. Select an image to analyze it; selection is disabled during background loads. **Build files**, directly below **Overview** in the left sidebar, opens a dedicated selection tab for the current ELF. Two bordered panels separate the linker map from compiler stack reports, with a shared search field and selected/report status. Clicking a map filename or its radio button selects that map directly; clicking the active map does nothing. Stack-report filenames and folder checkboxes toggle inclusion. The selected map supplies both physical memory regions and supported symbol dependencies; choosing another map replaces both inputs. **No map selected** uses ELF inference and clears imported dependencies. Missing cross references leave ELF-derived units visible with an explanation. Legacy independently saved dependency-map choices are ignored; existing memory-map choices remain valid. Files are not opened as text inside the application. Use the sidebar to switch views. Build files retains folder-scan notes, including unreadable directories and artifacts, before and after firmware selection. Folders without firmware list supporting files but require firmware selection before they can be applied. Failed selections retain the last successful firmware report. Select **Menu > Refresh**, click the reload icon in the header, or press F5 after rebuilding. The header reload button is replaced by the analysis spinner while loading, with “Analyzing...” right-aligned just to its left when space permits, and is disabled until a build folder is open. A background worker checks only the selected ELF’s metadata every two seconds, without reading file contents or rescanning the folder. A detected change to an available file turns the reload icon amber and adds a small dot; its tooltip explains that the analysis may be stale. The indicator remains until a successful ELF reload, including through failed refreshes, but is hidden while the file is missing or its metadata is unavailable. Checks continue during those periods so a changed file is indicated when it reappears. Checks stop when no firmware is selected. The comparison uses file size and modification/creation timestamps, plus file identity and change timestamps on Unix; changes preserving all tracked metadata may go undetected. Maps and stack reports are not monitored, and changes never trigger automatic analysis. Menu shortcut hints also show **Ctrl+O** to open a build folder and **F1** for About.

Selecting firmware automatically imports memory-region capacities from a matching GNU ld or Texas Instruments CGT map. Automatic selection parses GNU ld, TI CGT and LLVM lld maps into a common output-section representation, then uses one format-independent matcher to compare names, runtime addresses and sizes against every nonempty allocated ELF section, requiring at least two matching sections and complete coverage. Map load addresses must also agree with load addresses established by ELF PT_LOAD segments for file-backed sections. GNU rows without a load-address annotation establish equal load/runtime addresses; TI origins establish load addresses even without a RUN ADDR annotation. No-payload ELF sections supply no load-address evidence. This works with renamed maps and stripped ELFs; discarded/input sections, symbols and debug-only sections do not supply matching evidence. Maps with conflicting section placements or incomplete evidence are excluded. A unique content match takes precedence over filenames. If multiple maps have matching layouts, unique filename evidence (`app.map` or `app.elf.map` for `app.elf`, preferring a sibling) can disambiguate them; otherwise no map is selected. Maps without a supported section-placement table retain the unambiguous filename fallback; malformed supported tables are rejected. TI section-allocation parsing supports wrapped output names, their optional leading `*` placement marker, and RUN ADDR annotations; parsing stops before module summaries. It excludes nested input rows and retains explicit load addresses separately. Word-addressed TI targets and overlapping PAGE spaces remain unsupported. Region coverage alone is insufficient evidence. Analysis notes state the matching basis. Matching is evidence, not proof that artifacts came from the same build; maps with identical section layouts can still differ in other contents. Maps and ELF section headers are reread on selection and refresh. Ambiguous or unsupported maps leave capacity unknown and produce analysis notes. LLVM maps can supply dependencies but never physical capacities. Use a map’s radio button in **Build files** to select its memory regions. Manual map selection checks the same ELF section evidence on the worker thread. If the map conflicts, has incomplete evidence, or cannot be verified, a centered modal dims and blocks the application and lists the reasons, including section addresses and sizes where available. Long filenames in the warning stay on one line, with complete paths available on hover. **Ignore** applies the map; **Revert** or Escape keeps the previous selection and report. Clicking the backdrop leaves the warning open. Matching filenames do not suppress this check. Manual map choices are remembered for each firmware image in each build folder across restarts. Chosen maps are read again when reopening or refreshing firmware after a rebuild. The map currently supplying memory regions has its radio button selected in **Build files**. **Menu > Reset settings for this build folder** clears that folder’s saved choices and rediscovers the current firmware’s matching map. Linker scripts are not listed or evaluated. Memory roles are inferred from map region names and attributes. See the [GNU linker documentation](https://sourceware.org/binutils/docs/ld/Options.html) for generating a link map.

Map parsing automatically identifies GNU ld, Texas Instruments CGT, and LLVM lld ELF formats from their contents. TI CGT import supports old `attributes` tables and newer `used / unused / attr` tables, hexadecimal values, and non-overlapping `PAGE` groups. Capacities come from region lengths; usage remains calculated from the ELF. Explicit FLASH/ROM names take precedence over TI's often-permissive `RWIX` attributes. Empty TI ranges are skipped. Overlapping page address spaces and recognized word-addressed C28x/C54x/C55x targets are rejected because the current memory model requires one byte-addressed physical space. TI output-section placements support automatic matching, but TI cross references and section/object attribution are not imported. Unknown formats are identified as unsupported. See [TI's linker-map example](https://software-dl.ti.com/codegen/docs/tiarmclang/rel3_1_0_STS/compiler_manual/linker_description/12_linker_example/linker-example-stdz0759199.html).

LLVM lld ELF maps (`VMA LMA Size Align Out In Symbol`) provide parsed output-section placement with runtime/load addresses and sizes for automatic matching, plus dependency import from `--cref` tables. Generate these with `-Wl,-Map,app.map,--cref,--no-demangle` when linking with lld. Automatic matching and manual map selection in **Build files** both work. LLVM maps do not report physical region capacities: physical capacities remain unknown. Map placement does not override ELF analysis. LLVM's COFF, Mach-O, and WebAssembly map dialects are not supported. See [LLVM's ELF map writer](https://github.com/llvm/llvm-project/blob/main/lld/ELF/MapFile.cpp).

ELF/DWARF information always takes precedence over map information for symbol and source attribution. Maps supplement the ELF with region capacities and supported linker cross references; they never overwrite DWARF ownership or symbol sizes/addresses.

When firmware is selected, Snout guesses `.su` reports from linker-map object paths, matching target directories, or unambiguous ELF function matches. Review or change the selection with file and recursive directory checkboxes under **Stack usage files (.su)** in the **Build files** tab. Choices are saved per ELF immediately and restored on refresh and restart, including an empty selection. Selected directories include future reports; selecting a file includes future siblings while preserving existing unchecked files. Reports can span several targets or configurations, so verify their build ownership. The Stack view defaults to entries with candidates in the selected ELF and preserves report paths and matching evidence. Gray bars behind Local frame byte counts compare each row with the largest visible frame: a full cell is 100%, and half the bytes fills half the cell. Enable **Show unresolved reports** to inspect entries that could not be associated. Functions without matching reports are counted with unknown local stack size. No call-chain totals are invented.

The dark navy interface uses green accents, a firmware dropdown and left navigation. Navigation buttons highlight on hover; the selected view has a green edge. The Menu row reserves separate space for its three-line icon and label, with the full row opening the menu. The left navigation includes Overview, Build files, Baselines, Files, Symbols, Sections, Memory regions, Dependencies, Stack, and Hex viewer. **Build files** is a normal sidebar tab directly below **Overview**, used to select analysis inputs rather than view or edit file contents. Interface text has a 12 pt minimum; labels inside the zoomable dependency diagram scale with zoom.

Overview fits six bordered cards into the available screen without a scrollbar: Binary information, Memory usage, Flash and RAM Section type distributions, Top functions by size, and Top files by size. Binary information, Memory usage and both distributions occupy the upper row; two wider rankings occupy the lower row. Card titles wrap in narrower windows. The upper row reserves enough height to keep both memory-region controls and the regions link visible at the minimum 900 × 600 window size. Rankings show up to ten entries, adapting the visible count to available height; links open the full tables. Functions include only ELF Function symbols, ranked by unique Flash contribution. Files exclude unattributed storage and are also ranked by unique Flash contribution. Row tooltips retain complete names, addresses and byte counts; clicking a row opens its symbols. Binary information aligns labels and values in two columns. Each section-distribution donut uses smooth circular edges and a palette assigned in descending size order, independent of section names, with distinct colors for all visible slices and a matching, aligned name/percentage legend. The donuts show current Flash payload and static RAM usage respectively, with Flash and RAM center labels, grouping sections after the largest five as Other. Legends sit below the donuts in narrow cards so section names and percentages remain readable; wider cards place legends beside the donuts. Their legends open the Sections table and enable address changes while comparing, so moved sections remain visible. Snapshot filtering does not change the donuts, while rankings retain the Overview difference filter and size annotations. Clicking a ranking row or **View all functions** while comparing enables address changes in the Symbols table so address-only changes remain visible in the drilldown. Memory usage keeps full Flash payload and static RAM totals in the block tooltips, separately from physical occupancy. Each block starts directly with a region dropdown and a green Flash or blue RAM occupancy bar, with percentage inside the bar and used/capacity and free bytes below. The dropdown shows only the region name. It defaults to the region of that kind with the highest occupancy percentage (ties keep layout order); an explicit selection stays by name while that region remains available. Selections are saved immediately per ELF path within its build folder and restored on refresh, firmware switches and restarts. If a saved name no longer exists for that memory kind, the highest-occupancy region is shown instead; if no regions of that kind exist, capacity is unknown. Preferences without saved selections use the default without a migration. When no physical regions are available, the Memory usage card shows the known Flash payload and static RAM totals without occupancy bars, followed by one amber warning and an **Open Build files** link. The warning distinguishes no selected map from a selected map without physical capacities. If only one memory kind lacks a configured region, that kind still shows its unknown capacity explicitly. View all regions opens Memory regions, clears its search filter, and enables address changes while comparing so moved regions remain visible. Tooltips retain complete byte counts and explain runtime uncertainty. The header ELF selector shows a file icon, filename, firmware path, and chevron in a two-line button; its popup lists available firmware with a consistent row height for scrolling, including long lists. Beside it, the header shows the analyzed architecture and bit width with their icons. The architecture and bit-width details are measured before layout. Bit width and its icon are hidden first when space is tight; architecture and its icon are hidden only if they also cannot fit. Fit checks use the fixed boundary before reload, rather than a layout width that overflowing content can expand. The selected-baseline badge reserves space before sizing the ELF selector, keeping the comparison visible and its clear action usable at the minimum window width. The ELF selector retains its preferred 470-logical-pixel width while these details are hidden; it narrows only after both have disappeared and the selector itself cannot fit beside the baseline badge. Both details reappear as space becomes available. Reload remains right-aligned beside the window controls. Unused header space, including the gap between reload and the window controls, is available for dragging. Header content is clipped to its reserved area so it cannot paint beneath reload or the window controls.

Analysis notes remain available from **Menu > Analysis notes**; detailed sizes and attribution remain in the Files, Symbols, Sections and Memory regions views.

Tables support search and sorting, with right-aligned numeric columns. Columns fit their content when space allows and share spare width in smaller windows. Widths update when the window or table data changes; manual resizing remains available. Gray bars in the Size/ELF size, Flash and RAM columns of Sections, Symbols and Files compare each value with its own column’s largest visible value. Click a row to expand its details directly underneath; click it again to collapse. Expanded file rows include a button to inspect their symbols. Enable Directories for an optional source tree sidebar. Baseline cards highlight on hover and mark the selected comparison; **None** in the Baselines tab clears it; a header badge shows the current selection and clears it when clicked. The badge is hidden when no baseline is selected. Escape collapses the expanded row and notes. Firmware stays local. The CLI continues to accept individual ELF paths.


The **Dependencies** tab shows a hierarchical compilation-unit graph with clickable boxes and curved arrows, pan/zoom, directory grouping, search, and a focused neighborhood view. It flows top-to-bottom in tall canvases and left-to-right in wide canvases. Box area scales linearly from 0 B at a minimum size measured from the labels to the largest visible Flash/RAM contribution at 25 times that minimum area. Labels grow with their boxes, and each box displays its byte count. Large graphs wrap broad dependency ranks into compact bands, with nearby connections routed through shared gutters. Filtering shows matching units and their directly connected dependencies and dependents, within the selected-unit focus when enabled. Switching memory modes or filtering recalculates and fits the layout. Arrows run from a unit using a symbol to the unit defining it; selecting an arrow lists the connecting symbols. Sizes count attributed symbol bytes, excluding padding and unknown ownership. DWARF compilation-unit ownership takes priority over ELF unit labels; source locations in headers do not create units.

To supply connections, generate a GNU ld or LLVM lld ELF linker map with `-Wl,-Map,app.map,--cref,--no-demangle`. A unique matching map loads automatically with firmware. Alternatively, select a map in **Build files**; that same map supplies its memory regions and cross references when supported. The single map choice is saved per firmware and reread on F5 and restart. Failed selections or unreadable saved maps preserve the last successful report. Selecting a valid map without cross references clears the previous map's connections and leaves ELF-derived units visible with a note.

Cross references include functions, global data, and function addresses, and may include discarded code; they do not prove runtime calls. Source-unit connections require a global ELF definition of the connecting symbol; symbols missing from the ELF are omitted because they may be unresolved or discarded. Object-only graphs cannot distinguish unresolved weak references from definitions in GNU ld cross-reference tables, so their arrows need verification. Object/archive paths associate with source units only through exact ELF symbol definitions with one consistent unit owner. Ambiguous or unknown objects remain separate nodes with unknown memory contribution. LTO and stripped builds can limit source association. Without cross references, the graph shows known units and explains that connections are unavailable; an isolated node is not proof of independence. The committed Cortex-M fixtures include six source units and sixteen connections, including reciprocal dependencies; open `fixtures/build/gcc/cortex-m.elf` from the build folder to explore them. Weak aliases can inherit ownership from an exact function range with one known unit; conflicting owners remain ambiguous. ELF-only CLI JSON includes unit nodes in the additive `dependencies` field; automatic map import applies to build-folder analysis.


```sh
cargo run -p snout-cli -- analyze fixtures/build/gcc/cortex-m.elf
cargo run -p snout-cli -- files fixtures/build/gcc/cortex-m.elf
cargo run -p snout-cli -- symbols fixtures/build/gcc/cortex-m.elf
cargo run -p snout-cli -- diff fixtures/build/gcc/cortex-m.elf fixtures/build/gcc/cortex-m-grown.elf
cargo run -p snout-cli -- stack fixtures/build/gcc/cortex-m.elf --stack-usage fixtures/build/gcc/CMakeFiles/cortex-m-objects.dir/src
cargo run -p snout-cli -- analyze fixtures/build/gcc/cortex-m.elf --format json
cargo run -p snout-cli -- analyze fixtures/build/gcc/cortex-m.elf --map fixtures/build/gcc/cortex-m.map --format json
```

CLI `analyze`, `files`, `symbols`, and `stack` accept `--map FILE`. Maps supply physical memory regions (GNU ld/TI CGT) and dependency cross references (GNU ld/LLVM lld with `--cref`). JSON reports retain the imported map path, source/object evidence, connecting symbols, and limitations. LLVM maps provide no physical capacities; these remain unknown. JSON memory-layout files and `--config` are no longer supported.

The CLI uses ELF inference when `--map` is omitted and never automatically chooses a map. Pass maps explicitly for comparisons using `diff OLD.elf NEW.elf --old-map OLD.map --new-map NEW.map`. Verify that each map belongs to its ELF. Missing/unreadable maps, unsupported formats, malformed memory regions, or malformed cross-reference tables exit with status 1 and a diagnostic on stderr. Valid maps without cross references still report memory usage and explain how to generate `--cref` connections. Argument errors (including removed `--config`) exit with status 2. Desktop automatic matching compares output-section placement with the ELF before falling back to unique same-name maps without section evidence; ambiguous matches never select an arbitrary map.

For optimized standalone executables:

```sh
cargo build --workspace --release --locked
```

Executables are `target/release/snout` and `target/release/snout-cli` (with `.exe` on Windows). Neither requires a Rust installation on the destination machine. Desktop platform libraries still apply.

## Releases and updates

The [build workflow](../.github/workflows/build.yml) builds Windows and Linux on
pull requests, pushes to `main`, manual dispatch and reusable calls. It produces
downloadable artifacts without build labels and never publishes a release.
Feature-branch pushes build through the PR event, avoiding duplicate builds.
New PR commits cancel older runs for that PR; main pushes and manual runs build
independently. The [Store workflow](../.github/workflows/store.yml) is likewise
reusable and runs on relevant PRs, main pushes and manual dispatch. Neither build
workflow handles version-tag pushes independently.

Following Canvaz's workflow split, [release.yml](../.github/workflows/release.yml)
handles `v*` tag pushes and manual runs against an existing version tag. It rejects
branch refs and tags that differ from `Cargo.toml`. It calls both reusable build
workflows, waits for **both platforms and Store packaging** to succeed, then
publishes the privacy policy from that commit. The final release job waits for
all these jobs; failed, cancelled or skipped prerequisites block publication.
Runs for the same tag serialize without cancelling an in-progress release.

After both platform artifacts and the completed Store submission ZIP download,
`scripts/publish-release.sh` creates or
reuses a private draft, uploads all assets, and publishes only as its final
operation. An upload failure leaves an unpublished draft for retry. A rerun
replaces incomplete draft uploads but refuses to change an already published
release. Failed artifact downloads never reach draft creation. Normal PR/main
builds and manual build/Store runs never create or publish GitHub releases.

The header displays the pig artwork at 64 × 64 logical pixels, with the two-line application title vertically centered beside it. The header, About dialog, native window icon, Windows executable and Setup installer, and Linux desktop packages use the same pig artwork from `crates/snout/packaging/icons/snout.svg` (the supplied `pigbot-vector.svg`). After editing the SVG, install CairoSVG (`python3 -m pip install CairoSVG`) and run `python3 scripts/generate-icons.py` to regenerate the committed 512 px PNG and Windows ICO (16–256 px). Normal builds use the committed assets and do not require CairoSVG.

The desktop window uses an application-drawn frame instead of OS decorations. Minimize, maximize/restore and close controls stay at the right of the header, with a divider separating them from reload. Buttons highlight on hover and press; close highlights red. Window controls remain usable while application modal dialogs are open; those dialogs continue to block application actions. Drag unused header space to move the window, or double-click it to maximize/restore. Native movement starts once egui recognizes a drag, preserving real press/release timing for header clicks across intervening repaints. The thin visible border has invisible resize targets extending 8 logical pixels inward, with diagonal resizing available along the first 28 logical pixels of each edge near a corner. Corner targets follow the edge bands so nearby content remains clickable. Directional cursors identify the active resize direction; resizing is disabled while maximized or fullscreen. Window movement and resizing use the desktop window manager. After handing off a move or resize, the input hook releases egui’s pointer capture if the native backend consumes mouse-up, and ignores a delayed duplicate release so hover and subsequent clicks remain responsive. The native title and icon remain available to task switchers. Headless tests verify viewport commands and hover rendering, but native window-manager behavior requires platform testing.

Release assets include `store-submission.zip` for maintainers, portable archives containing the GUI and CLI, standalone GUI executables for the updater, Windows MSI and Setup installers, a Debian package, and an AppImage. Linux builds also capture `snout-screenshot.png` using the regular app with `fixtures/build/gcc/cortex-m.elf` selected. There is no demo build. The screenshot runs in an isolated Xvfb session with fresh preferences and update checks disabled.

The Debian package is named `snout` and installs the GUI as `/usr/bin/snout`. Its desktop launcher runs `snout`; the AppImage uses the same executable name internally. Cargo builds and portable archives also use `snout` (`snout.exe` on Windows).

The separate [Request APT publication workflow](../.github/workflows/publish-apt.yml)
runs after a successful **Release** workflow through `workflow_run`, or on manual
dispatch for recovery. It requests `publish-apt.yml` on `main` in the shared
[Rusty APT repository](https://github.com/rustypig91/rusty-apt), which imports
published release Debian assets, signs repository metadata and deploys APT.
APT reconciliation must follow GitHub publication because it consumes those
assets; a downstream APT failure does not undo a published release. No APT job
runs after publication inside the Release workflow itself. PRs and manual
build/Store runs do not dispatch APT publication. This repository needs the
`APT_PUBLISH_TOKEN` Actions secret with Actions write access only to
`rustypig91/rusty-apt`; signing and APT Pages configuration belong to that shared
repository. A failed dispatch can be retried through **Request APT publication**
or the shared repository's **Publish APT** workflow. The `workflow_run` listener
must be on the default branch to receive completion events and checks out no
source code.

Build packages locally using `bash scripts/build-release.sh` on x86_64 Debian/Ubuntu or `scripts\build-release.cmd` from a Windows developer shell. Output goes to `target/release-assets/<target>/`. The Linux script needs the desktop build packages listed above, plus `curl`, `pkg-config`, and `dpkg-dev`; packaging downloads linuxdeploy and installs cargo-deb if needed. Windows requires Rust, the C++ Build Tools and Windows SDK; the script downloads portable WiX and Inno Setup. To capture a screenshot locally, install `xvfb xauth xdotool imagemagick libvulkan1 mesa-vulkan-drivers`, then run:

```sh
bash scripts/capture-screenshot.sh target/release/snout target/snout-screenshot.png fixtures/build/gcc/cortex-m.elf
```

Direct-download desktop builds check GitHub releases at startup. **Menu → Check for updates** provides a manual check. **Privacy policy** opens the public policy, **Support developer** opens Buy Me a Coffee, and **About** shows the app version and repository link. Startup checks stay quiet on network errors, when up to date, or for a skipped version. Download and installation begin only when you press **Update**, and the download's published size and SHA-256 digest are verified before installation. Workspace preferences are saved before installing and restored after restarting. Portable binaries update in place, AppImages replace the original AppImage, and Windows Setup installations reuse their existing installation scope. Debian installations use the package manager instead. MSI installations are identified by this product’s Windows Installer UpgradeCode and the registered executable component, and offer a **Download MSI installer** button instead of in-app replacement. Run that MSI to upgrade the registered installation. Portable copies remain eligible for self-updates even when an MSI is installed elsewhere. If Windows Installer ownership cannot be checked, in-app replacement is disabled and the downloads page remains available. `--no-update-check` suppresses the startup request for one launch.

### Microsoft Store edition

The `microsoft-store` Cargo feature builds a separate desktop edition. It replaces
both GitHub update menu controls with **Open in Microsoft Store** and an update
management explanation. The URI is `ms-windows-store://pdp/?productid=9NWM3QW158VZ`.
Startup/manual GitHub checks, downloads, and installation are rejected before
workers start, even when saved preferences enable checks. Replacement workers
also enforce the Store policy. Windows and Store settings control external
updates; Snout does not request Store installation, termination or restart.
GitHub and Store releases can publish at different times.

Store preferences and snapshots use `snout-firmware-explorer-store` instead of
`snout-firmware-explorer`, with the same existing configuration-root selection.
On Windows the requested preferences path is
`%APPDATA%\snout-firmware-explorer-store\workspace.json`, with `snapshots` beside
it. Windows package virtualization may redirect these into per-user package
storage. **Open configuration folder** opens the path visible to that edition.
Neither edition imports data automatically. The workspace/snapshot schemas and
original direct-download paths are unchanged.

To migrate manually, launch and close the Store edition first, close both editions,
back up the destination, then copy **both `workspace.json` and the complete
`snapshots` directory** to the Store configuration directory. Do not copy snapshots
alone: startup cleanup uses the workspace's saved build settings to identify
retained snapshot folders. Do not merge live files. Back up important baselines
before uninstalling; Store uninstall retention is not guaranteed.

The reserved product is **Rusty's Snout - Firmware Explorer**:

| Field | Value |
| --- | --- |
| Identity Name | `rustypig91.RustysSnout-FirmwareExplorer` |
| Identity Publisher | `CN=B41BE868-9458-4B44-AE02-EA569AC9D53B` |
| PublisherDisplayName | `rustypig91` |
| Product ID | `9NWM3QW158VZ` |

The manifest at `crates/snout/packaging/windows/msix/AppxManifest.xml` packages
`snout.exe` as an x64, medium-integrity `packagedClassicApp` with `runFullTrust`,
for Windows 10 build 19041 or newer. This retains normal desktop file access and
native dialogs. It installs no drivers, services, elevation helper or updater.
The Store package contains the GUI and license; the CLI remains in the existing
direct-download archives. ARM64/x86 packages and CLI execution aliases are not
produced.

On Windows with Rust, Visual Studio C++ Build Tools and the Windows SDK, run:

```powershell
./scripts/build-msix.ps1
```

The script reads the `snout` version using Cargo metadata and appends `.0`.
For example, app `1.0.0` produces package `1.0.0.0`. There is no independent
package-version setting or override. Releases must use numeric `major.minor.patch`,
a nonzero major, and components no larger than 65535. Prerelease/build suffixes
are rejected. Increase the app version before submitting a newer Store package.
The manifest retains `@VERSION@` until packaging fills it.

The script builds only the Store feature with a static MSVC CRT in `target/store`,
independent of direct-download binaries. It generates 50/44/150-pixel logos and
200% variants from the committed icon, indexes them with MakePri, and packs with
MakeAppx validation enabled. Staging is unique and removed even on failure;
Cargo environment overrides and the caller's location are restored even if
cleanup fails. Relative `-OutputDirectory` paths resolve from the repository root.
Encoded compiler flags, including explicitly empty flags, take precedence over
plain flags; plain flags are split without empty arguments.

The unsigned output is
`target/release-assets/store/snout-store-<app-version>.0-x64.msix`, accompanied by
`.msix.sha256` and `.msix.build-info.json` sidecars. The metadata records app and
package versions, build target and MSIX SHA-256. Upload the MSIX to Partner Center;
Microsoft signs it during Store publication. It is not a public unsigned sideload
installer and needs no signing-certificate secret.

The separate **Microsoft Store MSIX** workflow tests the Store edition, packaging
script and submission archiver, checks tag/version consistency, and builds on
relevant PRs, main pushes, manual dispatch and reusable calls from the tag-release
workflow. It uploads `snout-store-msix-<run-id>` with the MSIX and sidecars.
Every successful Store run also creates `store-submission.zip` and uploads it as
`snout-store-submission-<run-id>`. Artifact replacement supports reruns.

Following Canvaz, `scripts/package-store-submission.py` bundles the MSIX, checksum,
build metadata, all committed listing PNGs and original icons, the manifest
reference, license, privacy policy, listing/reviewer notes and submission
instructions. It verifies that the package matches both digest sidecars before
archiving, requires exactly one MSIX and all referenced materials, and replaces
an existing ZIP only after a complete archive is written. Missing/corrupt package
files or missing artwork fail the Store job and block release publication.
The release workflow downloads and attaches this completed ZIP without repacking.
An archive upload/download failure also prevents GitHub publication.

This ZIP is a maintainer bundle: extract it and upload its **contained MSIX** to
Partner Center. CI does not submit or publish the Store listing. Windows Store
screenshots still need to be captured for submission; the Linux fixture screenshot
remains a separate GitHub release asset. The local archiver accepts an optional
`--screenshot` for reference but does not require one. To archive a local MSIX build:

```powershell
python scripts/package-store-submission.py --package-directory target/release-assets/store --output target/release-assets/store-submission.zip
```

The existing build workflow keeps producing the direct-download Windows/Linux
packages. These retain their normal installation and update behavior.

### Store listing and privacy publication

`docs/store-listing.md` contains listing text and reviewer instructions.
`crates/snout/packaging/windows/store-listing/` contains app tiles (71/150/300 px),
box art (1080/2160 px square) and poster art (720 × 1080 / 1440 × 2160).
Regenerate with `python3 scripts/generate-store-images.py` (Pillow required).
These are separate from manifest assets generated by the Windows packaging script.
Use current screenshots, including the existing fixture-based Overview capture;
confirm current Partner Center upload requirements when submitting.

`docs/site/privacy/index.html` covers local firmware processing, saved metadata,
retention, support correspondence and distribution-specific network behavior.
Keep the content and visible date aligned with app behavior. Both editions link
to `https://rustypig91.github.io/snout-firmware-explorer/privacy/` from the menu.

The reusable **Publish privacy policy** workflow publishes only `docs/site` to
GitHub Pages after the application and Store builds succeed, before the tagged
GitHub release is published. Deployment failure blocks release publication. Manual
dispatch remains available for first publication and policy-only updates.
Repository **Settings → Pages** must use
**GitHub Actions**. The `github-pages` environment must allow `v*` tags and the
manual-publication branch (normally `main`). The URL becomes live after a
successful deployment. Publishing replaces this repository's Pages site; keep
future pages under `docs/site` too. The release workflow calls it directly as a
required dependency.
Deployment runs serialize; artifacts use run/attempt names, with successful build
outputs retained for deploy-only retries. No custom token or domain is required.

Before submission, test the package on Windows with package identity. From a
Windows SDK Developer PowerShell, unpack to a persistent test directory:

```powershell
MakeAppx.exe unpack /p <package.msix> /d <test-directory>
Add-AppxPackage -Register <test-directory>\AppxManifest.xml
```

Development registration may require Developer Mode; retain the test directory
while registered and launch the Start menu entry. Check launch/icons and Windows
DX12 selection, folder/ELF opening, native dialogs, drag/drop, maps/stack reports,
baseline persistence/deletion, configuration-folder opening, Store/privacy links,
and independent storage with the direct edition installed. Test an upgrade with
saved settings and baselines present, and run the Windows App Certification Kit.
Final Store installation/signing/update delivery requires separate native testing;
headless tests and local development registration do not verify those behaviors.

Complete Partner Center availability/pricing, properties, age ratings, listing,
screenshots, support/privacy URLs and certification notes. Explain `runFullTrust`
as normal Win32 execution with native file dialogs and user-selected firmware
access. No connected hardware or ARM toolchain is required to inspect the committed
fixtures. Review the submission before publishing.

References: [manual desktop packaging](https://learn.microsoft.com/en-us/windows/msix/desktop/desktop-to-uwp-manual-conversion),
[Store package/version rules](https://learn.microsoft.com/en-us/windows/apps/publish/publish-your-app/msix/app-package-requirements),
and [submission readiness](https://learn.microsoft.com/en-us/windows/apps/publish/store-readiness-checklist).

## Architecture

```text
snout (egui/eframe) ─┐
                    ├── snout-core
snout-cli (clap) ────┘       ├── ELF adapter: goblin
                            ├── DWARF: gimli + addr2line
                            ├── accounting / attribution / tree
                            ├── comparison
                            └── compiler stack reports / call-graph model
```

The desktop header shows the selected firmware, its architecture with a chip icon, and its bit width with a register-width icon. Architecture and bit width come from the ELF metadata; hovering the bit width explains its source. The Menu popup, recent-folder submenu and ELF dropdown use the application’s dark palette, rounded controls, padded rows and green interaction accents. The selected ELF row has a green filename, tinted background and green edge matching the sidebar navigation.

The core accepts a path or byte slice plus analysis options and returns a serializable `Analysis`. Parser types stay internal. Frontends do not perform accounting. GUI analysis runs on a worker thread, and failed loads preserve the last successful report. Memory configuration changes are validated and applied together with a successful reanalysis.

The CLI's `analyze`, `files` and `symbols` JSON modes return the same complete report envelope, including `schema_version`, options, warnings, sections, symbols, file totals and address ranges. `diff` and `stack` have their own versioned envelopes. Comparison reports include section, file and symbol deltas; same-name sections are aggregated. Sizes and addresses are exact integers; signed comparison deltas are bytes. Exit status is zero for successful analysis (including documented missing information), nonzero for invalid input or command usage. No CI policy engine is included yet.

Use **Baselines** in the left sidebar to save, overwrite, or delete named firmware snapshots. Click a saved baseline card in the Baselines tab to select it; click the selected card again or select **None** to clear comparison. **Delete all baselines** removes the snapshots listed for the current firmware after confirmation. Each baseline card shows a large name on the left, smaller muted capture date/time and ELF path beside it, and a bin icon on the right. Capture times use the computer’s local timezone. The bin icon opens deletion confirmation without changing the selected comparison. The sidebar tab lists saved names, ELF paths and capture times. Saving shows a progress screen; overwriting or deleting opens a centered confirmation that blocks the background. Completing or canceling these operations returns to the Baselines tab. Saving does not automatically select the new baseline. The default tabs display changed sizes as `170.00 KiB (+1.00 KiB)` and changed addresses as `0x20000400 (+0x400)`. Unchanged values keep their normal display; new entries are marked `(new)`. Percentages show differences in percentage points. Snapshots are saved as individual JSON files under `snout-firmware-explorer/snapshots/<build-folder-id>/<firmware-id>/<snapshot-id>.json` in the user configuration directory. Saved snapshots survive refreshes and restarts. Baseline selection lasts only for the current session; the app starts with no baseline selected. At startup, snapshot folders are removed when their build folder is no longer listed in the saved build settings, and empty snapshot directories are pruned. Deleting a snapshot also removes any empty firmware, build, and snapshot storage directories. Folder and file names use compact stable IDs so identical names from different builds remain separate and paths stay short on Windows. Display names are stored in the JSON files. Nothing is saved to the build folder. Selecting **None** in the Baselines tab returns to the usual display without deleting snapshots. Resetting build-folder settings preserves snapshots. Selecting a baseline automatically shows only added, removed, and changed entries in every tab. The header identifies the active baseline; clicking its badge clears comparison without changing tabs. The badge is hidden when no baseline is selected. Creation and deletion are available only from the Baselines tab. The table toolbar retains **Show address changes** while comparing. Overview applies the filter to its rankings; firmware totals remain the full current totals.

Source files use consistent short paths across the desktop views, keeping at least the parent directory and filename and adding parents to distinguish duplicates across the whole report. Hover shows the full absolute source path; relative compiler paths use a unique matching DWARF source or the build folder when available. ELF compilation-unit labels without a proven file location remain labels. Full paths remain searchable in file tables. The directory tree keeps its folder hierarchy and shows full source paths on hover. The desktop overview shows Flash payload and static RAM section distributions, with detailed section and attribution data in the Sections and Files tables. BSS and reservation categories use section naming conventions; no-load storage alone does not establish how startup initializes it. Stack summaries show the largest uniquely matched local frame and explain why total stack is unknown. Reports without ELF matches remain visible when no matches exist, including stripped firmware. A loaded comparison annotates changed values and filters the rankings and detailed tables to changed entries. F5 keeps the baseline snapshot while reloading current firmware and rereading selected maps. You can drop a build folder or an ELF into the window.

### Parsing and UI choices

- [goblin](https://docs.rs/goblin/0.10.7/goblin/elf/struct.Elf.html) exposes ELF section and program headers, including physical load addresses, directly. This avoids writing a production binary parser.
- [gimli and addr2line](https://docs.rs/addr2line/0.25.1/addr2line/) provide DWARF line lookup. `cpp_demangle` and `rustc-demangle` preserve raw symbol names alongside readable names.
- [egui/eframe](https://docs.rs/eframe/0.36.2/eframe/) provides a native Rust desktop frontend rendered with wgpu, with virtualized table rows, contextual help and no web frontend build step. Versions are pinned by `Cargo.lock`; deliberate dependency upgrades should run the full checks. Headless tests exercise egui without a GPU; release screenshots use wgpu with Mesa's Lavapipe Vulkan software driver (`mesa-vulkan-drivers`).
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

Hex viewer displays a continuously scrolling, 16-byte-per-line hexdump of the current firmware. Only visible lines are decoded and formatted; scrolling uses a bounded virtual window so large sparse address ranges stay responsive without losing individual address precision. Choose Flash load contents or RAM startup contents, optionally narrowed to a configured memory region. Byte, halfword, word and doubleword grouping and little/big endian controls default to the ELF bitness and byte order. ASCII remains in address order. Enter a hexadecimal address or exact symbol/section name to jump to its highlighted line; section and symbol starts appear at the right with byte offsets and hover details. Hover a name to outline its exact byte extent in both hex and ASCII, including across lines and within endian-formatted values. Hover an individual hex byte to outline that byte and its corresponding ASCII character and see its exact address, value and content provenance, plus every section and symbol covering it with their start addresses, sizes and byte offsets. Byte tooltips follow the selected endian order within grouped values and include zero-sized labels only at their exact address. Padding before or after the selected range has no byte tooltip or byte hover outline. Zero-sized labels have no inferred extent. RAM combines file-backed initial values (including code copied to RAM) with blue, inferred zeros for conventional `.bss`, `.sbss` and `.dynbss` sections. Startup code is not verified. Reservations, gaps, segment padding and unavailable bytes appear as `??`; erased Flash, runtime relocations, heap, stack and live RAM are not inferred. Without configured regions, bounds cover the ELF section ranges, not the full chip capacity. Hex viewer always displays current firmware and does not compare against a baseline. Raw memory bytes remain session-only and are excluded from JSON reports and snapshots.

ELF metadata does not conclusively identify physical memory technology. Writable, copied, and no-payload allocated sections are inferred as RAM under the bare-metal model. Memory regions imported from linker maps classify matching load/runtime ranges. A region must contain the entire range. Uncovered ranges produce warnings and retain inferred classification. In Memory regions, each configured region shows its address bounds, capacity, used/free bytes and percentage used. Each region has a green Flash or blue RAM occupancy bar in a scrollable list; search filters region names. Symbols are inspected in the Symbols tab. Without configured regions, this tab shows inferred load/runtime address ranges instead. If no map is selected, Memory regions shows a warning that capacity and free space are unknown, with an amber warning panel and an **Open Build files** link. Select **No map selected** in Build files to use ELF inference, or choose a map radio button to import its regions. **Load map file...** at the bottom of the map list in Build files allows selecting a map outside the scanned folder. **Autodetect map file**, beside it, reruns automatic matching for the selected ELF using the scanned maps and their current contents, replacing the manual map or inference choice on success. The selected map or inference mode is saved per firmware across restarts. Occupancy counts the union of allocated load and runtime section ranges intersecting each physical region, including padding and reservations, without counting the same addresses twice. Boundary-crossing sections count only their intersection; analysis notes still flag incomplete coverage. Free space means capacity minus static ELF occupancy, not guaranteed runtime headroom or a contiguous allocation. Without map-provided regions, capacity and free space remain unknown. Stripped files retain section usage but may have no symbols.

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
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
```

Run `python3 -m unittest discover -s scripts/tests -p 'test_*.py' -v`
for archive checks and publication regressions with a mock GitHub CLI. These
exercise create/upload/publish failures, empty/missing assets, retrying drafts,
protecting published releases, ZIP contents, package/sidecar digest mismatches,
missing materials and preserving an existing ZIP on failure without contacting GitHub. The reusable build workflow runs them before
platform builds. Validate workflow syntax and reusable calls with `actionlint`.

Run `pwsh -NoProfile -File scripts/tests/build-msix.Tests.ps1` for packaging-script
regressions without Rust builds or SDK tools. Coverage includes derived package
versions, invalid versions, compiler flags, relative output paths, build/cleanup
failures and caller-state restoration. Store-feature tests cover update rejection,
menu controls and independent workspace/snapshot cleanup. Graphics tests inspect
the backend mask without loading native graphics drivers. These checks do not
replace native Windows packaging, installation and upgrade tests.

Tests cover real GNU Arm GCC Cortex-M ELF files, DWARF function locations, local/global symbols, aliases, stripped files, initialized data, reservations, RAM code, unusual sections, comparisons, malformed input, 32/64-bit and little/big-endian synthetic ELF, CLI JSON/errors, compiler stack reports, and headless rendering of every data view. The checked-in fixtures allow tests without an ARM toolchain. Their C sources and linker script are in `fixtures/src`; regeneration instructions are in `fixtures/README.md`. A Docker compiler environment builds Cortex-M3 firmware with GCC, upstream Clang/lld, classic TI Arm CGT and TI Arm Clang. Generated artifacts live in `fixtures/build/<toolchain>`; only ELF, map and stack-report files are retained and committed. Tests cover custom copied RAM sections, stripped images and growth comparisons without requiring compilers. GCC retains its reference capacities, cross references and supported stack-report format. See `fixtures/README.md` for generation commands and compiler-specific limitations.

Windows tests and builds were run locally. CI is configured for Windows and Linux; Representative Linux desktop execution and native visual/interaction review still need validation. Headless egui tests do not validate native file dialogs or graphics drivers.

## Current boundaries and next milestones

This is the first implementation milestone, not a claim of universal firmware support.

1. **Attribution:** extend DWARF definition support and add GNU linker MAP object/archive ownership. Map import reads region capacities and GNU ld / LLVM lld symbol cross references for the dependency graph; section/object memory accounting is not yet imported. Variable attribution supports direct address expressions; location lists, TLS, complex expressions and definitions requiring reference resolution remain unsupported. Global variables without a matching supported definition remain unattributed. Compressed and split/external DWARF are not supported.
2. **Memory layouts:** add saved target profiles, explicit unknown memory roles, overlay policies and segment-only fallback. Relocatable objects, overlapping ordinary allocated/load ranges and sectionless ELFs currently return clear unsupported errors. HEX/BIN analysis and standalone MAP symbol/section analysis are not supported. Dynamic-symbol-only attribution is not yet implemented.
3. **Comparisons:** normalize source roots across build machines, improve duplicate/renamed symbol matching and introduce simple CI budgets. Matching currently uses file label, section and mangled symbol name, with duplicate identities aggregated. Differences in attribution or debug availability can affect per-file/symbol deltas.
4. **Stack:** import evidenced call graphs and represent recursion, indirect calls, assembly, interrupts and missing data before estimating call chains. The model reserves these uncertainty categories. CFA/disassembly, RTOS task stacks and runtime high-water marks are later inputs.
5. **Desktop delivery:** validate on representative user firmware and Linux desktops, refine large-report performance, add installers, accessibility/keyboard review and preferences. Source tables currently rebuild their display rows each frame, although visible rows are virtualized. Region occupancy and largest-contributor rankings are cached per analysis. Workspace preferences remember the folder, firmware, layout, view and overview memory selection. Refresh (F5) rescans and reloads the selected firmware while preserving the last successful report on failure. Direct-download preferences are stored in `snout-firmware-explorer/workspace.json` (the Store edition uses `snout-firmware-explorer-store/workspace.json`) under the user configuration directory (APPDATA on Windows, XDG_CONFIG_HOME or ~/.config elsewhere). Use **Menu > Open configuration folder** to open Snout’s settings and snapshot directory in your system file explorer. No export dialog is included yet; use CLI JSON for export.

No VS Code dependency is present. A future extension or CI service can consume core reports or the CLI without the desktop frontend.

### Thread-local storage

Generic TLS analysis uses standard ELF `PT_TLS`, `SHF_TLS`, and `STT_TLS` metadata. Reports show the initialized and zero-initialized template sizes, alignment, and TLS variables with template offsets. In the desktop, Binary information links to the TLS details in Sections, expanding the panel and scrolling its contents to the top even if it was previously collapsed. The collapsible TLS panel uses a bounded scroll area for variable lists and retains baseline size and offset annotations; variable offsets are within the per-thread template, not physical runtime addresses. When `PT_TLS` is absent, the template extent is inferred from allocated TLS sections, including internal gaps. Initialized section payload remains in Flash accounting; TLS templates are excluded from static RAM and physical runtime ranges. ELF template addresses may overlap ordinary runtime sections.

The template size is a per-thread requirement, not a complete allocation size. Total TLS RAM remains unknown: the ELF does not establish live thread count, ABI/runtime overhead, or whether TLS occupies already-counted stack reservations. No RTOS-specific assumptions are applied. TLS variables are listed separately; DWARF source attribution for TLS remains unsupported.
