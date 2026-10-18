#!/usr/bin/env bash
# Build ConnectK's Rust workspace.
#
# Usage:
#   ./build.sh          release-build the engine and CLI
#   ./build.sh test     run the engine/CLI test suite
#   ./build.sh core     release-build only connectk-core
#   ./build.sh cli      release-build only the CLI (includes core)
#   ./build.sh gui      release-build the optional Tauri GUI
#   ./build.sh all      release-build engine, CLI, and GUI (if present)
#   ./build.sh clean    remove Cargo build outputs
#
# The GUI is a separate Cargo workspace under rust/gui/src-tauri. It is
# intentionally not part of the default build: it needs the WebKitGTK
# system stack, while the engine and CLI need only a Rust toolchain.
set -euo pipefail
cd "$(dirname "$0")"

if ! command -v cargo >/dev/null 2>&1; then
  echo "error: cargo not found. Install Rust via https://rustup.rs/ or your package manager." >&2
  echo "On Arch Linux, for example:  sudo pacman -S rustup && rustup default stable" >&2
  exit 1
fi

build_gui() {
  if [[ -d rust/gui/src-tauri ]]; then
    (cd rust/gui/src-tauri && cargo build --release)
    echo "built rust/gui/src-tauri/target/release/connectk-gui"
  else
    echo "GUI directory absent; skipping GUI build"
  fi
}

TARGET="${1:-cli}"
case "$TARGET" in
  clean)
    (cd rust && cargo clean)
    if [[ -d rust/gui/src-tauri ]]; then
      (cd rust/gui/src-tauri && cargo clean)
    fi
    echo "cleaned"
    ;;
  test)
    (cd rust && cargo test)
    ;;
  core)
    (cd rust && cargo build --release -p connectk-core)
    ;;
  cli)
    (cd rust && cargo build --release -p connectk-cli)
    echo "built rust/target/release/connectk"
    ;;
  gui)
    build_gui
    ;;
  all)
    (cd rust && cargo build --release)
    build_gui
    ;;
  *)
    echo "unknown target: $TARGET" >&2
    echo "usage: ./build.sh [test|core|cli|gui|all|clean]" >&2
    exit 2
    ;;
esac
