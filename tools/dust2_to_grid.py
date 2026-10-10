#!/usr/bin/env python3
"""Converts the de_dust2 geometry (an OBJ export of the CS 1.6 BSP, z up, GoldSrc units / 32) into Counterslop's
multi-level grid: games/dust/src/maps/dust.txt (layouts) and dust_heights.txt (heights), one block per layer.

    tools/dust2_to_grid.py path/to/Dust2.obj [--debug DIR]

Each 1 m cell is sampled 4 x 4 times with vertical rays. Upward faces with headroom over them are floors; a flood
fill from the bomb sites (step up a jump, drop any height) keeps the reachable ones, so roofs drop out. Per cell the
lowest reachable floor is the base layer, a second one stacked above it a slab in layer 1, and a ceiling over the top
floor a roof block. Cells without floor are walls (as tall as the faces in them) or crates (by texture). Bomb sites
are flood filled around the X decals, spawns placed on a checkerboard in each spawn yard.
"""

import argparse
import collections
import os
import sys

import numpy as np

SCALE = 0.8128  # meters per OBJ unit (32 GoldSrc units of an inch)
CELL = 1.0
SUB = 4  # samples per cell side
R = CELL / SUB
SHIFT = 4.0  # added to every height so the lowest floor sits near 0
HEADROOM = 1.85  # a floor needs this much air above it
JUMP = 1.15  # highest rise a player gets onto
ROOF_MAX = 4.5  # ceilings at most this high over the floor become roof blocks
ROOF = 2.0  # how thick the game makes a roof block
CRATE_TOPS = {"material_9", "material_17", "material_21"}
CRATE_SIDES = {"material_2", "material_12", "material_21", "material_31"}
# Brush entities that are only small light fittings.
IGNORED_MATERIALS = {"material_32", "material_33", "material_3", "material_18"}
# OBJ coordinates (meters) of the A and B site decals, and the spawn yards as (x0, y0, x1, y1).
SITE_X = {"A": (23.0, 49.2), "B": (-33.2, 53.8)}
SITE_RADIUS = {"A": 7.5, "B": 8.0}
SPAWN_AREA = {"T": (-44.0, -19.0, -24.0, -7.0), "C": (-17.5, 41.5, -6.5, 52.5)}
DIGITS = "0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ"


def load(path):
    verts, tris = [], []
    material = None
    for line in open(path):
        p = line.split()
        if not p:
            continue
        if p[0] == "v":
            verts.append(tuple(map(float, p[1:4])))
        elif p[0] == "usemtl":
            material = p[1]
        elif p[0] == "f" and material not in IGNORED_MATERIALS:
            ix = [int(x.split("/")[0]) - 1 for x in p[1:]]
            tris.extend((material, ix[0], ix[k], ix[k + 1]) for k in range(1, len(ix) - 1))
    return np.array(verts) * SCALE, tris


def rasterize(verts, tris, x0, y0, nx, ny):
    """Per sample: every non-vertical face crossing its vertical line as (z, normal z, material), and the highest
    point of any face over it (walls included)."""
    cols = [[[] for _ in range(nx)] for _ in range(ny)]
    top = np.full((ny, nx), -99.0)
    for material, ia, ib, ic in tris:
        a, b, c = verts[ia], verts[ib], verts[ic]
        n = np.cross(b - a, c - a)
        length = np.linalg.norm(n)
        if length < 1e-9:
            continue
        lo, hi = np.minimum(np.minimum(a, b), c), np.maximum(np.maximum(a, b), c)
        i0, i1 = max(int((lo[0] - x0) / R), 0), min(int((hi[0] - x0) / R), nx - 1)
        j0, j1 = max(int((lo[1] - y0) / R), 0), min(int((hi[1] - y0) / R), ny - 1)
        top[j0 : j1 + 1, i0 : i1 + 1] = np.maximum(top[j0 : j1 + 1, i0 : i1 + 1], hi[2])
        nz = n[2] / length
        if abs(nz) < 0.05:
            continue
        jj, ii = np.mgrid[j0 : j1 + 1, i0 : i1 + 1]
        px, py = x0 + (ii + 0.5) * R, y0 + (jj + 0.5) * R
        d = (b[1] - c[1]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[1] - c[1])
        l1 = ((b[1] - c[1]) * (px - c[0]) + (c[0] - b[0]) * (py - c[1])) / d
        l2 = ((c[1] - a[1]) * (px - c[0]) + (a[0] - c[0]) * (py - c[1])) / d
        l3 = 1 - l1 - l2
        inside = (l1 >= -1e-6) & (l2 >= -1e-6) & (l3 >= -1e-6)
        z = l1 * a[2] + l2 * b[2] + l3 * c[2]
        for i, j, zz in zip(ii[inside], jj[inside], z[inside]):
            cols[j][i].append((zz, nz, material))
    return cols, top


