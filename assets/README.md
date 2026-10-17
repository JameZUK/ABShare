# ABShare icons

An open book with sound radiating from it, as from a speaker. The style matches the sibling
project Lyrion Anywhere (dark gradient tile, single pale stroke colour, round caps, fading
sound arcs), in a warm paper tone (`#fbe7c6`) for audiobooks.

| File | Use |
|---|---|
| `logo.svg` | Master artwork (512 × 512, rounded tile) |
| `icon.svg`, `icon-192.png`, `icon-512.png` | PWA manifest icons (`purpose: any`) |
| `icon-maskable.svg`, `icon-maskable-*.png` | PWA maskable icons (artwork inside the 80 % safe zone) |
| `favicon.svg` | Browser tab (heavier strokes for 16–32 px) |
| `apple-touch-icon.png` | iOS Home Screen (180 × 180) |

PNGs are rendered from the SVGs with `rsvg-convert -w N -h N icon.svg -o icon-N.png`.
