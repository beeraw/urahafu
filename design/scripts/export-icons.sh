#!/usr/bin/env bash
# Regenerate all Urahafu icon assets (menu bar PNGs, app icon PNGs, iconset, .icns)
# from the two source SVGs. Safe to re-run; works from any cwd.
set -euo pipefail

# Resolve the directory this script lives in, regardless of cwd or symlinks.
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DESIGN_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

MENUBAR_DIR="$DESIGN_DIR/menubar-icon"
APPICON_DIR="$DESIGN_DIR/app-icon"
ASSETS_DIR="$DESIGN_DIR/../assets"

MENUBAR_SVG="$MENUBAR_DIR/menubar.svg"
APPICON_SVG="$APPICON_DIR/app-icon.svg"

for tool in rsvg-convert iconutil python3 magick; do
  if ! command -v "$tool" >/dev/null 2>&1; then
    echo "error: required tool '$tool' not found in PATH" >&2
    exit 1
  fi
done

echo "== Menu bar icon =="
rsvg-convert -w 18 -h 18 "$MENUBAR_SVG" -o "$MENUBAR_DIR/menubar.png"
rsvg-convert -w 36 -h 36 "$MENUBAR_SVG" -o "$MENUBAR_DIR/menubar@2x.png"
echo "  wrote menubar.png, menubar@2x.png"

echo "== Menu bar icon raw RGBA (for src/platform/tray.rs's include_bytes!) =="
mkdir -p "$ASSETS_DIR"
magick "$MENUBAR_DIR/menubar@2x.png" RGBA:"$ASSETS_DIR/menubar-icon@2x.rgba"
echo "  wrote $(basename "$ASSETS_DIR")/menubar-icon@2x.rgba (36x36, 4 bytes/px)"

echo "== App icon =="
rsvg-convert -w 1024 -h 1024 "$APPICON_SVG" -o "$APPICON_DIR/app-icon-1024.png"

ICONSET_DIR="$APPICON_DIR/Urahafu.iconset"
rm -rf "$ICONSET_DIR"
mkdir -p "$ICONSET_DIR"

# size:filename pairs required by iconutil, each rendered directly from the SVG
# (never upscaled/downscaled from another PNG) for maximum crispness.
ICON_SPECS=(
  "16:icon_16x16.png"
  "32:icon_16x16@2x.png"
  "32:icon_32x32.png"
  "64:icon_32x32@2x.png"
  "128:icon_128x128.png"
  "256:icon_128x128@2x.png"
  "256:icon_256x256.png"
  "512:icon_256x256@2x.png"
  "512:icon_512x512.png"
  "1024:icon_512x512@2x.png"
)

for spec in "${ICON_SPECS[@]}"; do
  size="${spec%%:*}"
  name="${spec##*:}"
  rsvg-convert -w "$size" -h "$size" "$APPICON_SVG" -o "$ICONSET_DIR/$name"
done
echo "  wrote app-icon-1024.png and $(basename "$ICONSET_DIR")/ (${#ICON_SPECS[@]} files)"

iconutil -c icns "$ICONSET_DIR" -o "$APPICON_DIR/Urahafu.icns"
echo "  wrote Urahafu.icns"

echo "== Previews =="
python3 "$SCRIPT_DIR/render_menubar_preview.py"
python3 "$SCRIPT_DIR/render_appicon_preview.py"
echo "  wrote menubar-icon/preview.png, app-icon/preview.png"

echo "Done."
