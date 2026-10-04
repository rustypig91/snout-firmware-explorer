# Cortex-M test firmware

These tiny files are generated from project-owned C sources for analysis tests. They are not board-bootable firmware and contain no vendor code.

The committed ELF files were generated with xPack GNU Arm GCC 14.2.1-1.1 for Cortex-M3, Thumb, `-O0 -g -gdwarf-4 -fstack-usage`. Three translation units exercise source attribution and local compilation-unit labels. The linker script explicitly places read-only code in Flash, a read-only function in RAM with its load image in Flash, initialized data, BSS, and a 128-byte RAM reservation.

```sh
python fixtures/generate.py --gcc arm-none-eabi-gcc
```

Alternatively use Clang with ARM target support and `ld.lld` on PATH:

```sh
python fixtures/generate.py
```

Run from the repository root. The script regenerates three ELF files, a matching `.map` for each ELF, and compiler `.su` reports. The committed GNU linker maps let the build-folder example automatically import the 256 KiB Flash and 64 KiB RAM capacities. The GNU maps include raw-symbol cross references for the Dependencies view (`--cref --no-demangle`). Opening `cortex-m.elf` or `cortex-m-grown.elf` from this build folder shows three source units and five directed connections. The stripped variant retains the object dependency graph with unknown source ownership and sizes. Clang/LLD also emits maps, but its map format does not support automatic capacity import. Toolchains may produce different code sizes; the committed baseline and CLI test totals refer to the stated GCC version. Debug paths reflect the generation machine and tests deliberately compare suffixes. No downloaded compiler is committed (`fixtures/build/` is ignored).

GNU `arm-none-eabi-size -A` / `arm-none-eabi-readelf -l -S` reference for the committed baseline:

| Section | Bytes | Flash | RAM |
|---|---:|---:|---:|
| `.text` | 288 | 288 | 0 |
| `.rodata` | 48 | 48 | 0 |
| `.unusual_constants` | 8 | 8 | 0 |
| `.ram_code` | 28 | 28 | 28 |
| `.data` | 8 | 8 | 8 |
| `.bss` | 68 | 0 | 68 |
| `.reserved` | 128 | 0 | 128 |
| **Total** | | **380** | **232** |

`cortex-m-grown.elf` adds eight 32-bit sample slots: Flash stays 380 bytes, RAM grows by 32 to 264 bytes. `cortex-m-stripped.elf` has identical load/runtime usage and no debug information or symbol table. The weak callback and its alias share an address and must not double-count storage.

The fixture's source calls produce these dependency arrows (the main unit's entry point is `Reset_Handler`):

- `main.c → diag.c` via `diagnose`
- `main.c → telemetry.c` via `telemetry_collect`
- `diag.c → telemetry.c` via `telemetry_scale`
- `telemetry.c → diag.c` via `diagnose`
- `telemetry.c → main.c` via `ram_function`

The reciprocal unit dependencies do not create recursive function calls: `telemetry_collect → diagnose → telemetry_scale → ram_function` ends at the helper in main. Rescan or press F5 after rebuilding fixtures to reload the maps.
