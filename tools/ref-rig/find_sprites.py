#!/usr/bin/env python3
"""find_sprites.py SHEET FW FH DIR [step] - list exact sprite-frame matches per capture.
For every step-th capture in DIR (files named <ns>.png), slides each non-empty frame of SHEET over
the whole image and prints (time ms, frame, centre x, centre y) for exact matches (5-bit colour)."""
import sys, os
import numpy as np
from PIL import Image
q = lambda a: (a >> 3).astype(np.int16)
sheet = q(np.asarray(Image.open(sys.argv[1]).convert('RGB')))
fw, fh, d = int(sys.argv[2]), int(sys.argv[3]), sys.argv[4]
step = int(sys.argv[5]) if len(sys.argv) > 5 else 1
cols = sheet.shape[1] // fw
frames = []
for i in range(cols * (sheet.shape[0] // fh)):
    f = sheet[(i // cols) * fh:(i // cols + 1) * fh, (i % cols) * fw:(i % cols + 1) * fw]
    m = f.sum(2) > 0
    if m.sum() > 3:
        # anchor pixel: the first opaque pixel, used to prefilter candidate positions
        ay, ax = np.argwhere(m)[0]
        frames.append((i, f, m, ay, ax, f[ay, ax]))
files = sorted(os.listdir(d))
t0 = int(files[0][:-4])
for name in files[::step]:
    img = q(np.asarray(Image.open(os.path.join(d, name)).convert('RGB')))
    H, W = img.shape[:2]
    hits = []
    for i, f, m, ay, ax, c in frames:
        ys, xs = np.nonzero((img == c).all(2))
        for y, x in zip(ys - ay, xs - ax):
            if y < 0 or x < 0 or y + fh > H or x + fw > W:
                continue
            if not (np.abs(img[y:y + fh, x:x + fw] - f).sum(2) > 1)[m].any():
                hits.append((i, int(x + fw // 2), int(y + fh // 2)))
    if hits:
        print(f"{(int(name[:-4]) - t0) / 1e6:8.1f}", sorted(hits))
