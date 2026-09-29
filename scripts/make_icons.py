#!/usr/bin/env python3
"""Render awake's app and tray icons without third-party packages.

Open eye = keeping awake, closed eye = off. macOS gets black+alpha template
images (the menu bar tints them); Windows/Linux get colored ones.
Run `mise run icons` to regenerate everything, including the bundle icons.
"""
import math
import struct
import zlib
from pathlib import Path

OUT = Path(__file__).resolve().parent.parent / "apps/awake-tray/src-tauri/icons"
SS = 4  # supersampling per axis

# Eye geometry in normalized coords (-1..1, y down): a vesica of two circles.
A, B = 0.86, 0.50  # half width, half height
R = (A * A + B * B) / (2 * B)


def sd_circle(x, y, cx, cy, r):
    return math.hypot(x - cx, y - cy) - r


def sd_eye(x, y):
    # Circle centred above passes through the bottom lid, and vice versa.
    return max(sd_circle(x, y, 0, -(R - B), R), sd_circle(x, y, 0, R - B, R))


def sd_segment(x, y, ax, ay, bx, by):
    px, py, dx, dy = x - ax, y - ay, bx - ax, by - ay
    h = max(0.0, min(1.0, (px * dx + py * dy) / (dx * dx + dy * dy)))
    return math.hypot(px - dx * h, py - dy * h)


def sd_round_rect(x, y, half, radius):
    qx, qy = abs(x) - half + radius, abs(y) - half + radius
    return math.hypot(max(qx, 0), max(qy, 0)) + min(max(qx, qy), 0) - radius


def open_eye(x, y, stroke):
    outline = abs(sd_eye(x, y)) - stroke / 2
    pupil = sd_circle(x, y, 0, 0, 0.27)
    return min(outline, pupil)


def closed_eye(x, y, stroke):
    # Lower lid arc (bottom of the circle centred above) plus three lashes.
    cy = -(R - B)
    arc = abs(sd_circle(x, y, 0, cy, R)) - stroke / 2
    if y < 0.05 or abs(x) > A:
        arc = max(arc, 1.0)
    lashes = 1.0
    for lx in (-0.5, 0.0, 0.5):
        ly = cy + math.sqrt(R * R - lx * lx)  # point on the arc
        nx, ny = lx / R, (ly - cy) / R  # outward normal
        lashes = min(lashes, sd_segment(x, y, lx + nx * 0.08, ly + ny * 0.08,
                                        lx + nx * 0.30, ly + ny * 0.30) - stroke / 2)
    return min(arc, lashes)


def render(size, shade):
    """shade(x, y) -> (r, g, b, a) in 0..1 for normalized coords."""
    rows = []
    for py in range(size):
        row = bytearray([0])  # filter byte
        for px in range(size):
            acc = [0.0, 0.0, 0.0, 0.0]
            for sy in range(SS):
                for sx in range(SS):
                    x = ((px + (sx + 0.5) / SS) / size) * 2 - 1
                    y = ((py + (sy + 0.5) / SS) / size) * 2 - 1
                    r, g, b, a = shade(x, y)
                    acc[0] += r * a
                    acc[1] += g * a
                    acc[2] += b * a
                    acc[3] += a
            n = SS * SS
            alpha = acc[3] / n
            if alpha > 0:
                rgb = [c / acc[3] for c in acc[:3]]
            else:
                rgb = [0, 0, 0]
            row += bytes(round(max(0, min(1, v)) * 255) for v in (*rgb, alpha))
        rows.append(bytes(row))
    return png(size, size, b"".join(rows))


def png(w, h, raw):
    def chunk(tag, data):
        c = struct.pack(">I", len(data)) + tag + data
        return c + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)

    return (b"\x89PNG\r\n\x1a\n"
            + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 6, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(raw, 9))
            + chunk(b"IEND", b""))


def hexrgb(h):
    return tuple(int(h[i:i + 2], 16) / 255 for i in (1, 3, 5))


def glyph_shader(shape, color, scale=0.92, stroke=0.15, outline=None):
    def shade(x, y):
        x, y = x / scale, y / scale
        d = shape(x, y, stroke)
        if d <= 0:
            return (*color, 1.0)
        if outline and d <= 0.09:
            return (*outline, 1.0)
        return (0, 0, 0, 0)
    return shade


def app_icon_shader():
    top, bottom = hexrgb("#4B5BD6"), hexrgb("#1B2466")
    white, amber, dark = (1, 1, 1), hexrgb("#FFB300"), hexrgb("#1B2466")

    def shade(x, y):
        if sd_round_rect(x, y, 0.80, 0.22) > 0:
            return (0, 0, 0, 0)
        t = (y + 1) / 2
        bg = tuple(top[i] * (1 - t) + bottom[i] * t for i in range(3))
        ex, ey = x / 0.62, y / 0.62
        if abs(sd_eye(ex, ey)) <= 0.075:
            return (*white, 1)
        if sd_eye(ex, ey) < 0:
            if sd_circle(ex, ey, -0.1, -0.1, 0.09) < 0:
                return (*white, 1)
            if sd_circle(ex, ey, 0, 0, 0.30) < 0:
                return (*amber, 1)
            if sd_circle(ex, ey, 0, 0, 0.38) < 0:
                return (*dark, 1)
            return (0.93, 0.93, 0.93, 1)
        return (*bg, 1)
    return shade


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    black = (0, 0, 0)
    amber, grey, dark = hexrgb("#F5A623"), hexrgb("#9AA0A6"), hexrgb("#202124")
    files = {
        "tray-on-template.png": render(44, glyph_shader(open_eye, black)),
        "tray-off-template.png": render(44, glyph_shader(closed_eye, black)),
        "tray-on.png": render(64, glyph_shader(open_eye, amber, 0.84, 0.17, dark)),
        "tray-off.png": render(64, glyph_shader(closed_eye, grey, 0.84, 0.17, dark)),
        "app-icon.png": render(1024, app_icon_shader()),
    }
    for name, data in files.items():
        (OUT / name).write_bytes(data)
        print(f"wrote {OUT / name}")


if __name__ == "__main__":
    main()
