#!/usr/bin/env python3
"""
Fast DM Icon Generator v2 — Modern Design
Panah download + petir dalam rounded square
"""

import struct
import zlib
import os
import math


def create_png(width, height, pixels):
    def chunk(ctype, data):
        c = ctype + data
        return struct.pack('>I', len(data)) + c + struct.pack('>I', zlib.crc32(c) & 0xFFFFFFFF)

    header = b'\x89PNG\r\n\x1a\n'
    ihdr = chunk(b'IHDR', struct.pack('>IIBBBBB', width, height, 8, 6, 0, 0, 0))

    raw = b''
    idx = 0
    for y in range(height):
        raw += b'\x00'
        for x in range(width):
            r, g, b, a = pixels[idx]
            raw += struct.pack('BBBB', r, g, b, a)
            idx += 1

    idat = chunk(b'IDAT', zlib.compress(raw, 9))
    iend = chunk(b'IEND', b'')
    return header + ihdr + idat + iend


def clamp(v, lo=0, hi=255):
    return max(lo, min(hi, int(v)))


def dist(x1, y1, x2, y2):
    return math.sqrt((x2 - x1)**2 + (y2 - y1)**2)


def lerp(a, b, t):
    return a + (b - a) * max(0, min(1, t))


def in_rounded_rect(px, py, rx, ry, rw, rh, rad):
    if px < rx or px > rx + rw or py < ry or py > ry + rh:
        return 0.0
    for cx, cy in [(rx+rad, ry+rad), (rx+rw-rad, ry+rad),
                   (rx+rad, ry+rh-rad), (rx+rw-rad, ry+rh-rad)]:
        in_cx = (px < rx+rad and cx == rx+rad) or (px > rx+rw-rad and cx == rx+rw-rad)
        in_cy = (py < ry+rad and cy == ry+rad) or (py > ry+rh-rad and cy == ry+rh-rad)
        if in_cx and in_cy:
            d = dist(px, py, cx, cy)
            if d > rad:
                return 0.0
            if d > rad - 1.5:
                return max(0, min(1, rad - d))
    return 1.0


def in_circle(px, py, cx, cy, r):
    d = dist(px, py, cx, cy)
    if d > r:
        return 0.0
    if d > r - 1.5:
        return max(0, min(1, r - d))
    return 1.0


def in_poly(px, py, poly):
    n = len(poly)
    inside = False
    j = n - 1
    for i in range(n):
        xi, yi = poly[i]
        xj, yj = poly[j]
        if ((yi > py) != (yj > py)) and (px < (xj-xi)*(py-yi)/(yj-yi)+xi):
            inside = not inside
        j = i
    return inside


