# Performance: benchmarks, and what we learned from Kite

This document records (1) how we benchmark the exact solvers,
(2) measured before/after numbers for the fast solver, (3) a
technique-by-technique analysis of the
[Kite](https://github.com/tristan852/kite) Java Connect Four solver
(read from its source, Oct 2026) versus this project, and (4) the
remaining roadmap.

**Licensing note.** Kite is GPLv3 and Pascal Pons's reference solver
is AGPLv3. We study their *algorithms* — negamax/alpha-beta, bitboards,
transposition tables, null-window search, threat pruning, opening
books; all standard techniques described in the literature (Allis
1988; Tromp's Fhourstones; Pons's tutorial) — and implement them
independently. No code is copied. (Using Kite as a code dependency
instead was considered and rejected: it would impose GPL on
distribution and only covers standard 7x6, while this engine is
generic over board size, k, directions, placement and misère.)

## Benchmark protocol

`core/src/test/java/connectk/core/BenchMain.java` loads Pascal
Pons's public test sets (1,000 positions each: endgame/midgame/opening
× easy/medium/hard; move sequence + expected score) — the same
protocol used by Fhourstones and Kite. The data files are *not*
committed; see the download link in the class javadoc.

```bash
./build.sh test   # compiles test sources into build/test
java -cp build/test:dist/connectk-core.jar connectk.core.BenchMain \
    path/to/midgame_medium.txt 200 10000 fast    # or: generic
```

