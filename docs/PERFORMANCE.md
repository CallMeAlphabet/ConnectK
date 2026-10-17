# Performance: benchmarks, and what we learned from Kite

This document records (1) how we benchmark the exact solvers,
(2) measured before/after numbers for the fast solver, (3) a
technique-by-technique analysis of the
[Kite](https://github.com/tristan852/kite) Java Connect Four solver
(read from its source, Oct 2026) versus this project, (4) round-2
results (dual-bound TT, MTD(f) experiment, parallel proving, opening
books) including the negative results, (5) what the
[WeakC4](https://github.com/2swap/WeakC4) weak solution teaches, and
(6) the remaining roadmap.

**Licensing note.** Kite and WeakC4 are GPLv3 and Pascal Pons's reference solver
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

### Round 2 (dual-bound TT, TT-move ordering, MTD(f) experiment, parallelism, books)

Same protocol and subsets, after this round's changes. Node counts on
the sets are essentially unchanged vs round 1 (our threat ordering +
PVS was already doing most of the work); per-node throughput improved
slightly (midgame_medium 43,501 nodes/pos at ~2.03M nodes/s):

| Test set | N | Fast r1 nodes/pos | Fast r2 nodes/pos | MTD(f) nodes/pos |
|---|---:|---:|---:|---:|
| endgame_easy | 1000 | 34 | 34 | 44 |
| midgame_easy | 100 | 216 | 214 | 369 |
| midgame_medium | 50 | 43,774 | 43,501 | 42,151 |
| opening_easy | 100 | 48,307 | 48,042 | 63,324 |
| opening_medium | 100 | 1,189,763 | 1,193,197 | 1,389,167 |

* **MTD(f): implemented, measured, reverted.** With a PVS core that
  already probes siblings with null windows, MTD(f)'s repeated root
  passes are pure overhead for *strong* solving: +17–31% nodes on the
  opening sets. This persisted after upgrading the TT to store both
  bounds, so bound-thrashing was not the cause. The dual-bound TT and
  TT-move ordering were kept (≈neutral here, principled, and a
  prerequisite if MTD is revisited for *weak* solving); the PVS root
  driver stays. The bounded search built for the experiment remains
  as `FastExactSolver.searchBound`.
* **Parallel single-position proving: correct, not yet faster at
  2 cores.** On hard opening `13712` (first `opening_hard` entry):
  sequential 71.4 s / 113.9M nodes; naive frontier splitting
  (exact-solve all 49 depth-2 subpositions) UNKNOWN @300 s after
  920M nodes — it loses root bound sharing and the shared TT, a
  useful negative result now documented in `ParallelExactSolver`.
  The replacement (root PVS splitting: first move exact, siblings
  refuted by parallel null-window tests) proves it in 94.0 s /
  171.0M nodes on 2 threads — still behind sequential, whose single
  shared TT exploits transpositions between sibling subtrees. The
  known fix is a lockless *shared-TT* parallel search (YBWC-style);
  until then `--threads` is opt-in and mainly helps batch work.
* **Opening books: infrastructure + generator work as designed.**
  `bookgen` on 6×4, depth 8: 30,447 canonical positions, all solved,
  29 s on 2 threads (checkpointed/resumable). Effect: the empty-6×4
  proof drops from 338,533 nodes / 205 ms to 2,250 nodes / 14 ms;
  a position *at* book depth answers in 1 node / 0 ms. This is the
  same mechanism that makes Kite's opening numbers look magical.
  Sizing honesty for 7×6: a 50-position depth-10 sample cost
  ~2.0M nodes ≈ 1.3 core-seconds per position, and there are on the
  order of a million canonical positions at that depth — so a full
  standard frontier book is hundreds of core-hours with the current
  solver (days on a many-core desktop, resumable throughout), and
  gets cheaper with every node-count improvement above. Kite's
  68M-position book was certainly a comparable compute project.

## What Kite does (from reading its source)

Kite (`net.kite.internal.board.Board`, ~1,700 lines) is a
standard-7×6-only, single-threaded exact solver. Its speed comes
from stacking many node-reduction techniques, plus a large
precomputed opening book:

| Technique | Kite | This project (after FastExactSolver) |
|---|---|---|
| Bitboard state | 2 longs, **8 bits/column** so mirroring is `Long.reverseBytes`; “ceiling” bitboard for landing cells | 2 longs, `height+1` bits/column (generic sizes); mirror by column-chunk reversal. conceptually the same |
| Position key / hash | canonical `active \| ceiling`, Murmur3-finalizer mix | canonical (current, mask) pair, splitmix-style mix; full-key verification |
| Transposition table | direct-mapped 64K entries, **1 long/entry** (48-bit partial hash + separate 8-bit min & max score bounds), always-replace + bound merging; plus a 16K “important positions” cache for expensive nodes | open-addressing with full-key verification, **separate lower+upper bounds merged on store** (round 2) + best move, ~2M entries, sized to position |
| Search driver | **MTD(f)-style weighted binary search** over a null-window core (bound “weights” bias the pivot) | PVS. MTD(f) tried and reverted (+17–31% nodes on openings with a PVS core — see round 2) |
| Direct score bounds | precomputed min/max tables per stone count; “no immediate win” variants clamp every node | equivalent bounds computed arithmetically (win ≥3 plies / loss ≥4 plies) |
| Tactical pruning | forced block, **stacked threat** (threat above the block) = loss, moves below opponent threats excluded, double-threat creation detected during ordering | all of these except ordering-time double-threat return (ours is found one ply later via the double-threat shortcut) |
| Shallow child probes | before recursing, probes TT for *all children* to find immediate refutations / known failing moves | not yet (roadmap) |
| Knowledge rules | **claim-even** (Victor Allis) implemented with bitboards, caps score at 0 or −1/−2/−3 by row pairs; “can no longer make a non-vertical line” cap | not yet — 7×6-specific knowledge; a generic version needs care (roadmap, standard-rules only) |
| Move ordering | tuned per-cell score tables (separate per player!) + popcounts of connection opportunities, immediate threats, “soon” threats (one cell above playable), column forks | centre + threat-cell + winning-spots-created popcount. soon-threat/fork features are roadmap |
| Opening book | **minimal perfect hash**: 16 MB bucket seeds + 6-bit packed scores for ~68M positions (plies ≤5, =10, =15), ~68 MB via Git LFS; also used *retrogradely* one ply above book depths | generated by our own solver (`bookgen`, any fast-path rules/depth, resumable); sorted-array binary-search lookup keyed by canonical (cur, mask) — simpler than perfect hashing, works for variants Kite cannot cover at all. Book *size* is now a compute choice, not missing tech |
| Symmetry | mirror-canonical everywhere | yes (when the direction set is mirror-symmetric) |
| Extras | skill levels/Elo estimation, game/move quality analysis, GraalVM native CLI | GUI/CLI analysis instead; skill levels out of scope |

Also noteworthy: Kite reuses one solver instance (and its caches)
across calls, and its caches are *tiny* (64K/16K entries) — with a
strong ordering + bounds, most searches never need a huge table; our
position-sized table follows the same insight less extremely.


## What WeakC4 teaches (2swap's search-free weak solution)

[WeakC4](https://2swap.github.io/WeakC4/explanation/) is a *weak*
solution for standard 7×6: a strategy that guarantees a first-player
win from the empty board, with **no search at runtime** and ~12 KB
of data. Structure (from the explanation page and the repo data,
Oct 2026; GPLv3 — ideas only, as with Kite):

* 654 opening-tree entries (`branches.json`). Following the tree,
  play reaches a "steady state" leaf after at most 18 plies
  (522 leaf entries, most at plies 8–14).
* Only **384 steady-state diagrams** cover all leaves. A diagram
  labels every empty cell with a priority level; to move: win if
  possible, block if forced, else play the unique playable cell at
  the first priority level containing exactly one playable cell
  (levels with ≥2 "cancel"). Victor Allis's *claim-even* (answer in the same
  column, claiming all even rows) is the special case that
  motivated the language. Diagrams were found by genetic search,
  SAT reduction, and by hand, then brute-force verified.
* Verification branches only on *opponent* replies (the strategy
  fixes our moves), so re-checking the whole certificate is cheaper
  than solving from scratch — their validator reportedly re-verifies
  faster than Fhourstones solves.

Relevance to this engine:

1. **Claim-even-class parity knowledge is empirically overwhelming.**
   *Every* leaf of a complete winning strategy closes by a
   generalized claim-even argument within 18 plies. That is strong
   evidence that Allis's knowledge rules — claim-even first — are
   the biggest remaining node-count lever for our search (Kite has
   bitboard claim-even; we do not). It moves to the top of the
   roadmap, to be derived from Allis (1988) with a proof sketch and
   validated against the Pons expected scores and random
   cross-checks vs the generic solver before being trusted.
2. **Strategy certificates are a cheaper product than strong
   solving.** For "prove this empty board is a first-player win"
   questions (e.g. the unpublished no-diagonals 7×6), extracting and
   verifying a strategy (branch only on opponent replies) can be
   exponentially cheaper than strong search. A future
   extract/verify-strategy feature is on the roadmap; our exact
   solver + TT already implicitly contains such strategies.
3. It does **not** replace strong solving: WeakC4 answers nothing
   about arbitrary positions, the second player, or any variant
   (sizes, k, no-diagonals, misère…) — exactly the territory where
   this generic engine is the tool. And it was itself built using
   strong solvers (Fhourstones/Pons) as substrate, like our books.

## Roadmap (priority order)

1. **Knowledge bounds where provable — claim-even first** (Victor Allis
   1988; standard-7×6-shaped parity arguments generalized carefully
   to even-height gravity boards). WeakC4 (above) is the empirical
   case; Kite's bitboard claim-even is the in-search precedent. Only
   with a proof sketch in comments + exhaustive cross-validation.
2. **Ordering features**: soon-threats and column forks (generic-k
   versions of Kite's), shallow child TT probes before recursing.
3. **Shared-TT parallel search** (lockless TT + YBWC-style work
   distribution inside the tree) — the design that can actually
   beat sequential on single hard positions. Needs a careful
   soundness argument for concurrent bound reads/writes.
4. **Bigger generated books** on desktop hardware (`bookgen`):
   7×6 standard frontier (depth 10–14+), and variant books
   (no-diagonals, K=5) that no existing solver ships. Optional:
   retrograde use one ply above book depth, à la Kite.
5. **Strategy extraction/verification** (WeakC4-inspired): emit a
   compact winning strategy for a proven-win position and verify it
   by opponent-only branching — a cheap, independently checkable
   certificate, and a possible weak-solving path for the open
   no-diagonals board.
6. Bigger/bucketed TT experiments (e.g. direct-mapped + "important
   positions" two-tier cache like Kite's).
7. SIMD (Vector API) last: the 7×6 board lives in one scalar
   register and search is branch/hash dominated; expect little.

Done since round 1: dual-bound TT + TT-move ordering (kept),
opening-book infrastructure + parallel resumable `bookgen`,
parallel exact proving (`--threads`; root-PVS splitting — correct
and useful for batches, single-position speed still sequential-led
until item 3), `searchBound` bounded-search API. Tried and reverted
with measurements: MTD(f) root driver, all-exact frontier splitting.

## Practical settings for long exact attempts (CLI)

```bash
# Serious attempt at the (unpublished) no-diagonals 7x6 board:
CONNECTK_JAVA_OPTS="-Xmx8g" ./run-cli.sh solve --diagonals false \
    --mode exact --time 3600000 --nodes 0 --no-theory
```

The solver reports nodes explored when the budget expires, and
never guesses. (`--threads N` exists, but on current measurements
sequential is at least as fast for single hard positions — see
round 2; the shared-TT design in the roadmap is the real fix.)

Generate and use an opening book (any fast-path rules; resumable —
re-run after interruptions and it continues where it stopped):

```bash
# Gauge first, then generate (long-running on all cores; resumable):
./run-cli.sh bookgen --depth 12 --out books/std7x6-d12.book \
    --threads 8 --per-pos-time 120000 --limit 1000   # smoke test
./run-cli.sh bookgen --depth 12 --out books/std7x6-d12.book \
    --threads 8 --per-pos-time 120000                # full run, resumes
./run-cli.sh solve --mode exact --book books/std7x6-d12.book --moves 4,4,5
```

Book files (`*.book`) are gitignored: they are regenerable products
of the solver, not source.
