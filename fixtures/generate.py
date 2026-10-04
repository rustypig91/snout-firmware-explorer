"""Regenerate committed Cortex-M fixtures using Clang + LLD or GNU Arm GCC.

Run from the repository root: python fixtures/generate.py [--gcc arm-none-eabi-gcc]
No external Python packages required. Tests use committed binaries.
"""
import argparse
from pathlib import Path
import shutil
import subprocess

root = Path(__file__).resolve().parent
parser = argparse.ArgumentParser()
parser.add_argument('--gcc', help='GNU Arm GCC executable; otherwise use clang + ld.lld')
args = parser.parse_args()
compiler = args.gcc or shutil.which('clang')
if not compiler:
    raise SystemExit('Install clang + ld.lld or pass --gcc arm-none-eabi-gcc')
compiler = str(Path(shutil.which(compiler) or compiler).resolve())
build = root / 'build'
build.mkdir(exist_ok=True)
flags = ['-mcpu=cortex-m3', '-mthumb', '-ffreestanding', '-fno-builtin', '-fno-common',
         '-fno-unwind-tables', '-fno-asynchronous-unwind-tables', '-O0', '-g', '-gdwarf-4', '-fstack-usage']
if not args.gcc:
    flags += ['--target=arm-none-eabi']
for variant, extra in [('cortex-m', 0), ('cortex-m-grown', 8)]:
    objects = []
    for source in ['main', 'diag']:
        obj = build / f'{variant}-{source}.o'
        subprocess.run([compiler, *flags, f'-DEXTRA={extra}', '-c', f'fixtures/src/{source}.c', '-o', str(obj)], check=True)
        objects.append(str(obj))
    if args.gcc:
        command = [compiler, '-mcpu=cortex-m3', '-mthumb', '-nostdlib', '-Wl,--build-id=none', '-Wl,--cref,--no-demangle', '-Wl,-T,fixtures/src/cortex-m.ld', *objects]
    else:
        linker = shutil.which('ld.lld') or str(Path(compiler).with_name('ld.lld.exe'))
        command = [linker, '-T', 'fixtures/src/cortex-m.ld', '--build-id=none', *objects]
    names = [variant, 'cortex-m-stripped'] if variant == 'cortex-m' else [variant]
    for name in names:
        mapfile = root / f'{name}.map'
        map_flag = f'-Wl,-Map,{mapfile}' if args.gcc else f'-Map={mapfile}'
        strip_flags = (['-Wl,--strip-all' if args.gcc else '--strip-all']
                       if name == 'cortex-m-stripped' else [])
        subprocess.run([*command, map_flag, *strip_flags, '-o', str(root / f'{name}.elf')], check=True)
    if variant == 'cortex-m':
        for su in build.glob(f'{variant}-*.su'):
            shutil.copyfile(su, root / su.name)
print('Generated ELF, linker map, and compiler stack-usage fixtures.')
