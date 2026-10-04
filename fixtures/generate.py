"""Regenerate the committed CMake sensor-monitor build using GNU Arm GCC or Clang.

Run from the repository root:
    python fixtures/generate.py --gcc arm-none-eabi-gcc

Tests use committed artifacts and require no ARM toolchain. CMake, Ninja, and
an ARM-capable compiler are required only when regenerating the fixture build.
"""

import argparse
from pathlib import Path
import shutil
import subprocess

root = Path(__file__).resolve().parent
parser = argparse.ArgumentParser()
parser.add_argument(
    "--gcc", help="GNU Arm GCC executable; otherwise use clang with ARM target support"
)
args = parser.parse_args()
compiler = args.gcc or shutil.which("clang")
if not compiler:
    raise SystemExit("Install clang + ld.lld or pass --gcc arm-none-eabi-gcc")
compiler = str(Path(shutil.which(compiler) or compiler).resolve())

build = root / "build"
subprocess.run(
    [
        "cmake",
        "-S",
        str(root),
        "-B",
        str(build),
        "-G",
        "Ninja",
        f"-DCMAKE_TOOLCHAIN_FILE={root / 'cmake' / 'arm-none-eabi.cmake'}",
        f"-DFIXTURE_C_COMPILER={compiler}",
    ],
    check=True,
)
subprocess.run(["cmake", "--build", str(build)], check=True)

# Preserve clean text/file modes in the committed artifact snapshot.
for name in ["cortex-m", "cortex-m-grown", "cortex-m-stripped"]:
    (build / f"{name}.elf").chmod(0o644)
    mapfile = build / f"{name}.map"
    lines = (line.rstrip() for line in mapfile.read_text().splitlines())
    mapfile.write_text("\n".join(lines) + "\n")

print("Generated CMake sensor-monitor build in fixtures/build/.")
