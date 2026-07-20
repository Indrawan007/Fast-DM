#!/usr/bin/env python3
"""
Fast DM Icon Generator
Membuat icon PNG murni dengan Python (tanpa Pillow/ImageMagick)

Icon: Panah download dengan petir di tengah lingkaran bergradasi
"""

import struct
import zlib
import os
import math


def create_png(width, height, pixels):
    """
    Buat file PNG dari array pixel RGBA.
    pixels = list of (r, g, b, a) per pixel, row by row
    """
    def chunk(chunk_type, data):
        c = chunk_type + data
        crc = struct.pack('>I', zlib.crc32(c) & 0xFFFFFFFF)
        return struct.pack('>I', len(data)) + c + crc

    # Header
    header = b'\x89PNG\r\n\x1a\n'

    # IHDR: width, height, bit_depth=8, color_type=6 (RGBA)
    ihdr_data = struct.pack('>IIBBBBB', width, height, 8, 6, 0, 0, 0)
    ihdr = chunk(b'IHDR', ihdr_data)

    # IDAT: pixel data
    raw = b''
    idx = 0
    for y in range(height):
        raw += b'\x00'  # filter: None
        for x in range(width):
            r, g, b, a = pixels[idx]
            raw += struct.pack('BBBB', r, g, b, a)
            idx += 1

    idat = chunk(b'IDAT', zlib.compress(raw, 9))

    # IEND
    iend = chunk(b'IEND', b'')

    return header + ihdr + idat + iend


def lerp(a, b, t):
    """Linear interpolation."""
    return a + (b - a) * t


def lerp_color(c1, c2, t):
    """Interpolate dua warna."""
    return tuple(int(lerp(c1[i], c2[i], t)) for i in range(len(c1)))


def distance(x1, y1, x2, y2):
    """Jarak euclidean."""
    return math.sqrt((x2 - x1) ** 2 + (y2 - y1) ** 2)


def clamp(val, lo, hi):
    return max(lo, min(hi, val))


def point_in_polygon(px, py, polygon):
    """Ray casting algorithm."""
    n = len(polygon)
    inside = False
    j = n - 1
    for i in range(n):
        xi, yi = polygon[i]
        xj, yj = polygon[j]
        if ((yi > py) != (yj > py)) and \
           (px < (xj - xi) * (py - yi) / (yj - yi) + xi):
            inside = not inside
        j = i
    return inside


def point_in_rounded_rect(px, py, rx, ry, rw, rh, radius):
    """Cek apakah titik di dalam rounded rectangle."""
    # Luar bounding box
    if px < rx or px > rx + rw or py < ry or py > ry + rh:
        return False, 0.0

    # Cek corners
    corners = [
        (rx + radius, ry + radius),             # top-left
        (rx + rw - radius, ry + radius),        # top-right
        (rx + radius, ry + rh - radius),        # bottom-left
        (rx + rw - radius, ry + rh - radius),   # bottom-right
    ]

    for cx, cy in corners:
        # Tentukan apakah titik di area corner
        in_corner_x = (px < rx + radius and cx == corners[0][0]) or \
                      (px > rx + rw - radius and cx == corners[1][0])
        in_corner_y = (py < ry + radius and cy == corners[0][1]) or \
                      (py > ry + rh - radius and cy == corners[2][1])

        if in_corner_x and in_corner_y:
            d = distance(px, py, cx, cy)
            if d > radius:
                return False, 0.0
            elif d > radius - 1.5:
                # Anti-aliasing di edge
                return True, clamp(radius - d, 0, 1)

    return True, 1.0


