# Wine bitmap fonts

Glyph bitmaps converted from fonts shipped with Wine 5.5 (wine-staging), used to draw the
startup screen's Windows-style dialogs exactly as the original Continuum looks under Wine:

- `mssansserif8.bfnt` — MS Sans Serif 8 pt from Wine's `sserife.fon`
  (Copyright (C) 2004 Huw D M Davies, Dmitry Timoshkov)
- `tahoma8.bfnt`, `tahoma8bold.bfnt` — the 11 ppem embedded bitmap strikes of Wine's
  `tahoma.ttf` / `tahomabd.ttf`
- `courier15.bfnt` — Wine's `cour.ttf` as Wine rasterises it in the original's macro edits
  (no embedded bitmaps at that size, so the 94 glyphs were captured from the screen)

Wine's fonts are free software under the GNU Lesser General Public License, version 2.1 or
later (https://www.gnu.org/licenses/old-licenses/lgpl-2.1.html). These files are converted
data only; regenerate them with `tools/fonts/extract_wine_fonts.py <wine>/share/wine/fonts .`.
Format: see that script.
