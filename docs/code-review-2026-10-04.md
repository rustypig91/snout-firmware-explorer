# Code review — 4 October 2026

## Purpose and design

Snout is a local desktop application and CLI for understanding linked embedded
firmware, initially ARM Cortex-M with GCC. Its shared Rust core reads ELF sections
and load segments, attributes symbol bytes using DWARF and ELF compilation-unit
labels, and produces serializable reports. The desktop frontend adds build-folder
discovery, saved artifact selections, memory-region views, linker-map dependency
graphs, compiler stack reports, and build comparisons.

The distinction between measured bytes and inferred behavior is central to the
design. Flash totals count allocated file payload; RAM totals count static runtime
storage. Aliases share unique attributed bytes. Compiler `.su` frames and GNU ld
cross references do not establish a runtime call graph or total stack requirement.
Missing information is generally represented through warnings and unknown values.

The core/frontend separation supports reuse and testing. The checked-in real and
synthetic fixtures cover accounting independently of an installed ARM toolchain.
The main remaining risks are incomplete debug/ownership support, matching artifacts
from different builds, UI work on large inputs, and installation-mode handling.

## Bugs fixed in this review

### P2: RAM load overlays bypassed validation

In `crates/firmware-analysis-core/src/elf.rs`, load-overlap validation considered
only sections contributing Flash bytes. Configuring the same load storage as RAM
therefore turned an unsupported overlapping image into a successful analysis.

Validation now checks every nonempty allocated load payload regardless of inferred
physical memory type. The regression modifies the second load segment in the real
Cortex-M fixture to overlap the first, while keeping runtime sections disjoint.
It verifies rejection for both Flash and RAM layouts.

### P2: Applying a layout retained stack matches from an older ELF

Both JSON configuration and manual map application reread the firmware from disk,
but the resulting `Loaded::Config` kept the previous stack report. After a rebuild,
the desktop could show stack candidates for functions absent from the new ELF and
old frame sizes beside current memory usage.

Configuration jobs now reload stack reports against the new analysis using the
saved selection. Firmware and stack results are applied together. A failed report
read clears the obsolete report, adds an analysis warning, and retains the user's
saved selection for retry. The regression covers JSON and map application, a
replacement stripped ELF, changed frame bytes, and an unreadable stack report.

### P2: Comparison reports omitted input analysis warnings

The standalone core comparison report retained comparison-specific warnings but
discarded the older and current analyses' diagnostics. CLI text printed those
separately, while exported comparison JSON did not. The desktop also lacked the
baseline's detailed analysis warnings in its comparison notes.

Comparisons now include input diagnostics labeled `Older build` and `Current
build`. CLI text uses that complete warning list. The regression verifies that
both inputs' distinct diagnostics survive comparison JSON serialization.

Each new regression was run against the previous behavior and failed before its
corresponding fix.

## Filed follow-ups

| Priority | Issue | Finding and next step |
| --- | --- | --- |
| P1 | [#6 — MSI updates](https://github.com/rustypig91/snout-firmware-explorer/issues/6) | Windows MSI installs fall through to portable self-replacement. Detect MSI ownership and use an MSI upgrade or direct users to the installer. Requires native Windows validation. |
| P2 | [#7 — PR validation](https://github.com/rustypig91/snout-firmware-explorer/issues/7) | Ordinary unlabeled PRs skip formatting, Clippy, and tests. Run validation independently of release packaging labels. |
| P2 | [#8 — Comparison source roots](https://github.com/rustypig91/snout-firmware-explorer/issues/8) | Different build-machine source roots appear as added/removed files and symbols. Add explicit source-root mappings with preserved original evidence. |
| P2 | [#9 — Compressed DWARF](https://github.com/rustypig91/snout-firmware-explorer/issues/9) | Compressed debug sections disable source attribution. Add bounded decompression with equivalent-fixture and corruption coverage. |
| P2 | [#10 — Large-report performance](https://github.com/rustypig91/snout-firmware-explorer/issues/10) | Tables rebuild/filter/sort full row sets per frame; dependency geometry computes on the UI thread. Benchmark representative reports, cache row indices, and compute layouts in workers. |
| P2 | [#11 — CLI map import](https://github.com/rustypig91/snout-firmware-explorer/issues/11) | The CLI cannot import map-backed dependency edges available in the GUI/core. Add explicit map input and define capacity/configuration precedence. |

Every issue includes code evidence, proposed behavior, and acceptance criteria.
Source-root normalization and compressed DWARF are existing documented boundaries;
the issues turn those roadmap items into actionable work.

## Validation and scope

- `cargo test --workspace --locked`: **170 passed**, zero failures.
- `cargo clippy --workspace --all-targets --locked -- -D warnings`: passed.
- `cargo fmt --all -- --check`: passed.
- `git diff --check`: passed.

Review covered the core parsers/accounting/report models, CLI behavior, desktop
loading and configuration flow, table and graph preparation, preferences, updater
mode selection, and build/release workflow.

Desktop interaction coverage is headless egui testing. This review did not exercise
native file dialogs, graphics drivers, Windows installers, live update
installation, release packaging, or representative large customer firmware. The
performance issue is based on the code's repeated work; no benchmarked latency is
claimed. The MSI finding is based on installation and updater code, with native
validation explicitly included in its acceptance criteria.
