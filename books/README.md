# Generated opening books

Book files (`*.book`) are regenerable products of the solver and are generally gitignored. The complete standard depth-10 book is bundled with the core crate at `rust/core/assets/std7x6-d10.book`, so a fresh build can use it immediately. This guide covers generating, merging, verifying, and using additional books. Every generated entry contains an exact
game value: `bookgen` writes only positions its exact solver proved.
Entries whose budget expires are reported as unsolved and omitted,
never filled with a heuristic value.

## Formats

### v1 — one ruleset, one-word gravity

The original format, retained for compatibility and used for the
standard 7x6 book:

```text
# connectk-book v1 depth=8
# rules: width=7 height=6 K=4 placement=GRAVITY directions=V,H,DU,DD misere=false
<canonical current hex> <canonical mask hex> <exact score> <canonical best column>
```

Older v1 files may omit the `# rules:` line; those remain readable
and are identified by the caller (or, for merging, by the historical
`std{W}x{H}...` / `nodiag{W}x{H}...` filename convention). New
generation writes the rules line, and resuming a file with the wrong
rules or depth is rejected.

### v2 — universal multi-section format

One file, normally `books/universal.book`, can contain generated
sections for as many dense rulesets as you choose to generate:

```text
# connectk-book v2 universal
# section rules: width=7 height=6 K=4 placement=GRAVITY directions=V,H,DU,DD misere=false
# depth=8
...
# section rules: width=4 height=4 K=3 placement=FREE directions=V,H,DU,DD misere=false
# depth=4
cur=... mask=... score=... best=<column,row>
```

"Universal" means the format, generator, loader, and solvers work
for every dense ruleset — gravity or free placement, misère on or
off, any enabled directions, and multi-word boards. No finite file
can precontain books for infinitely many boards and K values;
sections are added as they are generated. Loaders select and merge
only sections matching the active rules.

A book generated with `--depth D` (and no `--moves`) contains
canonical positions **at ply D**. Search from shallower positions
hits the book when it descends to that ply. A root already deeper
than D will not hit that frontier; use a deeper book or a segment
book rooted near the positions you play.

## Quick start: standard 7x6 book

Do a dry run first (enumerates and counts, solves nothing), then a
retained sample to estimate speed on your machine. Sample entries
are exact and are reused when you resume the full run.

```bash
DRY_RUN=1 DEPTH=8 cargo run --locked --manifest-path rust/tools/Cargo.toml -- generate-books standard
SAMPLE=200 DEPTH=8 cargo run --locked --manifest-path rust/tools/Cargo.toml -- generate-books standard
DEPTH=8 cargo run --locked --manifest-path rust/tools/Cargo.toml -- generate-books standard
cargo run --locked --manifest-path rust/tools/Cargo.toml -- generate-books verify books/std7x6-d8.book
```

Useful knobs are environment variables, for example
`THREADS=12 PER_POS_TIME=120000 FLUSH_EVERY=100`.
`FLUSH_EVERY=1` (the default) is the most crash-safe; larger values
trade a few possibly-lost in-flight entries for less I/O.

Depth advice for standard 7x6:

- **Depth 8** (91,295 canonical frontier positions) is the sensible
  first complete book.
- **Depth 10** has 809,464 frontier positions.
- **Depth 12** has 5,832,236 frontier positions and will produce a
  file on the order of a quarter of a gigabyte in v1 text format.
  Estimate with `SAMPLE=` first and consider sharding (below).

### Shard a standard book across machines/processes

Shards are a deterministic partition of the canonical frontier in
traversal order. Run one command per shard (copy the repository to
each machine; do **not** point concurrent processes at one file):

```bash
SHARD=1/8 DEPTH=8 cargo run --locked --manifest-path rust/tools/Cargo.toml -- generate-books standard-shard
SHARD=2/8 DEPTH=8 cargo run --locked --manifest-path rust/tools/Cargo.toml -- generate-books standard-shard
cargo run --locked --manifest-path rust/tools/Cargo.toml -- generate-books merge books/std7x6-d8-shard-*.book
```

The merged output is `books/universal.book` by default and is found
automatically by the GUI; the CLI accepts it with `--book`. Each
shard still traverses the whole game tree to enumerate its slice —
sharding divides the expensive solving, not the cheap traversal.
Segment books (below) divide both.

### Segment books

`MOVES=` roots generation at a reachable position; `DEPTH` then
counts plies beyond that root. This is usually the fastest way to
make a useful standard book for openings that actually occur:

```bash
MOVES=4,4,3,3 DEPTH=8 cargo run --locked --manifest-path rust/tools/Cargo.toml -- generate-books standard-segment
```

Different segments can be appended to the same universal book and
are merged automatically when loaded.

