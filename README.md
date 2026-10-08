# Rusty's Snout — Firmware Explorer

Understand where your embedded firmware uses memory and what changes between builds.

Snout is a standalone desktop app and command-line tool. Open a build folder or ELF file to explore Flash and static RAM usage, find the largest contributors, and compare saved builds. Firmware analysis stays on your computer.

## What you can do

- See Flash and RAM usage, with capacity and remaining space when a supported linker map supplies memory regions.
- Drill into source files, functions, variables and ELF sections to find what takes up memory.
- Save snapshots and compare builds to investigate size changes.
- Explore linker dependencies, compiler stack reports and firmware bytes in the hex viewer.

The main target is ARM Cortex-M firmware built with GCC. Other ELF files may work, with limits depending on their metadata. Static RAM usage does not predict all runtime heap and stack requirements.

## Get started

Download a Windows or Linux build from [Releases](https://github.com/rustypig91/snout-firmware-explorer/releases).

1. Open Snout and choose **Open build folder**, or drag in a folder or ELF file.
2. Select your firmware in the sidebar.
3. Start with **Overview**, then explore **Files** or **Symbols** to find the largest contributors.

Matching linker maps and stack reports are discovered automatically where possible. Press **F5** after rebuilding to refresh the analysis. Use **Baselines** in the left sidebar to save builds for comparison, and select a baseline or **None** within that tab. The header shows the active baseline.

## Build from source

With a stable Rust toolchain and the [platform build dependencies](docs/technical-reference.md#opening-firmware-and-selecting-supporting-files) installed:

```sh
cargo run -p snout -- fixtures/build/cortex-m.elf
```

The CLI can also analyze firmware or compare two builds:

```sh
cargo run -p snout-cli -- analyze fixtures/build/cortex-m.elf
cargo run -p snout-cli -- diff fixtures/build/cortex-m.elf fixtures/build/cortex-m-grown.elf
```

See the [technical reference](docs/technical-reference.md) for detailed behavior, supported inputs, memory accounting, CLI options and development checks. Coding agents should start with [AGENTS.md](AGENTS.md).

## License

[Apache 2.0](LICENSE).
