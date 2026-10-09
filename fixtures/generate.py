"""Build Cortex-M analysis fixtures with GCC, LLVM, or TI's Arm compilers.

Outputs live in fixtures/build/<toolchain>; temporary CMake build trees are
removed after exporting only ELF, map and compiler stack-report files.
"""

import argparse
from pathlib import Path
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parent
IMAGES = ("cortex-m", "cortex-m-grown", "cortex-m-stripped")
TOOLCHAINS = {
    "gcc": ("gcc", "arm-none-eabi.cmake"),
    "llvm": ("clang", "llvm-arm.cmake"),
    "ti-cgt": ("armcl", "ti-arm.cmake"),
    "ti-clang": ("tiarmclang", "ti-clang-arm.cmake"),
}


def build_fixture(name, compiler, build, custom_sections):
    subprocess.run(
        [
            "cmake", "-S", str(ROOT), "-B", str(build), "-G", "Ninja",
            f"-DCMAKE_TOOLCHAIN_FILE={ROOT / 'cmake' / TOOLCHAINS[name][1]}",
            f"-DFIXTURE_C_COMPILER={compiler}", f"-DCMAKE_C_COMPILER={compiler}",
            f"-DFIXTURE_CUSTOM_SECTIONS={'ON' if custom_sections else 'OFF'}",
        ],
        check=True,
    )
    subprocess.run(["cmake", "--build", str(build)], check=True)

    for image in IMAGES:
        (build / f"{image}.elf").chmod(0o644)
        mapfile = build / f"{image}.map"
        mapfile.write_text("\n".join(line.rstrip() for line in mapfile.read_text().splitlines()) + "\n")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--toolchain", choices=(*TOOLCHAINS, "all"), default="gcc")
    parser.add_argument("--gcc", default="arm-none-eabi-gcc", help="GNU Arm GCC executable")
    parser.add_argument("--clang", default="clang", help="LLVM Clang executable (requires lld)")
    parser.add_argument("--armcl", default="armcl", help="Classic TI Arm CGT executable")
    parser.add_argument("--tiarmclang", default="tiarmclang", help="TI Arm Clang executable")
    parser.add_argument("--output-dir", type=Path, default=ROOT / "build", help="Artifact root (default: fixtures/build); one subfolder per compiler")
    parser.add_argument("--custom-sections", action="store_true", help="Include the custom RAM code section in a GCC-only build")
    args = parser.parse_args()
    names = list(TOOLCHAINS) if args.toolchain == "all" else [args.toolchain]

    # Resolve every requested compiler before modifying any artifacts. `all`
    # fails explicitly if a compiler is missing rather than silently skipping it.
    compilers = {}
    for name in names:
        executable = getattr(args, TOOLCHAINS[name][0])
        compiler = shutil.which(executable)
        if not compiler:
            parser.error(f"Compiler for {name} not found: {executable}. Install it or pass --{TOOLCHAINS[name][0]} /path/to/compiler")
        compilers[name] = str(Path(compiler).resolve())

    with tempfile.TemporaryDirectory(prefix="snout-fixtures-") as temporary:
        for name in names:
            build = Path(temporary) / name
            compiler = compilers[name]
            # Keep the established GCC accounting reference unchanged. Other
            # compilers exercise the additional long custom RAM section.
            custom_sections = name != "gcc" or args.custom_sections
            build_fixture(name, compiler, build, custom_sections)
            destination = args.output_dir.resolve() / name
            # These compiler subfolders are generated artifacts, owned by this
            # script. Replace them only after a successful compiler build.
            if destination.exists():
                shutil.rmtree(destination)
            destination.mkdir(parents=True)
            artifacts = [build / f"{image}.{suffix}" for image in IMAGES for suffix in ("elf", "map")]
            artifacts.extend(sorted(build.rglob("*.su")))
            for source in artifacts:
                output = destination / source.relative_to(build)
                output.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(source, output)
                output.chmod(0o644)
            print(f"Generated {name} Cortex-M artifacts in {destination}.")


if __name__ == "__main__":
    main()
