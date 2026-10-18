#!/usr/bin/env bash
# Run the ConnectK CLI. Builds the release binary first if missing.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")" && pwd)"
BIN="$ROOT/rust/target/release/connectk"

if [[ ! -x "$BIN" ]]; then
  "$ROOT/build.sh" cli
fi
exec "$BIN" "$@"