## Universal and custom books

Generate a small starter set (5x4 and 6x4 gravity sections, free
3x3/K=3, and misère 4x4/K=4):

```bash
cargo run --locked --manifest-path rust/tools/Cargo.toml -- generate-books universal-starter
cargo run --locked --manifest-path rust/tools/Cargo.toml -- generate-books info books/universal.book
cargo run --locked --manifest-path rust/tools/Cargo.toml -- generate-books verify books/universal.book
```

Or generate exactly one custom rules section:

```bash
WIDTH=4 HEIGHT=3 K=3 PLACEMENT=free DEPTH=6 cargo run --locked --manifest-path rust/tools/Cargo.toml -- generate-books custom

WIDTH=5 HEIGHT=4 K=4 PLACEMENT=gravity MISERE=1 DEPTH=8 cargo run --locked --manifest-path rust/tools/Cargo.toml -- generate-books custom

WIDTH=9 HEIGHT=9 K=5 PLACEMENT=free DEPTH=4 SAMPLE=50 cargo run --locked --manifest-path rust/tools/Cargo.toml -- generate-books custom
```

For gravity-only variants, raw `bookgen` remains available:

```bash
cargo run --locked --manifest-path rust/cli/Cargo.toml -- bookgen --width 6 --height 4 --diagonals false --depth 12 \
    --out books/nodiag6x4-d12.book --threads 8 --per-pos-time 20000
```

## Inspect, merge, verify

```bash
cargo run --locked --manifest-path rust/cli/Cargo.toml -- book-info --book books/universal.book

cargo run --locked --manifest-path rust/tools/Cargo.toml -- generate-books merge books/std7x6-d8-shard-1of2.book books/std7x6-d8-shard-2of2.book

SAMPLE=256 cargo run --locked --manifest-path rust/tools/Cargo.toml -- generate-books verify books/universal.book
```

`book-merge` detects and rejects conflicting scores for the same
canonical position. `book-verify` distinguishes three outcomes:
verified, not independently proven within the supplied budget, and
hard failures (score mismatch, illegal/non-achieving stored best
move, or an unreconstructible key). Verification proves the sampled
entries under the same exact-score semantics as generation; set
`SAMPLE=0` if you need entry-by-entry verification of the whole
file.

## Using a book

```bash
cargo run --locked --manifest-path rust/cli/Cargo.toml -- solve --mode exact --book books/std7x6-d8.book
cargo run --locked --manifest-path rust/cli/Cargo.toml -- solve --width 4 --height 3 --k 3 --placement free \
    --mode exact --book books/universal.book
```

The default solver, CLI, and GUI automatically discover and merge
every matching candidate among named books, `books/universal.book`,
and `books/custom.book`. A partial deeper book therefore adds its
exact entries to the shipped complete book instead of shadowing it.
Discovery checks `$CONNECTK_BOOK_DIR`, project-local `books` directories, the crate's bundled asset directory, and `/usr/share/connectk/books` on Linux. It does not depend on launching from the repository root. The CLI's `--no-book` flag disables books entirely.

### Generate, verify, and inspect

From the repository root, generate or resume the standard depth-8 book, sample its verification, then print its inventory:

```bash
DEPTH=8 PER_POS_TIME=120000 FLUSH_EVERY=100 cargo run --locked --manifest-path rust/tools/Cargo.toml -- generate-books standard
SAMPLE=256 PER_POS_TIME=120000 cargo run --locked --manifest-path rust/tools/Cargo.toml -- generate-books verify books/std7x6-d8.book
cargo run --locked --manifest-path rust/tools/Cargo.toml -- generate-books info books/std7x6-d8.book
```

Depth 8 is the recommended first shippable standard book. For wider
opening coverage, use `DEPTH=10`; expect substantially more
generation time and a much larger file. Neither is a "quick" job:
estimate first with `SAMPLE=200`, and re-run the same command after
any interruption.

## Inventory

| File | Rules | Depth | Entries | Generation | Effect example (empty board) |
|---|---|---:|---:|---:|---|
| `std7x6-d10.book` | 7×6 K=4 | 10 | 809,464 | complete canonical frontier; 31,932,911 bytes | automatically loaded |

The complete standard book is also included in the `connectk-core` crate so a Cargo-installed CLI can discover it without a working-directory setting. Other generated books are not bundled by default.

## Sizing notes

- Standard 7×6 canonical frontier sizes: ply 4 = 568, ply 6 =
  8,231, ply 8 = 91,295, ply 10 = 809,464, ply 12 = 5,832,236.
- Enumeration keeps visited canonical keys in memory. Very deep
  frontiers on large boards can need substantial RAM even before
  solving starts; prefer segment books there.
