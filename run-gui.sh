#!/usr/bin/env bash
# Run the optional Tauri GUI. Builds the release binary first if missing.
#
# On Arch Linux, Tauri 2 needs the WebKitGTK 4.1 stack, for example:
#   sudo pacman -S --needed webkit2gtk-4.1 base-devel openssl \
#       appmenu-gtk-module libappindicator-gtk3 librsvg xdotool
# plus a stable Rust toolchain. The window is rendered by the system
# WebKitGTK webview, so it runs natively in a Wayland session.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")" && pwd)"
BIN="$ROOT/rust/gui/src-tauri/target/release/connectk-gui"

if [[ ! -x "$BIN" ]]; then
  "$ROOT/build.sh" gui
fi
# Run from the repository root so the GUI finds ./books/*.book.
cd "$ROOT"
exec "$BIN"