def floors(col):
    """(z, ceiling, material) of every upward face with air over it, lowest first."""
    hits = sorted(col)
    out = []
    for k, (z, nz, material) in enumerate(hits):
        if nz <= 0.7:
            continue
        above = [h for h in hits[k + 1 :] if h[0] > z + 0.05]
        if above and above[0][1] > 0:
            continue
        ceiling = above[0][0] if above else 99.0
        if ceiling - z >= HEADROOM:
            out.append((z, ceiling, material))
    return out


def reachable(fl, seeds):
    nx, ny = len(fl[0]), len(fl)
    seen = set(seeds)
    queue = collections.deque(seeds)
    while queue:
        i, j, k = queue.popleft()
        z, ceiling, _ = fl[j][i][k]
        for a, b in ((i + 1, j), (i - 1, j), (i, j + 1), (i, j - 1)):
            if not (0 <= a < nx and 0 <= b < ny):
                continue
            for kk, (z2, c2, _) in enumerate(fl[b][a]):
                if (a, b, kk) not in seen and z2 - z <= JUMP and min(ceiling, c2) - max(z, z2) >= HEADROOM:
                    seen.add((a, b, kk))
                    queue.append((a, b, kk))
    return seen


def quarter(h):
    return int(round((h + SHIFT) * 4))


def digit(q):
    return DIGITS[max(0, min(q, len(DIGITS) - 1))]


