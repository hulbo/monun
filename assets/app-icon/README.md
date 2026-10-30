# Monun app icon

A sheet of grid paper (모눈종이) with one selected cell and its fill handle, on a full-bleed green
tile (`#1f9d55`, deep green `#0f5c30`, paper `#fbfaf5`, grid `#a8dcbc`). The artwork is a
512 × 512 viewBox clipped to a rounded square (`rx=112`).

To regenerate after editing `monun.svg`: render each `hicolor/<n>x<n>` PNG and `monun-1024.png`
with `resvg -w <n> -h <n> monun.svg <out>`, copy the SVG to `hicolor/scalable`, render the macOS
files from the tile scaled into an 824/1024 body (`translate(100 100) scale(1.609375)` in a
1024 viewBox), pack `monun.ico` with `cargo xtask ico assets/app-icon/monun.ico <16…256 px PNGs>`
and `monun.icns` from the 128–1024 px macOS renders as PNG entries (`ic07`–`ic10`).

| File | Use |
|---|---|
| `monun.svg` | Canonical artwork; also `hicolor/scalable` |
| `monun-1024.png` | 1024 px render of the full tile |
| `monun-macos-512.png` | Runtime Dock icon on macOS |
| `monun.icns` | macOS bundle icon |
| `monun.ico` | Windows exe icon, 16–256 px (embedded by `apps/monun/build.rs`) |
| `hicolor/<size>/apps/kr.hulbo.monun.png` | Linux icon theme, 16–512 px; the 256 px one is also the runtime window icon |
| `hicolor/scalable/apps/kr.hulbo.monun.svg` | Linux scalable icon |
| `LICENSE.txt` | MIT OR Apache-2.0 |

The app ID is `kr.hulbo.monun`. `cargo xtask ico` packs square PNGs (≤ 256 px) into the `.ico`.
