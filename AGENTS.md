# Repository guide for coding agents

## Start here

- `README.md` is the short user introduction. Keep it focused on the application's purpose and getting started.
- `docs/technical-reference.md` documents current behavior, accounting rules, persistence, supported formats, limitations, packaging and validation. Read the relevant sections before changing those areas.
- Source code and tests are authoritative if documentation differs. Update the reference when behavior changes; update the README only when the user introduction or quick start needs to change.
- `fixtures/README.md` explains how to regenerate the committed firmware fixtures.

## Code map

- `crates/firmware-analysis-core`: ELF/DWARF and map parsing, memory accounting, attribution, comparisons, dependencies and stack reports. Keep accounting in the core.
- `crates/firmware-gui`: egui/eframe desktop UI, workspace preferences, snapshots and updates. Analysis runs on a worker thread; failed loads preserve the last successful report.
- `crates/firmware-cli`: command-line interface and JSON output.
- `fixtures`: committed firmware and compiler reports for reproducible tests.
- `scripts` and `.github/workflows/build.yml`: packaging and release automation.

## Preserve these semantics

- Distinguish Flash payload, static RAM, physical region occupancy and runtime memory requirements. Do not infer unknown capacity or runtime heap/stack demand.
- ELF/DWARF attribution takes precedence over map information. Automatic map selection requires an unambiguous match.
- Linker cross references are symbol dependencies, not proven runtime calls. Compiler stack reports give local frames, not call-chain totals.
- Preserve exact integer sizes and addresses, explicit uncertainty, and saved workspace/snapshot compatibility.

## Validation

Run checks appropriate to the change. For Rust changes, check formatting and relevant tests. For broader core or cross-crate changes, use the workspace checks:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

GUI tests include headless rendering and interactions; they do not verify native dialogs or graphics drivers. Do not require an ARM toolchain for routine tests; use committed fixtures.
