# Nodal logo

- `nodal-mark.svg` — mark for dark backgrounds (ring #ECE7E1, dot #EE8A3F)
- `nodal-mark-light-bg.svg` — mark for light backgrounds (ring #171411)
- `nodal-app-icon.svg` — 1024×1024 macOS app icon (824px squircle-ish tile, 100px margin per Apple grid)

Colors: accent `#EE8A3F` (≈ oklch(0.74 0.15 55)), ink `#ECE7E1`, background `#171411`.
Wordmark: "Nodal", system font (SF Pro Display) bold, letter-spacing −0.045em. Tagline: "Agent OS for Claude".

Regenerate the app icons after editing `nodal-app-icon.svg`:

```sh
pnpm tauri icon src-tauri/icons/source/nodal-app-icon.svg
rm -rf src-tauri/icons/android src-tauri/icons/ios
```
