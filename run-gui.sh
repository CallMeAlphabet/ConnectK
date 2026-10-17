#!/usr/bin/env bash
# Run the Swing GUI. Builds first if the jars are missing.
#
# Wayland behaviour:
#  - Stock OpenJDK: Swing is an X11 client and will run through XWayland
#    automatically. Fractional scaling may look soft; see README.
#  - JetBrains Runtime / Wakefield JDK: set CONNECTK_NATIVE_WAYLAND=1 to
#    enable the native Wayland toolkit with the same jar:
#        CONNECTK_NATIVE_WAYLAND=1 ./run-gui.sh
#    (On a stock OpenJDK this flag makes the JVM fail to start, because
#    WLToolkit does not exist there yet — that is expected.)
set -euo pipefail
cd "$(dirname "$0")"
if [[ ! -f dist/connectk-gui.jar ]]; then
  ./build.sh gui
fi
JAVA_BIN="java"
if [[ -n "${JAVA_HOME:-}" && -x "$JAVA_HOME/bin/java" ]]; then
  JAVA_BIN="$JAVA_HOME/bin/java"
fi
OPTS=()
if [[ "${CONNECTK_NATIVE_WAYLAND:-0}" == "1" ]]; then
  OPTS+=(-Dawt.toolkit.name=WLToolkit)
fi
if [[ -n "${CONNECTK_JAVA_OPTS:-}" ]]; then
  # shellcheck disable=SC2206
  OPTS+=(${CONNECTK_JAVA_OPTS})
fi
exec "$JAVA_BIN" "${OPTS[@]}" --module-path dist \
  --module connectk.gui/connectk.gui.ConnectKGui "$@"
