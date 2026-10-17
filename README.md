# Connect-k Solver

A generic Connect-4/5/6-style game engine and solver in pure Java 21.
One parameterised ruleset covers board width/height, line length **k**,
**diagonals on/off**, **gravity or free placement**, and **misère**
(making a line loses).

The solving policy is honest about mathematics:

- **Where a position is mathematically solved** — either published
  (theory database) or proved by exhaustive search in the run — the
  result is labelled **PROVEN**, with the exact outcome and, when known,
  a proven optimal move and the ply the game ends on.
- **Where there is no proof yet**, the engine plays with *depth-limited*
  search (iterative-deepening alpha-beta + heuristic evaluation) and the
  result is labelled **HEURISTIC — NOT a proof**.
- If a proof attempt runs out of budget, you get **UNKNOWN**, never a
  guess dressed up as a solution.

## Quick start (Arch Linux)

```bash
sudo pacman -S --needed jdk21-openjdk   # or jdk-openjdk (any JDK >= 21)

./build.sh test      # build everything, run the test suite
./run-gui.sh         # graphical UI
./run-cli.sh solve   # headless: analyse a position
```

No Gradle, no Maven, no downloads: `build.sh` uses only `javac`/`jar`.
Build outputs land in `dist/` (`connectk-core.jar`, `connectk-cli.jar`,
`connectk-gui.jar`) as JPMS modules that also work on the classpath.

## CLI examples

```bash
# Standard game, empty board -> answered from published theory
./run-cli.sh solve

# No-diagonals variant, depth-limited play (unpublished ruleset)
./run-cli.sh solve --diagonals false --mode depth --depth 12

# Prove a position exactly (here: 6x4 is a second-player win)
./run-cli.sh solve --width 6 --height 4 --mode exact --time 60000 --nodes 0 --no-theory

# Analyse a concrete game (1-based columns)
./run-cli.sh solve --moves 4,4,5,3,5,4 --mode auto

# Free placement (Gomoku-style), moves as column:row
./run-cli.sh solve --placement free --width 9 --height 9 --k 5 \
    --moves "5:5;5:6;6:5" --mode depth --depth 4

# What does published mathematics say about this board?
./run-cli.sh theory --width 8 --height 8

# Watch the engine play itself
./run-cli.sh selfplay --depth 8 --move-time 300

# Generate an exact opening book (parallel, resumable), then use it
./run-cli.sh bookgen --width 6 --height 4 --depth 8 --out 6x4-d8.book --threads 4
./run-cli.sh solve --width 6 --height 4 --mode exact --no-theory --book 6x4-d8.book --time 30000 --nodes 0
```

`--mode auto` (default) = theory where published, exact proof when
tractable, otherwise depth-limited. `--mode exact` returns UNKNOWN
rather than falling back. `--mode depth` never claims a proof.

## GUI

One window: rules bar on top (presets, W/H/K, placement, diagonals,
misère), the board in the centre (click a column/cell; hover shows the
landing disc), and an analysis panel on the right — published-theory
status of the current position, player modes (Human/AI in any
combination), solver mode/depth/time/node budgets, and an analysis view
whose badge states plainly whether the answer is *proven* or *heuristic*.
All searching happens off the UI thread with a working Cancel button.

![Board rendering](board-screenshot.png)

## Wayland (Arch)

Honest status of Java on Wayland in 2026:

- **Stock OpenJDK is still an X11 client.** On a Wayland session, Swing
  runs through **XWayland** automatically — this GUI works out of the
  box on GNOME/KDE/Sway/Hyprland etc. as long as XWayland is enabled in
  your compositor (default everywhere). JavaFX would not be any more
  native: its Linux backend also targets XWayland today.
- **Native Wayland exists via JetBrains Runtime / OpenJDK Project
  Wakefield** (the `WLToolkit`), and Swing is the toolkit it supports.
  If your `java` is a JetBrains Runtime (or a Wakefield build), the same
  jar runs natively:

  ```bash
  CONNECTK_NATIVE_WAYLAND=1 ./run-gui.sh
  ```

  On stock OpenJDK this makes the JVM fail to start (there is no
  WLToolkit yet) — expected; just run without the flag.
- **Fractional scaling**: XWayland apps can look soft at e.g. 125%/150%
  scaling, depending on compositor settings. The board is vector-drawn,
  so if needed you can force integer UI scaling, e.g.
  `CONNECTK_JAVA_OPTS="-Dsun.java2d.uiScale=2" ./run-gui.sh`, or use the
  native toolkit above, which renders crisply at fractional scales.
- The UI avoids everything that breaks under Wayland: no absolute window
  positioning (the compositor places the window), no `Robot`, no tray,
  no global coordinates.

References: OpenJDK Wakefield pure-Wayland toolkit prototype
<https://wiki.openjdk.org/display/wakefield/Pure+Wayland+toolkit+prototype>;
JetBrains, "Wayland By Default in 2026.1 EAP"
<https://blog.jetbrains.com/platform/2026/02/wayland-by-default-in-2026-1-eap/>;
OpenJFX XWayland status (JDK-8326710)
<https://bugs.openjdk.org/browse/JDK-8326710>.

## Removing the GUI

The GUI is one JPMS module and nothing depends on it:

```bash
rm -rf gui run-gui.sh
./build.sh        # still works: builds connectk-core.jar + connectk-cli.jar
./run-cli.sh solve
```

`build.sh` skips modules whose directory is absent. Deleting `cli/` as
well leaves a pure engine library (`./build.sh core`).

## What the solver can and cannot do

- Exact proofs: small boards from scratch (5x4 draw, 6x4
  second-player win — both near-instant), plus standard 7x6 positions
  once enough cells are filled / tactical. A primitive one-word
  bitboard fast path (`FastExactSolver`) handles gravity rules at
  millions of nodes/sec: Pons *opening_easy* benchmark positions
  (~28 empty cells) prove in tens of milliseconds each. Exact work
  can also be parallelized (`--threads`) and short-circuited by
  generated opening books (`bookgen` produces them offline for any
  fast-path rules, resumable; `--book` uses them) — e.g. a depth-8
  6x4 book cuts that empty-board proof from 338,533 nodes to 2,250.
  The empty
  standard board is answered from published theory rather than
  recomputed from scratch. See [docs/PERFORMANCE.md](docs/PERFORMANCE.md)
  for benchmarks and the comparison with the Kite solver.
- Depth-limited play uses an evaluation over all potential k-lines
  (gravity-aware threat support, playable-threat boosts). It plays
  sensibly at depth 8–12 on 7x6 but is *not* perfect — and says so.
- Not implemented: official Connect6 (two stones per turn, an unsolved
  game), PopOut (cyclic games need proof-number search / retrograde
  analysis), and large-board free-placement proofs (Gomoku was weakly
  solved with threat-space + proof-number search, a different machinery).

See [DESIGN.md](DESIGN.md) for architecture and [docs/THEORY.md](docs/THEORY.md)
for the published results and their sources.

## Layout

```
core/   engine: model, evaluator, solvers, theory database (java.base only)
cli/    headless command-line front end
gui/    optional Swing front end (delete freely)
build.sh / run-cli.sh / run-gui.sh
```
