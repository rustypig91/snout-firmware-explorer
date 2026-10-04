"""Regenerate the sensor-monitor fixtures using GNU Arm GCC or Clang + LLD.

Run from the repository root:
    python fixtures/generate.py --gcc arm-none-eabi-gcc

Tests use committed artifacts and require no ARM toolchain. GNU ld maps also
provide memory capacities and the dependency graph's symbol cross references.
"""

import argparse
from pathlib import Path
import shutil
import subprocess

root = Path(__file__).resolve().parent
parser = argparse.ArgumentParser()
parser.add_argument(
    "--gcc", help="GNU Arm GCC executable; otherwise use clang + ld.lld"
)
args = parser.parse_args()
compiler = args.gcc or shutil.which("clang")
if not compiler:
    raise SystemExit("Install clang + ld.lld or pass --gcc arm-none-eabi-gcc")
compiler = str(Path(shutil.which(compiler) or compiler).resolve())

build = root / "build"
build.mkdir(exist_ok=True)
flags = [
    "-mcpu=cortex-m3",
    "-mthumb",
    "-ffreestanding",
    "-fno-builtin",
    "-fno-common",
    "-fno-unwind-tables",
    "-fno-asynchronous-unwind-tables",
    "-O0",
    "-g",
    "-gdwarf-4",
    "-fstack-usage",
    "-Wall",
    "-Wextra",
    "-Werror",
]
if not args.gcc:
    flags += ["--target=arm-none-eabi"]

sources = ["main", "config", "sensor", "diag", "telemetry", "transport"]
for variant, extra in [("cortex-m", 0), ("cortex-m-grown", 8)]:
    objects = []
    for source in sources:
        obj = build / f"{variant}-{source}.o"
        subprocess.run(
            [
                compiler,
                *flags,
                f"-DEXTRA={extra}",
                "-c",
                f"fixtures/src/{source}.c",
                "-o",
                str(obj),
            ],
            check=True,
        )
        objects.append(str(obj))

    if args.gcc:
        command = [
            compiler,
            "-mcpu=cortex-m3",
            "-mthumb",
            "-nostdlib",
            "-Wl,--build-id=none",
            "-Wl,--cref,--no-demangle",
            "-Wl,-T,fixtures/src/cortex-m.ld",
            *objects,
        ]
    else:
        linker = shutil.which("ld.lld") or str(
            Path(compiler).with_name("ld.lld.exe")
        )
        command = [
            linker,
            "-T",
            "fixtures/src/cortex-m.ld",
            "--build-id=none",
            *objects,
        ]

    # The stripped ELF comes from the same baseline objects, preserving layout.
    names = [variant, "cortex-m-stripped"] if variant == "cortex-m" else [variant]
    for name in names:
        mapfile = root / f"{name}.map"
        map_flag = f"-Wl,-Map,{mapfile}" if args.gcc else f"-Map={mapfile}"
        strip_flags = []
        if name == "cortex-m-stripped":
            strip_flags = ["-Wl,--strip-all" if args.gcc else "--strip-all"]

        elf = root / f"{name}.elf"
        subprocess.run(
            [*command, map_flag, *strip_flags, "-o", str(elf)], check=True
        )
        # Fixture ELFs are data inputs, not host executables.
        elf.chmod(0o644)
        # GNU ld adds trailing spaces to fill rows; keep committed maps clean.
        lines = (line.rstrip() for line in mapfile.read_text().splitlines())
        mapfile.write_text("\n".join(lines) + "\n")

    # Keep one report set in the example folder; recursive scans would otherwise
    # load baseline and grown intermediate reports as additional copies.
    for source in sources:
        su = build / f"{variant}-{source}.su"
        if variant == "cortex-m":
            shutil.copyfile(su, root / su.name)
        su.unlink()

print("Generated sensor-monitor ELF, linker-map, and stack-usage fixtures.")
