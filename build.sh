#!/usr/bin/env bash
# Zero-dependency build: only a JDK 21+ is required (no Gradle/Maven).
#
# Usage:
#   ./build.sh          build all modules that exist (core, cli, gui)
#   ./build.sh test     build and run the core test suite
#   ./build.sh core     build only the core module
#   ./build.sh cli      build core + cli
#   ./build.sh gui      build core + gui
#   ./build.sh clean    remove build/ and dist/
#
# Modules whose directory does not exist are skipped silently. That is the
# point of the layout: delete the gui/ directory and this script still
# builds core + cli. Delete cli/ too and you have a pure engine library.
set -euo pipefail
cd "$(dirname "$0")"

if [[ -n "${JAVA_HOME:-}" && -x "$JAVA_HOME/bin/javac" ]]; then
  JAVAC="$JAVA_HOME/bin/javac"
  JAVA="$JAVA_HOME/bin/java"
  JAR="$JAVA_HOME/bin/jar"
else
  JAVAC="$(command -v javac || true)"
  JAVA="$(command -v java || true)"
  JAR="$(command -v jar || true)"
fi
if [[ -z "${JAVAC:-}" ]]; then
  echo "error: javac not found. On Arch Linux:  sudo pacman -S jdk21-openjdk" >&2
  exit 1
fi
VERSION="$("$JAVAC" -version 2>&1 | awk '{print $2}' | cut -d. -f1)"
if [[ "$VERSION" -lt 21 ]]; then
  echo "error: JDK 21+ required, found javac $VERSION" >&2
  echo "On Arch Linux:  sudo pacman -S jdk21-openjdk   (and set JAVA_HOME if needed)" >&2
  exit 1
fi

TARGET="${1:-all}"
case "$TARGET" in
  clean) rm -rf build dist; echo "cleaned"; exit 0 ;;
esac

mkdir -p build dist
compile_module() {
  local name="$1" main_class="${2:-}" extra_path="${3:-}"
  local out="build/$name"
  mkdir -p "$out"
  find "$name/src/main/java" -name '*.java' > "build/$name.sources"
  if [[ -n "$extra_path" ]]; then
    "$JAVAC" --release 21 --module-path "$extra_path" -d "$out" @"build/$name.sources"
  else
    "$JAVAC" --release 21 -d "$out" @"build/$name.sources"
  fi
  local jar_file="dist/connectk-$name.jar"
  if [[ -n "$main_class" ]]; then
    "$JAR" --create --file "$jar_file" --main-class "$main_class" -C "$out" .
  else
    "$JAR" --create --file "$jar_file" -C "$out" .
  fi
  echo "built $jar_file"
}

compile_module core

want_cli=0; want_gui=0; run_tests=0
case "$TARGET" in
  all)  want_cli=1; want_gui=1 ;;
  test) want_cli=1; want_gui=1; run_tests=1 ;;
  core) ;;
  cli)  want_cli=1 ;;
  gui)  want_gui=1 ;;
  *) echo "unknown target: $TARGET" >&2; exit 2 ;;
esac

if [[ "$want_cli" == 1 && -d cli ]]; then
  compile_module cli connectk.cli.Main dist/connectk-core.jar
fi
if [[ "$want_gui" == 1 && -d gui ]]; then
  compile_module gui connectk.gui.ConnectKGui dist/connectk-core.jar
fi

if [[ "$run_tests" == 1 ]]; then
  mkdir -p build/test
  find core/src/test/java -name '*.java' > build/test.sources
  "$JAVAC" --release 21 -cp dist/connectk-core.jar -d build/test @build/test.sources
  "$JAVA" -cp "build/test:dist/connectk-core.jar" connectk.core.TestMain
fi
