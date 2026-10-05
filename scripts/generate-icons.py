"""Rasterize the rect/ellipse-only Snout SVG to PNG and a multi-size Windows ICO.

No third-party dependencies. Run from any directory after editing snout.svg.
The small renderer deliberately rejects unsupported SVG shapes.
"""
from pathlib import Path
import struct
import xml.etree.ElementTree as ET
import zlib

ROOT = Path(__file__).resolve().parents[1] / "crates/firmware-gui/packaging/icons"


def png(size):
    svg = ET.parse(ROOT / "snout.svg").getroot()
    shapes = []
    for element in svg:
        kind = element.tag.rsplit("}", 1)[-1]
        if kind not in ("rect", "ellipse"):
            raise ValueError(f"Unsupported SVG shape: {kind}")
        color = bytes.fromhex(element.attrib["fill"].lstrip("#"))
        shapes.append((kind, {k: float(v) for k, v in element.attrib.items() if k != "fill"}, color))

    def pixel(x, y):
        result = (0, 0, 0, 0)
        for kind, a, color in shapes:
            if kind == "ellipse":
                inside = ((x-a["cx"])/a["rx"])**2 + ((y-a["cy"])/a["ry"])**2 <= 1
            else:
                left, top = a.get("x", 0), a.get("y", 0)
                right, bottom = left+a["width"], top+a["height"]
                radius = a.get("rx", 0)
                dx = max(left+radius-x, 0, x-(right-radius))
                dy = max(top+radius-y, 0, y-(bottom-radius))
                inside = left <= x <= right and top <= y <= bottom and dx*dx+dy*dy <= radius*radius
            if inside:
                result = (*color, 255)
        return result

    rows = bytearray()
    for y in range(size):
        rows.append(0)
        for x in range(size):
            samples = [pixel((x+(sx+0.5)/4)*256/size, (y+(sy+0.5)/4)*256/size)
                       for sy in range(4) for sx in range(4)]
            alpha = sum(p[3] for p in samples)
            rows.extend(round(sum(p[c]*p[3] for p in samples)/alpha) if alpha else 0 for c in range(3))
            rows.append(round(alpha/16))

    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind+data))
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(rows, 9)) + chunk(b"IEND", b""))


if __name__ == "__main__":
    sizes = [16, 24, 32, 48, 64, 128, 256]
    images = [png(size) for size in sizes]
    offset = 6 + 16*len(sizes)
    directory = bytearray(struct.pack("<HHH", 0, 1, len(sizes)))
    for size, data in zip(sizes, images):
        directory.extend(struct.pack("<BBBBHHII", size % 256, size % 256, 0, 0, 1, 32, len(data), offset))
        offset += len(data)
    (ROOT / "snout.ico").write_bytes(directory + b"".join(images))
    (ROOT / "snout.png").write_bytes(images[-1])
