# Generated opening books

Book files (`*.book`) are **regenerable products of the solver** and are
gitignored; this file records what has been generated and how to
regenerate or extend it. All books were produced by `bookgen` with the
exact solver and contain only proven scores (each entry was fully
solved; there are no heuristic entries).

## Inventory (generated 2026-10-17, 2 sandbox threads)

| File | Rules | Depth | Entries | Generation | Effect example (empty board) |
|---|---|---:|---:|---:|---|
| `std6x4-d12.book` | 6×4 K=4 | 12 | 684,064 | 101 s, 120M nodes | proof 338k nodes → 20k |
| `nodiag6x4-d12.book` | 6×4 K=4 no diagonals | 12 | 686,162 | 107 s, 216M nodes | proof 118k → 9.3k nodes; empty board is a proven **draw** |
| `std5x4-d12.book` | 5×4 K=4 | 12 | 120,204 | 6 s, 3M nodes | proof → 5.3k nodes |
| `std7x4-d10.book` | 7×4 K=4 | 10 | 552,811 | 1,297 s, 2.56B nodes | proof 1.98M → 10.4k nodes |

None of these exist in Kite (standard 7×6 only). Positions at book
depth answer in 1 node; shallower positions get exact cutoffs at the
book frontier throughout the search.

## Regenerate / extend

```bash
# any fast-path rules; resumable - safe to interrupt and re-run
./run-cli.sh bookgen --width 6 --height 4 --diagonals false --depth 12 \
    --out books/nodiag6x4-d12.book --threads 8 --per-pos-time 20000

# use a book
./run-cli.sh solve --width 6 --height 4 --diagonals false --mode exact \
    --book books/nodiag6x4-d12.book --time 60000 --nodes 0 --no-theory
```

## Sizing notes for bigger frontiers (measured samples)

- Standard 7×6 depth 10: ~2M nodes ≈ 1.3 core-s per position (round-2
  solver); on the order of a million canonical positions — days on a
  many-core desktop.
- 7×6 no-diagonals depth 8: ~20M nodes ≈ 10 core-s per position, some
  positions not solved within 60 s. This variant's empty board is
  unpublished precisely because these subpositions are hard.
- 7×6 K=5 depth 8: ~80 core-s per position. Frontier books here need
  either much more compute or a faster solver (claim-even does not
  apply to K=5's geometry as strongly; these runs predate claim-even
  gains only marginally).
