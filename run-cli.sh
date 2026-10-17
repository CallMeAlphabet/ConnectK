#!/usr/bin/env bash
# Run the headless CLI. Builds first if the jars are missing.
set -euo pipefail
cd "$(dirname "$0")"
if [[ ! -f dist/connectk-cli.jar ]]; then
  ./build.sh cli
fi
JAVA_BIN="java"
if [[ -n "${JAVA_HOME:-}" && -x "$JAVA_HOME/bin/java" ]]; then
  JAVA_BIN="$JAVA_HOME/bin/java"
fi
exec "$JAVA_BIN" ${CONNECTK_JAVA_OPTS:-} --module-path dist \
  --module connectk.cli/connectk.cli.Main "$@"
