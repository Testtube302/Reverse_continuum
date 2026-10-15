#!/usr/bin/env python3
"""Converts Wine's bitmap fonts into AIContinuum's small .bfnt format.

Usage: extract_wine_fonts.py WINE_FONT_DIR OUT_DIR
Reads sserife.fon (MS Sans Serif, 8 pt) and the 11 ppem embedded bitmap strikes of
tahoma.ttf / tahomabd.ttf (Tahoma 8 pt at 96 dpi). Needs fontTools.

.bfnt: b"BFNT", u8 ascent, u8 height, u16 count, then per glyph:
u16 code, i8 advance, i8 x offset, i8 y offset (top of bitmap below the cell top),
u8 width, u8 height, rows of ceil(width/8) bytes, MSB first.
"""
import struct, sys, os
from fontTools.ttLib import TTFont

def write(path, ascent, height, glyphs):
    out = bytearray(b"BFNT") + struct.pack("<BBH", ascent, height, len(glyphs))
    for code, adv, x, y, w, h, rows in glyphs:
        out += struct.pack("<HbbbBB", code, adv, x, y, w, h)
        for row in rows:
            out += row
    open(path, "wb").write(out)

def pack(bits, w):
    row = bytearray((w + 7) // 8)
    for i, b in enumerate(bits):
        if b:
            row[i // 8] |= 0x80 >> (i % 8)
    return bytes(row)

def fon(path, points):
    d = open(path, "rb").read()
    ne = struct.unpack_from("<H", d, 0x3C)[0]
    rt = ne + struct.unpack_from("<H", d, ne + 0x24)[0]
    shift = struct.unpack_from("<H", d, rt)[0]
    p = rt + 2
    fonts = []
    while True:
        tid = struct.unpack_from("<H", d, p)[0]
        if tid == 0:
            break
        count = struct.unpack_from("<H", d, p + 2)[0]
        p += 8
        for _ in range(count):
            off, ln = struct.unpack_from("<HH", d, p)
            if tid == 0x8008:
                fonts.append(d[off << shift:(off << shift) + (ln << shift)])
            p += 12
    for f in fonts:
        ver, = struct.unpack_from("<H", f, 0)
        pts, = struct.unpack_from("<H", f, 0x44)
        if pts != points:
            continue
        ascent, = struct.unpack_from("<H", f, 0x4A)
        height, = struct.unpack_from("<H", f, 0x58)
        first, last = f[0x5F], f[0x60]
        table = 0x76 if ver == 0x200 else 0x94
        entry = 4 if ver == 0x200 else 6
        glyphs = []
        for i in range(last - first + 1):
            w, = struct.unpack_from("<H", f, table + i * entry)
            off = struct.unpack_from("<H" if ver == 0x200 else "<I", f, table + i * entry + 2)[0]
            cols = (w + 7) // 8
            rows = []
            for y in range(height):
                bits = []
                for c in range(cols):
                    byte = f[off + c * height + y]
                    bits += [(byte >> (7 - k)) & 1 for k in range(8)]
                rows.append(pack(bits[:w], w))
            glyphs.append((first + i, w, 0, 0, w, height, rows))
        return ascent, height, glyphs
    raise SystemExit(f"{path}: no {points} pt font")

def ttf_strike(path, ppem):
    t = TTFont(path)
    cmap = t.getBestCmap()
    strike = next(s for s in t["EBLC"].strikes if s.bitmapSizeTable.ppemY == ppem)
    hori = strike.bitmapSizeTable.hori
    ascent, height = hori.ascender, hori.ascender - hori.descender
    data = next(i for i, s in enumerate(t["EBLC"].strikes) if s is strike)
    bitmaps = t["EBDT"].strikeData[data]
    glyphs = []
    for code in range(32, 256):
        name = cmap.get(code)
        if name is None:
            continue
        if name not in bitmaps:
            # Blank glyphs (space) have no bitmap: advance from the outline metrics.
            adv = round(t["hmtx"][name][0] * ppem / t["head"].unitsPerEm)
            glyphs.append((code, adv, 0, 0, 0, 0, []))
            continue
        g = bitmaps[name]
        g.decompile()
        m = g.metrics
        w, h = m.width, m.height
        rows = [pack([(g.getRow(y, bitDepth=1, metrics=m)[x // 8] >> (7 - x % 8)) & 1 for x in range(w)], w) for y in range(h)]
        adv = getattr(m, "horiAdvance", getattr(m, "Advance", 0))
        bx = getattr(m, "horiBearingX", getattr(m, "BearingX", 0))
        by = getattr(m, "horiBearingY", getattr(m, "BearingY", 0))
        glyphs.append((code, adv, bx, ascent - by, w, h, rows))
    return ascent, height, glyphs

if __name__ == "__main__":
    src, out = sys.argv[1], sys.argv[2]
    write(os.path.join(out, "mssansserif8.bfnt"), *fon(os.path.join(src, "sserife.fon"), 8))
    write(os.path.join(out, "tahoma8.bfnt"), *ttf_strike(os.path.join(src, "tahoma.ttf"), 11))
    write(os.path.join(out, "tahoma8bold.bfnt"), *ttf_strike(os.path.join(src, "tahomabd.ttf"), 11))
