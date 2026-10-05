# Large-report UI performance (#10)

Measured on Windows x86-64 on 5 October 2026, using Rust 1.98.1 and the
unoptimized test profile. Both revisions ran on the same machine, sequentially.
The baseline was commit `88a61ee` with only the benchmark module added.

Run the deterministic, headless egui benchmark with:

```text
cargo test -p firmware-gui performance:: --locked -- --ignored --nocapture --test-threads=1
```

The report contains 50,000 uniquely named symbols with varied numeric sizes.
The dependency graph contains 48 nodes and 182 edges. Unchanged-frame figures
use 30 samples after the initial frame/layout settles. Search and sort figures
measure the frame that applies the changed control. These measure CPU-side
egui work, not GPU presentation or end-to-end input latency on a physical display.

| Operation | Before (ms) | After (ms) |
| --- | ---: | ---: |
| Symbols: cold frame | 225.07 | 184.99 |
| Symbols: unchanged frame, median | 218.02 | 5.24 |
| Symbols: unchanged frame, p95 | 230.32 | 5.53 |
| Symbols: search | 171.66 | 32.42 |
| Symbols: sort after search | 169.48 | 12.87 |
| Graph: initial frame | 592.15 | 10.12 |
| Graph: unchanged frame, median | 1.92 | 1.16 |
| Graph: unchanged frame, p95 | 2.01 | 1.22 |
| Graph: search | 19.81 | 0.55 |

Graph initial/search frames now submit background work and display a preparing
message. Their timings measure UI responsiveness, not completion of the layout.
Geometry still requires computation. A single worker per graph view coalesces
pending requests; revisions prevent an old result from replacing the current
selection. Filtering and other controls remain available while it runs.

Table rows and lowercase search cells are retained for the active report/view
selection. Filtered/sorted indices and numeric bar maxima are retained separately;
sort-only changes reuse filtering. Normal rows use constant-height virtualization.
The cache intentionally trades memory for repeated-frame latency: initial row and
detail-string preparation still runs once on the UI thread when the report or
view selection changes. Only the active table is retained. Search/sort do not
reformat those strings. Expanded details preserve the existing variable-height
layout and selection behavior.

Regression coverage checks cache reuse/invalidation, stable numeric sorting,
filtered bar scaling, obsolete graph results, and existing headless pointer,
keyboard, node/edge, zoom and memory-mode interactions.

# Windows icon

`scripts/generate-icons.py` derives the PNG and seven-size ICO from the existing
Snout SVG using only Python's standard library. The build embeds the ICO as a
Windows executable resource; the PNG also supplies the native window icon.
Inno Setup uses the same icon for setup and explicit executable icon references
for its shortcuts. WiX uses the executable resource for advertised shortcuts and
Add/Remove Programs.

Native Windows resource inspection confirmed the compiled executable contains
16, 24, 32, 48, 64, 128 and 256 pixel icon entries. Inno Setup 6.7.3 successfully
compiled the setup installer using the local debug executable. This was a
packaging smoke check; it did not install over the user's existing application
or test the installed Start menu visually. WiX packaging was not run locally.

Workspace tests plus the final regression runs passed (189 tests, with the
three manual benchmarks ignored by ordinary test runs). All benchmarks were also
run explicitly. Formatting, Clippy with warnings denied, and `git diff --check`
passed. Rebuilding and distributing a release installer is required to update
existing installations.


# Startup analysis follow-up

Stack report matching previously scanned every ELF function for every `.su`
entry, including normalizing/comparing all known source paths. It now builds
name, source-line and parameterless-name indices once per analysis and checks
source compatibility only among candidates. Automatic report discovery reuses
that index across files. Candidate ordering, ambiguous matches, conflicting
absolute paths, and source-line/overload fallback rules are preserved.

ELF symbol collection now looks up sections directly by ELF index. Byte
attribution uses ranges in the already section-sorted symbol table instead of
scanning every symbol for every section.

Reproduce the stack-stage benchmark:

```text
cargo test -p firmware-analysis-core --test stack large_stack_analysis_latency --locked -- --ignored --nocapture
```

On the same Windows machine/debug profile used above, matching 2,000 uniquely
named functions to 2,000 stack entries took **14,831.99 ms before** and
**39.31 ms after**, including index construction and report-file parsing.
Synthetic input generation and initial fixture ELF decoding are outside this
timer. This is a stack-stage measurement, not total application startup latency.
The user's large project has not been profiled; ELF/DWARF decoding, map import
and filesystem discovery can still contribute to startup time.