def convert(obj):
    verts, tris = load(obj)
    used = sorted({i for t in tris for i in t[1:]})
    lo, hi = verts[used].min(0), verts[used].max(0)
    x0, y0 = np.floor(lo[0]) - CELL, np.floor(lo[1]) - CELL
    nx, ny = int((hi[0] - x0) / R) + SUB, int((hi[1] - y0) / R) + SUB
    nx, ny = nx - nx % SUB, ny - ny % SUB
    cols, top = rasterize(verts, tris, x0, y0, nx, ny)
    fl = [[floors(cols[j][i]) for i in range(nx)] for j in range(ny)]
    seeds = []
    for x, y in SITE_X.values():
        i, j = int((x - x0) / R), int((y - y0) / R)
        seeds.append((i, j, 0))
    seen = reachable(fl, seeds)

    w, d = nx // SUB, ny // SUB
    # Cells are indexed [row][column] with row 0 the northernmost, as the text grid reads.
    base = [["#"] * w for _ in range(d)]
    base_h = [[None] * w for _ in range(d)]
    slab = [[None] * w for _ in range(d)]
    roof = [[None] * w for _ in range(d)]
    for cj in range(d):
        row = d - 1 - cj
        for ci in range(w):
            samples = [(ci * SUB + a, cj * SUB + b) for a in range(SUB) for b in range(SUB)]
            stacks = [[f for k, f in enumerate(fl[j][i]) if (i, j, k) in seen] for i, j in samples]
            ground = [s[0] for s in stacks if s]
            if len(ground) * 2 >= len(samples):
                on_crate = sum(f[2] in CRATE_TOPS for f in ground)
                z = float(np.median([f[0] for f in ground]))
                ceiling = float(np.median([f[1] for f in ground]))
                if on_crate * 2 > len(ground):
                    base[row][ci] = "k"
                else:
                    base[row][ci] = "."
                base_h[row][ci] = z
                upper = [s[1] for s in stacks if len(s) > 1]
                if len(upper) * 2 >= len(samples) and base[row][ci] == ".":
                    slab[row][ci] = float(np.median([f[0] for f in upper]))
                    ceiling = float(np.median([f[1] for f in upper]))
                # Ceilings well overhead (arches, beams) are left out.
                if ceiling - (slab[row][ci] if slab[row][ci] is not None else z) < ROOF_MAX and base[row][ci] == ".":
                    roof[row][ci] = ceiling
                continue
            tops = [max(cols[j][i], key=lambda h: h[0]) for i, j in samples if cols[j][i]]
            crate = [t for t in tops if t[2] in CRATE_TOPS and t[1] > 0.7]
            if len(crate) * 2 > len(samples):
                base[row][ci] = "k"
                base_h[row][ci] = float(np.median([t[0] for t in crate]))
            else:
                h = max(top[j][i] for i, j in samples)
                base_h[row][ci] = h if h > -50 else None
    # Walls with no faces of their own (outside the map) stand as tall as the walls next to them.
    for row in range(d):
        for ci in range(w):
            if base[row][ci] == "#" and base_h[row][ci] is None:
                near = [base_h[r][c] for r in range(max(row - 3, 0), min(row + 4, d)) for c in range(max(ci - 3, 0), min(ci + 4, w)) if base_h[r][c] is not None]
                base_h[row][ci] = max(near) if near else 4.0
    # Walls next to a roof block (2 m thick in the game) rise above it, so the roof reads as part of a building.
    for row in range(d):
        for ci in range(w):
            if base[row][ci] != "#":
                continue
            near = [roof[r][c] for r in range(max(row - 1, 0), min(row + 2, d)) for c in range(max(ci - 1, 0), min(ci + 2, w)) if roof[r][c] is not None]
            if near:
                base_h[row][ci] = max(base_h[row][ci], max(near) + ROOF + 1.0)
    mark_sites(base, base_h, x0, y0, d)
    mark_spawns(base, base_h, slab, roof, x0, y0, d)
    # Crop to the walkable area plus a wall border.
    rows = [r for r in range(d) if any(c != "#" for c in base[r])]
    cols_used = [c for c in range(w) if any(base[r][c] != "#" for r in range(d))]
    r0, r1, c0, c1 = rows[0] - 1, rows[-1] + 1, cols_used[0] - 1, cols_used[-1] + 1
    crop = lambda g: [line[c0 : c1 + 1] for line in g[r0 : r1 + 1]]
    base, base_h, slab, roof = crop(base), crop(base_h), crop(slab), crop(roof)
    layers = [(base, [[digit(quarter(h)) for h in line] for line in base_h])]
    # Slabs and roofs over the base share layer 1; roofs over slabs go to layer 2.
    l1 = [[" "] * len(base[0]) for _ in base]
    l1h = [[" "] * len(base[0]) for _ in base]
    l2 = [[" "] * len(base[0]) for _ in base]
    l2h = [[" "] * len(base[0]) for _ in base]
    for r, line in enumerate(base):
        for c in range(len(line)):
            if slab[r][c] is not None:
                l1[r][c], l1h[r][c] = ".", digit(quarter(slab[r][c]))
                if roof[r][c] is not None:
                    l2[r][c], l2h[r][c] = "#", digit(quarter(roof[r][c]))
            elif roof[r][c] is not None:
                l1[r][c], l1h[r][c] = "#", digit(quarter(roof[r][c]))
    layers.append((l1, l1h))
    if any(ch != " " for line in l2 for ch in line):
        layers.append((l2, l2h))
    origin = (x0 + c0 * CELL, y0 + (d - 1 - r1) * CELL)
    return layers, origin


def cell_of(x, y, x0, y0, d):
    return d - 1 - int((y - y0) / CELL), int((x - x0) / CELL)


