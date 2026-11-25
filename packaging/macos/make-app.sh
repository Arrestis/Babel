#!/usr/bin/env bash
# Builds BabelCraft.app (universal: Apple Silicon + Intel) and a .dmg.
# Run on macOS from the project root:  bash packaging/macos/make-app.sh [--features whisper,metal]
set -euo pipefail
FEATURES=("$@")
VERSION=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)

rustup target add aarch64-apple-darwin x86_64-apple-darwin
# (the ${…+…} form keeps macOS's older bash happy when no options are given)
cargo build --release --target aarch64-apple-darwin ${FEATURES[@]+"${FEATURES[@]}"}
cargo build --release --target x86_64-apple-darwin ${FEATURES[@]+"${FEATURES[@]}"}

APP=dist/BabelCraft.app
rm -rf "$APP" && mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
lipo -create -output "$APP/Contents/MacOS/babelcraft" \
  target/aarch64-apple-darwin/release/babelcraft \
  target/x86_64-apple-darwin/release/babelcraft
sed "s/1.0.0/$VERSION/g" packaging/macos/Info.plist > "$APP/Contents/Info.plist"

# Icon (.icns) from the 1024 px PNG.
ICONSET=dist/BabelCraft.iconset
rm -rf "$ICONSET" && mkdir -p "$ICONSET"
for s in 16 32 128 256 512; do
  sips -z $s $s assets/icon-1024.png --out "$ICONSET/icon_${s}x${s}.png" >/dev/null
  d=$((s * 2))
  sips -z $d $d assets/icon-1024.png --out "$ICONSET/icon_${s}x${s}@2x.png" >/dev/null
done
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/BabelCraft.icns"
rm -rf "$ICONSET"

# Ad-hoc signature so Gatekeeper lets it run after "Open" from the context menu.
codesign --force --deep --sign - "$APP" || true

hdiutil create -volname BabelCraft -srcfolder "$APP" -ov -format UDZO "dist/BabelCraft-$VERSION-macos-universal.dmg"
echo "Built $APP and dist/BabelCraft-$VERSION-macos-universal.dmg"
