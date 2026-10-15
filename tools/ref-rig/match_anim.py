#!/usr/bin/env python3
"""match_anim.py SHEET FW FH DIR [cx cy] [search]
For every captured frame in DIR (files named <ns>.png), find the sheet frame that best matches the
region centred at (cx, cy) (searching +-search px), using only the sprite's non-black pixels.
Prints time (ms from first file), best frame, offset and error. Both quantised to 5 bits/channel."""
import sys, os
import numpy as np
from PIL import Image

sheet_path, fw, fh, d = sys.argv[1], int(sys.argv[2]), int(sys.argv[3]), sys.argv[4]
cx, cy = (int(sys.argv[5]), int(sys.argv[6])) if len(sys.argv) > 6 else (200, 200)
search = int(sys.argv[7]) if len(sys.argv) > 7 else 3
q = lambda a: (a >> 3).astype(np.int16)
sheet = q(np.asarray(Image.open(sheet_path).convert('RGB')))
cols, rows = sheet.shape[1] // fw, sheet.shape[0] // fh
frames = []
for i in range(cols * rows):
    f = sheet[(i // cols) * fh:(i // cols + 1) * fh, (i % cols) * fw:(i % cols + 1) * fw]
    m = f.sum(axis=2) > 0
    if m.sum() > 0:
        frames.append((i, f, m))
files = sorted(os.listdir(d))
t0 = int(files[0][:-4])
for name in files:
    img = q(np.asarray(Image.open(os.path.join(d, name)).convert('RGB')))
    best = None
    for dy in range(-search, search + 1):
        for dx in range(-search, search + 1):
            x0, y0 = cx - fw // 2 + dx, cy - fh // 2 + dy
            reg = img[y0:y0 + fh, x0:x0 + fw]
            for i, f, m in frames:
                err = (np.abs(reg - f).sum(axis=2) > 1)[m].mean()
                if best is None or err < best[0]:
                    best = (err, i, dx, dy)
    print(f"{(int(name[:-4]) - t0) / 1e6:8.1f} ms  frame {best[1]:3d}  off {best[2]:+d},{best[3]:+d}  err {best[0]:.3f}")
