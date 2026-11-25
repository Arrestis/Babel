#!/usr/bin/env bash
# Builds a portable AppImage that runs on most Linux distributions.
# Usage (from the project root, after `cargo build --release`):
#   bash packaging/linux/make-appimage.sh [path/to/babelcraft-binary]
set -euo pipefail
BIN=${1:-target/release/babelcraft}
ARCH=$(uname -m)
VERSION=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)

APPDIR=dist/BabelCraft.AppDir
rm -rf "$APPDIR" && mkdir -p "$APPDIR/usr/bin" "$APPDIR/usr/share/applications" "$APPDIR/usr/share/icons/hicolor/256x256/apps"
cp "$BIN" "$APPDIR/usr/bin/babelcraft"
cp packaging/linux/babelcraft.desktop "$APPDIR/usr/share/applications/"
cp packaging/linux/babelcraft.desktop "$APPDIR/"
cp assets/icon-256.png "$APPDIR/usr/share/icons/hicolor/256x256/apps/babelcraft.png"
cp assets/icon-256.png "$APPDIR/babelcraft.png"
cat > "$APPDIR/AppRun" <<'EOF'
#!/bin/sh
HERE="$(dirname "$(readlink -f "$0")")"
exec "$HERE/usr/bin/babelcraft" "$@"
EOF
chmod +x "$APPDIR/AppRun"

TOOL=dist/appimagetool-$ARCH.AppImage
if [ ! -x "$TOOL" ]; then
  curl -fsSL -o "$TOOL" "https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-$ARCH.AppImage"
  chmod +x "$TOOL"
fi
ARCH=$ARCH APPIMAGE_EXTRACT_AND_RUN=1 "$TOOL" "$APPDIR" "dist/BabelCraft-$VERSION-linux-$ARCH.AppImage"
echo "Built dist/BabelCraft-$VERSION-linux-$ARCH.AppImage"