One solver instance is reused per run (its transposition table is
cleared per position, like Pons's `reset()`); a short JIT warm-up is
discarded. Reported throughput uses solver-reported search time.
Score agreement is checked against the expected value on every
position (Pons score = `sign * ceil(|ourScore|/2)`).

## Measured results (2026-10-17, sandbox: 2 vCPU, Temurin 21, -Xmx1200m)

Absolute numbers from a weak shared sandbox — use them for
*relative* comparison; expect several× higher nodes/sec on a modern
desktop. "Generic" = original `ExactSolver`, "Fast" = new
`FastExactSolver`. Subsets are the first N positions of each file.

| Test set | N | Generic nodes/pos | Generic ms/pos | Fast nodes/pos | Fast ms/pos |
|---|---:|---:|---:|---:|---:|
| endgame_easy | 1000 | 73 | 0.13 | 34 | 0.003 |
| midgame_easy | 100 | 2,699 | 5.4 | 216 | 0.07 |
| midgame_medium | 50 | 101,550 | 235 | 43,774 | 25 |
| opening_easy | 100 | 18/20 solved @30 s cap | ~5,300 (solved) | 48,307 | 27 |
| opening_medium | 100 | not attempted | – | 1,189,763 | 695 |
| opening_hard | 50 | not attempted | – | 48.3M (48/50 @120 s cap) | 30,200 |

Single-thread throughput: generic ≈ 0.43–0.63M nodes/s, fast ≈
1.6–1.9M nodes/s (solver-reported). The bigger factor on easy/medium
sets is *node count* (PVS + direct score bounds + non-losing-move
restriction), not just node speed.

Empty-board probes through the CLI (`--mode exact --no-theory`,
90 s cap, fast solver): standard 7×6 explored 144M nodes and
no-diagonals 7×6 explored 194M nodes, both inconclusive. For
reference, Kite solves the empty standard board with ~234M node
evaluations total (its README benchmark, desktop CPU) — our node
*rate* is now in a sane range; our node *count* for hard positions
is the remaining gap (see roadmap).

## What Kite does (from reading its source)

Kite (`net.kite.internal.board.Board`, ~1,700 lines) is a
standard-7×6-only, single-threaded exact solver. Its speed comes
from stacking many node-reduction techniques, plus a large
precomputed opening book:

| Technique | Kite | This project (after FastExactSolver) |
|---|---|---|
| Bitboard state | 2 longs, **8 bits/column** so mirroring is `Long.reverseBytes`; “ceiling” bitboard for landing cells | 2 longs, `height+1` bits/column (generic sizes); mirror by column-chunk reversal. ✅ conceptually same |
| Position key / hash | canonical `active \| ceiling`, Murmur3-finalizer mix | canonical (current, mask) pair, splitmix-style mix; full-key verification ✅ |
| Transposition table | direct-mapped 64K entries, **1 long/entry** (48-bit partial hash + separate 8-bit min & max score bounds), always-replace + bound merging; plus a 16K “important positions” cache for expensive nodes | open-addressing, exact/lower/upper + best move, ~2M entries, sized to position. Different trade-off; storing min/max separately is interesting |
| Search driver | **MTD(f)-style weighted binary search** over a null-window core (bound “weights” bias the pivot) | full-window alpha-beta with PVS. ⏳ MTD(f) is roadmap item #1 |
| Direct score bounds | precomputed min/max tables per stone count; “no immediate win” variants clamp every node | ✅ equivalent bounds computed arithmetically (win ≥3 plies / loss ≥4 plies) |
| Tactical pruning | forced block, **stacked threat** (threat above the block) = loss, moves below opponent threats excluded, double-threat creation detected during ordering | ✅ all of these except ordering-time double-threat return (ours is found one ply later via the double-threat shortcut) |
| Shallow child probes | before recursing, probes TT for *all children* to find immediate refutations / known failing moves | ❌ not yet (roadmap) |
| Knowledge rules | **claim-even** (Victor Allis) implemented with bitboards, caps score at 0 or −1/−2/−3 by row pairs; “can no longer make a non-vertical line” cap | ❌ 7×6-specific knowledge; a generic version needs care (roadmap, standard-rules only) |
| Move ordering | tuned per-cell score tables (separate per player!) + popcounts of connection opportunities, immediate threats, “soon” threats (one cell above playable), column forks | centre + threat-cell + winning-spots-created popcount. ⏳ add soon-threat/fork features |
| Opening book | **minimal perfect hash**: 16 MB bucket seeds + 6-bit packed scores for ~68M positions (plies ≤5, =10, =15), ~68 MB via Git LFS; also used *retrogradely* one ply above book depths | ❌ none yet (roadmap: generate our own with our solver). This is why Kite “solves” opening_hard in ~22 µs/457 nodes: it is mostly book lookups |
| Symmetry | mirror-canonical everywhere | ✅ (when the direction set is mirror-symmetric) |
| Extras | skill levels/Elo estimation, game/move quality analysis, GraalVM native CLI | GUI/CLI analysis instead; skill levels out of scope |

Also noteworthy: Kite reuses one solver instance (and its caches)
across calls, and its caches are *tiny* (64K/16K entries) — with a
strong ordering + bounds, most searches never need a huge table; our
position-sized table follows the same insight less extremely.

## Roadmap (priority order)

1. **MTD(f) / null-window strong-solver driver** on top of the fast
   core, incl. per-move analysis at the root. Expected: the single
   biggest node-count reduction on hard/drawish positions.
2. **Generated opening book** for standard 7×6 (exact scores to a
   fixed ply, produced offline by our own solver; compact on-disk
   format, not committed if large). Makes standard play *perfect*
   when combined with the exact endgame solver, and collapses
   opening_hard-type positions.
3. **Parallel frontier proving** for long CLI runs: split the game
   tree at ply N into independent subpositions, solve with a work
   queue across cores, checkpoint results to disk so runs resume.
   This is the realistic path for the open no-diagonals 7×6 board.
4. **Ordering features**: soon-threats and column forks (generic-k
   versions of Kite's), shallow child TT probes.
5. **Knowledge bounds where provable** (claim-even-style parity
   arguments are standard-7×6 specific; only add with a proof sketch
   in comments + brute-force cross-checks).
6. Bigger/bucketed TT experiments; separate min/max bound storage.
7. SIMD (AVX2/Vector API) last: the 7×6 board lives in one scalar
   register and search is branch/hash dominated; expect little.
   BMI2 bit tricks would need a native module — only if profiling
   demands it.

## Practical settings for long exact attempts (CLI)

```bash
# Serious attempt at the (unpublished) no-diagonals 7x6 board:
CONNECTK_JAVA_OPTS="-Xmx8g" ./run-cli.sh solve --diagonals false \
    --mode exact --time 3600000 --nodes 0 --no-theory
```

Single-threaded for now; the solver reports nodes explored when the
budget expires, and never guesses.