def generate_icon(size):
    pixels = []
    cx, cy = size / 2, size / 2
    pad = size * 0.04
    rad = size * 0.24

    # Colors — Catppuccin Mocha
    c_bg1 = (24, 24, 37)        # #181825 (dark)
    c_bg2 = (30, 30, 46)        # #1e1e2e
    c_grad_top = (137, 180, 250) # #89b4fa (blue)
    c_grad_bot = (116, 199, 236) # #74c7ec (sapphire)
    c_arrow = (17, 17, 27)       # #11111b (dark arrow)
    c_bolt = (249, 226, 175)     # #f9e2af (yellow)
    c_glow = (137, 180, 250)     # #89b4fa

    for y in range(size):
        for x in range(size):
            r, g, b, a = 0, 0, 0, 0

            # ── Shadow ──
            sa = in_rounded_rect(
                x, y,
                pad + size*0.02, pad + size*0.025,
                size - pad*2, size - pad*2,
                rad
            )
            if sa > 0:
                r, g, b = 0, 0, 0
                a = clamp(50 * sa)

            # ── Main background with gradient ──
            ma = in_rounded_rect(
                x, y, pad, pad,
                size - pad*2, size - pad*2,
                rad
            )

            if ma > 0:
                # Diagonal gradient
                t = ((x - pad) / (size - pad*2) * 0.4 +
                     (y - pad) / (size - pad*2) * 0.6)
                t = max(0, min(1, t))

                gr = lerp(c_grad_top[0], c_grad_bot[0], t)
                gg = lerp(c_grad_top[1], c_grad_bot[1], t)
                gb = lerp(c_grad_top[2], c_grad_bot[2], t)

                # Radial highlight (top-left area)
                d_hl = dist(x, y, cx * 0.6, cy * 0.5) / (size * 0.6)
                if d_hl < 1.0:
                    hl = (1.0 - d_hl) * 0.2
                    gr = min(255, gr + hl * 60)
                    gg = min(255, gg + hl * 60)
                    gb = min(255, gb + hl * 60)

                r, g, b = clamp(gr), clamp(gg), clamp(gb)
                a = clamp(255 * ma)

                # ── Arrow shaft (rounded rectangle) ──
                shaft_w = size * 0.14
                shaft_h = size * 0.26
                shaft_top = cy - size * 0.15
                shaft_left = cx - shaft_w / 2
                shaft_rad = shaft_w * 0.3

                in_shaft = in_rounded_rect(
                    x, y,
                    shaft_left, shaft_top,
                    shaft_w, shaft_h,
                    shaft_rad
                )

                # ── Arrow head (triangle) ──
                head_top = shaft_top + shaft_h - size * 0.02
                head_bot = head_top + size * 0.18
                head_w = size * 0.34
                head_left = cx - head_w / 2

                triangle = [
                    (cx, head_bot),
                    (head_left, head_top),
                    (head_left + head_w, head_top),
                ]
                in_head = in_poly(x, y, triangle)

                # ── Tray / base line ──
                tray_top = head_bot + size * 0.06
                tray_h = size * 0.035
                tray_w = size * 0.44
                tray_left = cx - tray_w / 2
                tray_rad = tray_h * 0.5

                in_tray = in_rounded_rect(
                    x, y,
                    tray_left, tray_top,
                    tray_w, tray_h,
                    tray_rad
                )

                # ── Side walls ──
                wall_w = size * 0.035
                wall_h = size * 0.09
                wall_top = tray_top - wall_h

                in_lwall = in_rounded_rect(
                    x, y,
                    tray_left, wall_top,
                    wall_w, wall_h + tray_h,
                    wall_w * 0.3
                )
                in_rwall = in_rounded_rect(
                    x, y,
                    tray_left + tray_w - wall_w, wall_top,
                    wall_w, wall_h + tray_h,
                    wall_w * 0.3
                )

                # Draw arrow (dark color on gradient)
                if in_shaft > 0 or in_head or in_tray > 0 or in_lwall > 0 or in_rwall > 0:
                    arrow_alpha = max(in_shaft, 1.0 if in_head else 0.0,
                                     in_tray, in_lwall, in_rwall)
                    r = clamp(lerp(r, c_arrow[0], arrow_alpha))
                    g = clamp(lerp(g, c_arrow[1], arrow_alpha))
                    b = clamp(lerp(b, c_arrow[2], arrow_alpha))

                    # ── Lightning bolt on arrow ──
                    if in_shaft > 0 or in_head:
                        bs = size * 0.07
                        bx, by = cx, cy - size * 0.01

                        bolt = [
                            (bx + bs*0.25,  by - bs*1.5),
                            (bx - bs*0.4,   by + bs*0.05),
                            (bx + bs*0.06,  by + bs*0.05),
                            (bx - bs*0.25,  by + bs*1.5),
                            (bx + bs*0.4,   by - bs*0.05),
                            (bx - bs*0.06,  by - bs*0.05),
                        ]

                        if in_poly(x, y, bolt):
                            r, g, b = c_bolt
                            a = clamp(255 * ma)

            # ── Outer glow ──
            if ma == 0:
                glow_r = size * 0.025
                for tr in range(1, int(glow_r) + 1):
                    for dx, dy in [(-1,0),(1,0),(0,-1),(0,1),(-1,-1),(1,1)]:
                        tx, ty = x + dx*tr, y + dy*tr
                        ta = in_rounded_rect(
                            tx, ty, pad, pad,
                            size-pad*2, size-pad*2, rad
                        )
                        if ta > 0:
                            intensity = 1.0 - tr / glow_r
                            r, g, b = c_glow
                            a = clamp(25 * intensity)
                            break

            pixels.append((clamp(r), clamp(g), clamp(b), clamp(a)))

    return create_png(size, size, pixels)


def main():
    script_dir = os.path.dirname(os.path.abspath(__file__))
    icons_dir = os.path.join(script_dir, "extension", "icons")
    os.makedirs(icons_dir, exist_ok=True)

    for sz in [16, 32, 48, 128]:
        fname = "icon{}.png".format(sz)
        print("Generating {} ({}x{})...".format(fname, sz, sz))
        data = generate_icon(sz)
        path = os.path.join(icons_dir, fname)
        with open(path, 'wb') as f:
            f.write(data)
        print("  -> {} ({} bytes)".format(path, len(data)))

    # App icon
    import shutil
    src = os.path.join(icons_dir, "icon128.png")
    dst = os.path.join(script_dir, "fast-dm-icon.png")
    if os.path.exists(src):
        shutil.copy2(src, dst)
        print("  -> {} (app icon)".format(dst))

    print("\nDone!")


if __name__ == "__main__":
    main()
