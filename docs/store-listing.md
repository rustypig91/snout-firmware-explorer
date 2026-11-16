# Microsoft Store listing draft

Product: **Rusty's Snout - Firmware Explorer**

Short description: Understand where embedded firmware uses memory and what changes between builds.

Description:

Open a firmware build folder or ELF file to explore Flash and static RAM usage,
find the largest contributors, and compare saved builds. Firmware analysis stays
on your computer. Snout is a standalone desktop application, with no account or
ARM toolchain needed to inspect existing builds.

- Explore source files, functions, variables, ELF sections and firmware bytes.
- Compare saved baselines to investigate size and address changes.
- View physical memory regions when a matching linker map provides capacities.
- Inspect linker symbol dependencies and compiler-reported local stack frames.

The main target is ARM Cortex-M firmware built with GCC. Other ELF files may work
depending on their metadata. Static RAM totals do not predict all runtime heap
and stack requirements. Linker dependencies do not prove runtime calls, and local
stack frames are not call-chain totals. HEX/BIN input is not supported.

The Store edition includes the desktop GUI. The separate CLI remains available
in the project's direct-download packages. Updates to this edition are managed
by Microsoft Store.

Support: https://github.com/rustypig91/snout-firmware-explorer/issues

Privacy policy (publish before submission):
https://rustypig91.github.io/snout-firmware-explorer/privacy/

## Certification notes draft

Snout is an existing Rust/egui Win32 desktop application. The `runFullTrust`
capability allows its normal desktop execution, native file dialogs and access
to user-selected firmware build folders. It installs no drivers, services,
elevated helpers or separate updater. The Store edition cannot check, download
or install GitHub updates. No account or connected hardware is needed.

Reviewer steps:

1. Launch from the installed Start menu entry; check the title, icon and version.
2. Obtain the committed sample firmware from the project's `fixtures/build/gcc`
   directory, keeping the ELF, map and stack reports together.
3. Choose **Open build folder**, select `cortex-m.elf`, and inspect Overview,
   Symbols, Sections, Dependencies and Stack. Drag/drop the ELF as another test.
4. Save a baseline, close and reopen the app, and confirm it remains listed.
5. Use **Open configuration folder** and **Privacy policy** from the menu.
6. Confirm **Open in Microsoft Store** opens product `9NWM3QW158VZ`, and the
   GitHub update controls are absent.

Upload current screenshots and the generated listing PNGs from
`crates/snout/packaging/windows/store-listing/`. The repository's screenshot
workflow produces an Overview capture with the committed fixture; verify that
uploaded screenshots match the submitted version. Complete pricing/availability,
properties, age ratings and all remaining Partner Center sections before submission.

Tagged releases attach `store-submission.zip`, which groups the MSIX, checksum,
build metadata and these listing materials. Every successful Store workflow
also uploads it in `snout-store-submission-<run-id>`. Extract the ZIP and upload
the contained MSIX to Partner Center. Submission instructions are in its README;
Windows Store screenshots still need to be prepared separately.
