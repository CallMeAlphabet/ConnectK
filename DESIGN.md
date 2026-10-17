# Design

## Module layout

```
core/   module connectk.core   — pure engine, depends on java.base only
  src/main/java/connectk/core/
    model/    Rules, Position (bitboard), Move, Player, Direction, Bits
    eval/     Evaluator (heuristic, used only by the depth-limited solver)
    solver/   ExactSolver, FastExactSolver (one-word gravity fast path),
              DepthLimitedSolver, ConnectKSolver (facade),
              SolveRequest, SearchResult, SolveMode, listeners/tokens
    theory/   TheoryDatabase (published results), TheoryResult
  src/test/java/connectk/core/TestMain.java   (zero-dependency test runner)

cli/    module connectk.cli    — headless command-line front end
gui/    module connectk.gui    — optional Swing front end
```

Dependency direction is strictly `gui -> core` and `cli -> core`. The core
contains no AWT/Swing/console code. **Deleting `gui/` (and `run-gui.sh`)
must always leave a working build** — `build.sh` silently skips missing
modules, and this is exercised by copying the tree, deleting `gui/`, and
rebuilding. The same holds for `cli/`.

JPMS `module-info.java` files enforce the boundaries at compile time, but
the jars also work on the plain classpath (that is how the test runner
uses the core jar).

## Rules as parameters

A `Rules` record carries width, height, k, placement (gravity/free), the
set of line directions (any subset — e.g. no diagonals), and the misère
flag. Rules are consulted in exactly three places:

- move generation (`Position.legalMoves`),
- line/threat detection (`Position.hasLine`, `Position.winningSpots`),
- terminal evaluation (`Position.winner`).

All search code is rules-agnostic. Consequences worth remembering:

- An opening book or theory entry is valid only for the exact `Rules`
  tuple it was computed for. `TheoryDatabase` therefore refuses to answer
  for anything but classical gravity Connect-4 with all directions.
- Turning diagonals off or changing k is a parameter change, but it
  changes the game completely — the solver must (and does) re-derive
  everything from search.

## Position representation

Two bitboards with layout `index = column*(height+1) + row`; the extra
bit per column is a never-set sentinel that prevents shift-based line
tests from wrapping across columns.

- `current` — stones of the side to move; `mask` — all stones.
  Opponent stones are `mask ^ current`.
- Play: `current ^= mask; mask |= bit`. Undo is the exact inverse, so
  search uses one mutable `Position` with play/undo (no allocation per
  node except transposition keys). Front ends use `copy()` snapshots.
- Bitboards are `long[]`, so boards needing more than 64 bits
  (e.g. 10x7 = 80 bits, 19x19 = 380 bits) work unchanged; the common
  one-word case has allocation-free fast paths for line tests and
  winning-spot computation.
- `winningSpots(player)` computes all cells completing a line for any k
  and any direction subset by AND-ing shifted copies of the player's
  bitboard over every "hole" offset in a k-window.
- `canonicalKey()` folds the left-right mirror into transposition keys,
  but only when the direction set is mirror-symmetric (both diagonals or
  neither).

## Solvers and the meaning of a result

`SearchResult` always carries a `Certainty`:

| Certainty | Meaning |
|---|---|
| `PROVEN_EXACT` | Exhaustive search in this run proved the outcome and the move. |
| `PROVEN_BY_THEORY` | Outcome from published results (see docs/THEORY.md). `moveIsProvenOptimal` says whether the move is published too. |
| `HEURISTIC_DEPTH_LIMITED` | Depth-limited search + heuristic evaluation. A strength estimate, never a proof. |
| `UNKNOWN_LIMIT_REACHED` | Budget ran out; no conclusion. |

### ExactSolver

Negamax to the end of the game:

- alpha-beta windows; exact score convention `±(cells - finalPly + 1)`
  (faster wins / slower losses preferred), 0 = draw;
- transposition table (canonical keys, exact/lower/upper bounds);
- tactical shortcuts: immediate line completion wins; a single opponent
  threat restricts the move list to the block; two distinct opponent
  threats are an exact loss in 2 plies; in misère, line-completing moves
  are suicidal and filtered;
- move ordering: centre-first, block opponent threats, then by the number
  of winning spots created; moves that hand the opponent an immediate
  line are deprioritised;
- hard node/time/cancellation budgets → `UNKNOWN`, never a guess.

Correctness is anchored by tests: the optimized solver is cross-checked
against an independent brute-force alpha-beta (no table, no shortcuts) on
random positions across rule variants, and against published outcomes
(4x4 draw, 6x4 second-player win, 3x3 k=3 draw, standard tactics).

Practical reach (this Java implementation, single thread): small boards
from the empty position (e.g. 5x4 in ~0.2 s, 6x4 in ~2 s), standard 7x6
positions from roughly the last ~25 empty cells depending on how tactical
they are. The empty 7x6 standard game is answered from published theory,
not recomputed — a from-scratch proof needs billions of nodes even in
optimized C++.

### DepthLimitedSolver

Iterative-deepening negamax (depth 1, 2, 3, …) so a timeout keeps the
last complete iteration. Depth-aware transposition table, history
heuristic, threat-based ordering, and the `Evaluator` at the horizon:
a sum over all potential k-lines, geometric in stone count, discounted
by the support stones a gravity threat still needs, and boosted for
immediately playable threats.

For free-placement boards with many empty cells, candidate moves are
restricted to cells within Chebyshev distance 2 of a stone (capped after
ordering). This is standard for Gomoku-style play and is disclosed in the
result explanation — one more reason these results are labelled
heuristic.

### ConnectKSolver (facade) — AUTO policy

1. Published theory with a published move → answer immediately.
2. Tractable-looking positions (few cells remaining or small boards) →
   exact attempt within its own small budget.
3. Otherwise depth-limited search. If theory knows the outcome but not a
   move, the outcome is reported as proven and the move (from depth
   search) is explicitly marked not certified.

`EXACT` mode never falls back to heuristics. `DEPTH_LIMITED` mode never
consults theory and never claims a proof.

## GUI and Wayland

The GUI is a single Swing window:

- All search runs in a `SwingWorker` with a cooperative cancellation
  token; the EDT never searches. Board input is disabled during a search
  so results cannot be applied to a changed position.
- Board rendering is vector Java2D (no image assets), so it scales
  cleanly.
- Wayland-safe choices: `setLocationByPlatform(true)` (the compositor
  places the window — clients cannot know absolute coordinates under
  Wayland), no `java.awt.Robot`, no system tray, no always-on-top, no
  multi-window choreography. Rendering goes through XWayland on stock
  OpenJDK and through the native `WLToolkit` on JetBrains Runtime /
  Wakefield builds with zero code changes (see README).

## Testing

`./build.sh test` compiles and runs `connectk.core.TestMain`, a
zero-dependency runner (no JUnit download). It covers model mechanics,
rule variants (diagonals off, k=5, misère, free placement, multi-word
boards), exact-solver correctness vs. brute force and vs. published
results, theory-database behaviour, and solver labelling honesty
(a DEPTH_LIMITED run must not claim a proof).

## Deliberate non-goals (this version)

- Official **Connect6** (two stones per turn) — unsolved game; would need
  a different move type and threat machinery.
- Threat-space / proof-number search for large free-placement boards
  (that is how Gomoku was weakly solved; our free-placement exact search
  intentionally covers small boards only).
- PopOut-style cyclic rule sets (positions can repeat, so plain negamax
  does not terminate; needs proof-number search or retrograde analysis).
- Parallel search. The solvers are single-threaded and single-use;
  parallelising root moves would be the natural next step.
