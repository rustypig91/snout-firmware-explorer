"""Regenerate the committed CMake sensor-monitor build using GNU Arm GCC.

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
    "--gcc", default="arm-none-eabi-gcc", help="GNU Arm GCC executable (default: arm-none-eabi-gcc)"
)
args = parser.parse_args()
compiler = shutil.which(args.gcc)
if not compiler:
    raise SystemExit("Install GNU Arm GCC or pass --gcc /path/to/arm-none-eabi-gcc")
compiler = str(Path(compiler).resolve())

build = root / "build"
# Compiler changes need fresh CMake identification, while the committed object
# and stack-report directories must remain intact until the build replaces them.
cache = build / "CMakeCache.txt"
if cache.exists():
    for line in cache.read_text().splitlines():
        if line.startswith(("CMAKE_C_COMPILER:", "FIXTURE_C_COMPILER:")):
            previous = line.split("=", 1)[1]
            if Path(previous).resolve() != Path(compiler):
                cache.unlink()
                for directory in (build / "CMakeFiles").glob("[0-9]*"):
                    if directory.is_dir():
                        shutil.rmtree(directory)
            break

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
        f"-DCMAKE_C_COMPILER={compiler}",
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