def generate_icon(size):
    """
    Generate icon Fast DM.

    Design:
    - Rounded square background (gradient biru)
    - Panah download putih di tengah
    - Petir kuning kecil di dalam panah
    - Subtle shadow dan glow
    """
    pixels = []
    cx, cy = size / 2, size / 2
    margin = size * 0.06
    corner_radius = size * 0.22

    # ── Warna tema ──
    bg_top    = (69, 71, 90)       # #45475a (gelap)
    bg_bot    = (30, 30, 46)       # #1e1e2e (lebih gelap)
    grad_top  = (116, 199, 236)    # #74c7ec (sapphire)
    grad_bot  = (137, 180, 250)    # #89b4fa (blue)
    arrow_col = (205, 214, 244)    # #cdd6f4 (text)
    bolt_col  = (249, 226, 175)    # #f9e2af (yellow)
    shadow    = (17, 17, 27)       # #11111b
    glow_col  = (137, 180, 250)    # #89b4fa

    for y in range(size):
        for x in range(size):
            # ── Layer 0: Transparent background ──
            r, g, b, a = 0, 0, 0, 0

            # Normalized coordinates
            nx = x / size
            ny = y / size

            # ── Layer 1: Drop shadow ──
            shadow_offset = size * 0.02
            in_shadow, sa = point_in_rounded_rect(
                x, y,
                margin + shadow_offset,
                margin + shadow_offset,
                size - margin * 2,
                size - margin * 2,
                corner_radius
            )
            if in_shadow and sa > 0:
                r, g, b = shadow
                a = int(80 * sa)

            # ── Layer 2: Main rounded rect with gradient ──
            in_rect, rect_alpha = point_in_rounded_rect(
                x, y,
                margin, margin,
                size - margin * 2,
                size - margin * 2,
                corner_radius
            )

            if in_rect and rect_alpha > 0:
                # Gradient dari atas ke bawah
                t = clamp((y - margin) / (size - margin * 2), 0, 1)
                gr, gg, gb = lerp_color(grad_top, grad_bot, t)

                # Subtle radial highlight di tengah-atas
                dist_center = distance(x, y, cx, cy * 0.7) / (size * 0.5)
                if dist_center < 1.0:
                    highlight = (1.0 - dist_center) * 0.15
                    gr = min(255, int(gr + highlight * 80))
                    gg = min(255, int(gg + highlight * 80))
                    gb = min(255, int(gb + highlight * 80))

                r, g, b = gr, gg, gb
                a = int(255 * rect_alpha)

            # ── Layer 3: Download arrow ──
            if in_rect and rect_alpha > 0:
                # Arrow body (vertikal rectangle)
                arrow_w = size * 0.16
                arrow_h = size * 0.30
                arrow_top = cy - size * 0.18
                arrow_left = cx - arrow_w / 2

                # Rounded top for arrow shaft
                in_shaft = False
                if (arrow_left <= x <= arrow_left + arrow_w and
                        arrow_top <= y <= arrow_top + arrow_h):
                    in_shaft = True

                # Arrow head (triangle pointing down)
                head_top = arrow_top + arrow_h - size * 0.02
                head_bot = head_top + size * 0.22
                head_w = size * 0.38
                head_left = cx - head_w / 2

                triangle = [
                    (cx, head_bot),           # bottom point
                    (head_left, head_top),    # top-left
                    (head_left + head_w, head_top),  # top-right
                ]

                in_head = point_in_polygon(x, y, triangle)

                # Bottom line (tray/platform)
                tray_top = head_bot + size * 0.06
                tray_h = size * 0.04
                tray_w = size * 0.50
                tray_left = cx - tray_w / 2
                tray_radius = tray_h / 2

                in_tray = False
                if (tray_left <= x <= tray_left + tray_w and
                        tray_top <= y <= tray_top + tray_h):
                    in_tray = True

                # Side bars of tray
                sidebar_w = size * 0.04
                sidebar_h = size * 0.12
                sidebar_top = tray_top - sidebar_h

                in_left_bar = (tray_left <= x <= tray_left + sidebar_w and
                               sidebar_top <= y <= tray_top + tray_h)
                in_right_bar = (tray_left + tray_w - sidebar_w <= x <= tray_left + tray_w and
                                sidebar_top <= y <= tray_top + tray_h)

                if in_shaft or in_head or in_tray or in_left_bar or in_right_bar:
                    # Anti-aliasing: soften edges
                    r, g, b = arrow_col
                    a = int(255 * rect_alpha)

                    # ── Layer 4: Lightning bolt overlay on arrow ──
                    if in_shaft or in_head:
                        bolt_cx = cx
                        bolt_cy = cy - size * 0.02
                        bolt_s = size * 0.09  # scale

                        # Bolt shape (simple zigzag)
                        bolt_points = [
                            (bolt_cx + bolt_s * 0.3, bolt_cy - bolt_s * 1.8),
                            (bolt_cx - bolt_s * 0.5, bolt_cy + bolt_s * 0.1),
                            (bolt_cx + bolt_s * 0.1, bolt_cy + bolt_s * 0.1),
                            (bolt_cx - bolt_s * 0.3, bolt_cy + bolt_s * 1.8),
                            (bolt_cx + bolt_s * 0.5, bolt_cy - bolt_s * 0.1),
                            (bolt_cx - bolt_s * 0.1, bolt_cy - bolt_s * 0.1),
                        ]

                        if point_in_polygon(x, y, bolt_points):
                            r, g, b = bolt_col
                            a = int(255 * rect_alpha)

            # ── Layer 5: Subtle outer glow ──
            if not in_rect:
                glow_dist = 0
                for test_r in range(1, int(size * 0.03) + 1):
                    for dx, dy in [(-1, 0), (1, 0), (0, -1), (0, 1)]:
                        tx, ty = x + dx * test_r, y + dy * test_r
                        in_t, _ = point_in_rounded_rect(
                            tx, ty, margin, margin,
                            size - margin * 2, size - margin * 2,
                            corner_radius
                        )
                        if in_t:
                            glow_dist = max(glow_dist, 1.0 - test_r / (size * 0.03))

                if glow_dist > 0:
                    r, g, b = glow_col
                    a = int(30 * glow_dist)

            pixels.append((
                clamp(r, 0, 255),
                clamp(g, 0, 255),
                clamp(b, 0, 255),
                clamp(a, 0, 255)
            ))

    return create_png(size, size, pixels)


def generate_all_icons(output_dir):
    """Generate semua ukuran icon."""
    os.makedirs(output_dir, exist_ok=True)

    sizes = {
        "icon16.png":  16,
        "icon32.png":  32,
        "icon48.png":  48,
        "icon128.png": 128,
    }

    for filename, size in sizes.items():
        print("Generating {} ({}x{})...".format(filename, size, size))
        png_data = generate_icon(size)
        filepath = os.path.join(output_dir, filename)
        with open(filepath, 'wb') as f:
            f.write(png_data)
        print("  -> {} ({} bytes)".format(filepath, len(png_data)))

    # Copy icon128 sebagai app icon juga
    import shutil
    app_icon = os.path.join(output_dir, "..", "fast-dm-icon.png")
    shutil.copy2(
        os.path.join(output_dir, "icon128.png"),
        app_icon
    )
    print("  -> {} (app icon)".format(app_icon))

    print("\nDone! All icons generated.")


if __name__ == "__main__":
    script_dir = os.path.dirname(os.path.abspath(__file__))
    icons_dir = os.path.join(script_dir, "extension", "icons")
    generate_all_icons(icons_dir)
