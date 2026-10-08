"""Generate the application PNG and multi-size Windows ICO from snout.svg.

Install CairoSVG (python3 -m pip install CairoSVG), then run this script from
any directory. Generated assets are committed; app builds need no SVG renderer.
"""
from pathlib import Path
import struct

import cairosvg

ROOT = Path(__file__).resolve().parents[1] / "crates/firmware-gui/packaging/icons"


def png(size):
    return cairosvg.svg2png(
        url=str(ROOT / "snout.svg"), output_width=size, output_height=size
    )


if __name__ == "__main__":
    sizes = [16, 24, 32, 48, 64, 128, 256]
    images = [png(size) for size in sizes]
    offset = 6 + 16 * len(sizes)
    directory = bytearray(struct.pack("<HHH", 0, 1, len(sizes)))
    for size, data in zip(sizes, images):
        directory.extend(
            struct.pack("<BBBBHHII", size % 256, size % 256, 0, 0, 1, 32, len(data), offset)
        )
        offset += len(data)
    (ROOT / "snout.ico").write_bytes(directory + b"".join(images))
    (ROOT / "snout.png").write_bytes(png(512))
