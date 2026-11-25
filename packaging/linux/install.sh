#!/usr/bin/env bash
# Installs BabelCraft for the current user (no root needed):
#   ~/.local/bin/babelcraft, desktop entry and icon.
# Run from the unpacked release folder or the project root after `cargo build --release`.
set -euo pipefail
BIN=babelcraft
[ -x "$BIN" ] || BIN=target/release/babelcraft
ICON=icon-256.png
[ -f "$ICON" ] || ICON=assets/icon-256.png
DESKTOP=babelcraft.desktop
[ -f "$DESKTOP" ] || DESKTOP=packaging/linux/babelcraft.desktop

install -Dm755 "$BIN" "$HOME/.local/bin/babelcraft"
install -Dm644 "$ICON" "$HOME/.local/share/icons/hicolor/256x256/apps/babelcraft.png"
install -Dm644 "$DESKTOP" "$HOME/.local/share/applications/babelcraft.desktop"
command -v update-desktop-database >/dev/null && update-desktop-database "$HOME/.local/share/applications" || true
echo "Installed. Make sure ~/.local/bin is on your PATH, then start BabelCraft from your app menu."
