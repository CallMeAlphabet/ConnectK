# ConnectK

ConnectK is a Rust engine and command-line solver for Connect-K games. It supports gravity and free placement, configurable board dimensions and connect length, optional diagonals, and misère rules.

> **AI-use notice:** This project is maintained with AI assistance. Some code and documentation were generated or revised with AI tools. The maintainer reviews them, but errors may remain. Check results before relying on them.

## Contents

- [Quick start](#quick-start)
- [Usage](#usage)
- [Features](#features)
- [GUI](#gui)
- [Opening books](#opening-books)
- [Attribution](#attribution)

## Quick start

Install stable Rust, then clone and run the solver:

```bash
git clone https://github.com/CallMeAlphabet/ConnectK.git
cd ConnectK
cargo run --locked --manifest-path rust/cli/Cargo.toml -- solve
```

Or install the CLI from crates.io:

```bash
cargo install connectk-cli
connectk solve
```

## Usage

Solve a position or configure the rules directly:

```bash
connectk solve
connectk solve --moves 4,4,3,3,7
connectk solve --width 5 --height 4 --k 4
connectk solve --width 8 --height 8 --k 5 --placement free
connectk solve --width 7 --height 6 --misere
connectk theory --width 8 --height 8
connectk selfplay --width 4 --height 4 --mode exact
```

From a source checkout, prefix commands with:

```bash
cargo run --locked --manifest-path rust/cli/Cargo.toml --
```

Run `connectk help` to see all commands and options. The CLI supports time and node limits, parallel search, and opening-book generation and verification.

## Features

- Exact search when the position and search budget allow a proof.
- Theory support for published Connect Four results and mathematical cases.
- Depth-limited estimates for positions that are not proved. These are labelled heuristic.
- Opening-book generation, merging, and verification. The standard 7×6 depth-10 book is bundled with the core crate.
- Custom board dimensions and connect length `K`, gravity or free placement, optional diagonals, and misère play.
- CLI and optional Linux GUI.
- No third-party Rust crate dependencies.

Solver results use these certainty labels:

| Status | Meaning |
| --- | --- |
| `PROVEN_EXACT` | Search proved the result and best move. |
| `PROVEN_BY_THEORY` | A theorem or published result proves the outcome. A best move is marked proven only when known. |
| `HEURISTIC_DEPTH_LIMITED` | A depth-limited estimate, not a proof. |
| `UNKNOWN_LIMIT_REACHED` | The search budget ended without a proof or recommendation. |

Automatic solving uses applicable theory and opening books, then attempts exact search. If it cannot prove a result, it may return a depth-limited estimate. Discovered matching book files are merged with the bundled standard book.

## GUI

On Linux, `connectk gui` builds the GUI before starting it. The native GUI requires GTK 3 and WebKitGTK. On Arch Linux, install them with:

```bash
sudo pacman -S --needed gtk3 webkit2gtk-4.1
```

Launch the GUI from a source checkout with:

```bash
cargo run --locked --manifest-path rust/cli/Cargo.toml -- gui
```

Use `--browser` to open the interface in a browser, `--web` to serve it without opening a browser, or `--server` to run the server only. The local GUI server uses a per-session token and is intended for local use, not as a public web service.

## Opening books

The bundled standard 7×6 depth-10 book is `rust/core/assets/std7x6-d10.book`. Other matching books are discovered automatically.

Use the Rust tools to generate or verify books:

```bash
cargo run --locked --manifest-path rust/tools/Cargo.toml -- generate-books --help
cargo run --locked --manifest-path rust/tools/Cargo.toml -- generate-books universal-starter
cargo run --locked --manifest-path rust/tools/Cargo.toml -- generate-books verify books/universal.book
```

Generation can be resumed or split into shards. The v2 format stores sections for different rulesets. See [`books/README.md`](books/README.md) for options. Large books can take time and memory; use `SAMPLE` to estimate the work first.

## Attribution

ConnectK is an independent implementation. Its search design draws inspiration from published Connect Four solver work by Pascal Pons and from Kite. No solver source code from those projects is included.