def mark_sites(base, base_h, x0, y0, d):
    for site, (x, y) in SITE_X.items():
        start = cell_of(x, y, x0, y0, d)
        queue, seen = collections.deque([start]), {start}
        while queue:
            r, c = queue.popleft()
            if base[r][c] == ".":
                base[r][c] = site
            for a, b in ((r + 1, c), (r - 1, c), (r, c + 1), (r, c - 1)):
                if (a, b) in seen or base[a][b] not in ".k":
                    continue
                if np.hypot(b - start[1], a - start[0]) * CELL > SITE_RADIUS[site] or abs(base_h[a][b] - base_h[r][c]) > 0.6:
                    continue
                seen.add((a, b))
                queue.append((a, b))


def mark_spawns(base, base_h, slab, roof, x0, y0, d):
    for team, (ax, ay, bx, by) in SPAWN_AREA.items():
        r1, c0 = cell_of(ax, ay, x0, y0, d)
        r0, c1 = cell_of(bx, by, x0, y0, d)
        spots = []
        for r in range(r0, r1 + 1):
            for c in range(c0 + (r + c0) % 2, c1 + 1, 2):
                around = [(r, c), (r + 1, c), (r - 1, c), (r, c + 1), (r, c - 1)]
                flat = all(base[a][b] == "." and abs(base_h[a][b] - base_h[r][c]) < 0.4 and slab[a][b] is None for a, b in around)
                if flat and roof[r][c] is None:
                    spots.append((r, c))
        mid = np.mean(spots, axis=0)
        spots.sort(key=lambda p: np.hypot(p[0] - mid[0], p[1] - mid[1]))
        if len(spots) < 16:
            sys.exit(f"only {len(spots)} spawn spots for {team}")
        for r, c in spots[:16]:
            base[r][c] = team


def debug_images(layers, out):
    from PIL import Image, ImageDraw

    scale = 6
    for n, (chars, heights) in enumerate(layers):
        h, w = len(chars), len(chars[0])
        img = np.zeros((h, w, 3), np.uint8)
        for r in range(h):
            for c in range(w):
                ch, hd = chars[r][c], heights[r][c]
                if ch == " ":
                    continue
                v = DIGITS.index(hd) / 4.0
                shade = int(np.clip(40 + v * 22, 0, 255))
                color = {
                    ".": (shade, shade, shade),
                    "#": (shade // 3, shade // 4, shade // 2) if n == 0 else (60, 60, 200),
                    "k": (150, 90, 30),
                    "A": (220, shade // 2, shade // 2),
                    "B": (shade // 2, shade // 2, 220),
                    "T": (230, 200, 0),
                    "C": (0, 200, 230),
                }.get(ch, (255, 0, 255))
                img[r, c] = color
        im = Image.fromarray(img).resize((w * scale, h * scale), Image.NEAREST)
        draw = ImageDraw.Draw(im)
        for k in range(0, max(w, h), 10):
            draw.line([(k * scale, 0), (k * scale, 4)], fill=(255, 255, 0))
            draw.text((k * scale + 2, 0), str(k), fill=(255, 255, 0))
            draw.line([(0, k * scale), (4, k * scale)], fill=(255, 255, 0))
            draw.text((2, k * scale + 2), str(k), fill=(255, 255, 0))
        im.save(os.path.join(out, f"layer{n}.png"))


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("obj")
    ap.add_argument("--out", default=os.path.join(os.path.dirname(__file__), "..", "games", "dust", "src", "maps"))
    ap.add_argument("--debug", help="write a top-down image of each layer here")
    args = ap.parse_args()
    layers, origin = convert(args.obj)
    with open(os.path.join(args.out, "dust.txt"), "w") as f:
        f.write("\n\n".join("\n".join("".join(line) for line in chars) for chars, _ in layers) + "\n")
    with open(os.path.join(args.out, "dust_heights.txt"), "w") as f:
        f.write("\n\n".join("\n".join("".join(line) for line in hs) for _, hs in layers) + "\n")
    if args.debug:
        debug_images(layers, args.debug)
    print(f"{len(layers[0][0][0])} x {len(layers[0][0])} cells, {len(layers)} layers, grid corner at OBJ ({origin[0]:.1f}, {origin[1]:.1f})")


if __name__ == "__main__":
    main()
