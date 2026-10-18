# Performance and correctness validation (Rust)

This document records what the Rust implementation measures today, how
it was validated against the implementation it replaced, and where it
honestly stands relative to specialized standard-7×6 solvers. Unless a
row says otherwise, measurements were taken in the constrained
2-core/2 GB Linux sandbox used for development; absolute nodes/second
will be higher on a normal desktop, and node counts (the portable
metric) are reported wherever possible.

## Build configuration

Following the [Rust Performance Book](https://nnethercote.github.io/perf-book/):

* engine/CLI workspace (`rust/Cargo.toml`): `opt-level = 3`,
  `lto = "fat"`, `codegen-units = 1`, `panic = "abort"`, `strip = true`;
* `rust/.cargo/config.toml`: `target-cpu=native` (delete for portable
  binaries);
* `[profile.test] opt-level = 2`, because the property tests run
  thousands of exact solves;
* Tauri GUI crate: thin LTO instead of fat — linking the GTK/WebKitGTK
  dependency tree with fat LTO needs several GB of RAM, and the engine
  code still compiles at opt-level 3.

The exact solver's hot loop is allocation-free: primitive `u64` board
words, fixed-capacity open-addressing tables with packed keys, and
preallocated per-ply move scratch arrays. The default transposition table is 2^22 entries
(smaller for small solves); `CONNECTK_TT_CAPACITY` overrides it.

## Startup and footprint

| Measurement | Value |
|---|---:|
| Stripped CLI binary | 641,208 bytes (~0.6 MB) |
| Stripped Tauri GUI binary | 5,814,864 bytes (~5.6 MB; uses system WebKitGTK) |
| No-work CLI call (`connectk theory --width 7 --height 6`) | ~0.016 s wall |
| Engine test suite | 36 tests, ~0.7–1.3 s |

There is no runtime to start and nothing to warm up: the first exact
solve runs at full speed.

## Parity with the previous solver

The port was required to be *semantically identical* before it was
allowed to be faster. Two checks pin that down:

* **Node parity.** The hard standard opening `1,3,7,1,2` at table size
  2^22 takes **76,563,549 nodes** — exactly the previous solver's count —
  and the Pons `endgame_easy` set totals 30,663 nodes, also exact.
  (At 2^21 the hard opening needs 84,311,585 nodes, which is why 2^22
  is the default.)
* **Book parity.** Regenerating `books/std5x4-d12.book` with the Rust
  `bookgen` produces all 120,204 entries with identical keys, scores,
  and best-column sets (~1.5 s with 8 worker threads).

## Exact solver on the Pons benchmark sets

`rust/core/examples/bench.rs` loads Pascal Pons's benchmark files
(`moves score` lines; download from
<https://github.com/PascalPons/connect4>, not committed here) and
converts scores between the two conventions.

```bash
# from rust/
cargo run --release -p connectk-core --example bench -- \
    /path/to/Test_L3_R1 1000 10000 fast
```

Fast exact solver, single thread, sandbox:

| Set | Solved | Total nodes | Nodes/s |
|---|---:|---:|---:|
| endgame_easy | 1000/1000 | 30,663 | — |
| midgame_easy | 1000/1000 | 811,860 | ~1.9 M |
| midgame_medium | 1000/1000 | 30,773,016 | ~1.7 M |
| opening_easy | 1000/1000 | 41,880,939 | ~1.7 M |
| opening_medium | 991/1000 (9 budget timeouts) | 1,087,606,263 incl. aborted | ~1.6 M |

Zero score mismatches against the published values.

## Depth-limited solver and PVS

The Rust port added interior principal-variation search to the
depth-limited solver (null-window probes for non-first moves,
re-searched inside the window). At equal depth the returned scores are
unchanged; only ties between equally valued moves may resolve
differently (one observed case: a mirror-symmetric pair). Wall-clock
at depth 14 on the built-in `depth_bench` positions, before → after:

| Position | Score | Before | After (PVS) |
|---|---:|---:|---:|
| empty 7×6 | −23 | 18.1 s | 12.6 s |
| after `4` | −1198 | 8.4 s | 4.0 s |
| `44337` | −1 | 1.7 s | 0.8 s |

Node counts can rise where null-window probes miss (empty board
+10 %); nodes/second rises more, so wall time still falls. In GUI play
this is what buys extra plies inside the same time budget.

## Claim-even validation

Generalized claim-even is tested, not assumed: the test suite checks
claim-even bounds against the generic exact solver on seeded random
positions, and the `claim_fuzz` example fuzzes random
playouts (latest short run: 800 games, 5,157 positions, 93
exact-checked firings, 0 violations).

## Honest comparison

Specialized standard-7×6 solvers (Pascal Pons's C++, its Rust ports,
Kite) reach roughly 8–20 M nodes/s on desktop hardware by hard-wiring
the 7×6 geometry. ConnectK deliberately stays generic — arbitrary
width/height, K = 3–6, no-diagonals, free placement, misère, published
theory, and opening books for variants no 7×6-only solver covers —
which costs constant factors in the inner loop. In this sandbox the
Rust engine is modestly faster per node than the solver it replaced
(~1.6–2.0 M vs ~1.3–1.5 M nodes/s) with dramatically better startup,
and identical node counts mean no solved position was lost in the move.
Beating the specialized solvers' raw 7×6 speed remains future work;
claiming otherwise would be dishonest.

Ideas measured and rejected during development (kept out deliberately):
column-history ordering in the fast solver (more nodes), transposition
probe gates tighter than 12 plies remaining, 2^23 default tables
(memory pressure, no node win), an MTD(f) root driver, and null-move /
late-move reductions anywhere near exact search (unsound for proofs).

Future work, in expected-value order: profile-guided optimization
(`cargo-pgo`) for release builds, a 7×6-specialized inner loop behind
the generic API, larger generated books for the 7×4/6×4 variants, and
desktop-hardware benchmark runs to replace the sandbox numbers above.
