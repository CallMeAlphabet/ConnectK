# ConnectK

ConnectK is a Rust engine and command-line solver for Connect-K games. It supports gravity and free placement, configurable board dimensions and connect length, optional diagonals, and misère rules.

> **AI-use notice:** This project is maintained with AI assistance. Some code, tests, and documentation were generated or revised with AI tools. The maintainer reviews them, but errors may remain. Check results before relying on them.

## Contents

- [Quick Start](#quick-start)
- [Features](#features)
- [Usage](#usage)
- [Build and Test](#build-and-test)
- [GUI](#gui)
- [Opening Books](#opening-books)
- [Attribution](#attribution)
- [License](#license)

## Quick Start

Install stable Rust, then build and run the CLI on Linux:

```bash
git clone https://github.com/CallMeAlphabet/ConnectK.git
cd ConnectK
cargo run --locked --manifest-path rust/cli/Cargo.toml -- solve
```

To install the CLI from crates.io:

```bash
cargo install connectk-cli
connectk solve
```

Arch Linux users can install a source package or prebuilt binaries from the AUR:

```bash
paru -S connectk
paru -S connectk-bin
```

The native GUI uses GTK 3 and WebKitGTK at runtime. On Arch Linux, install them with:

```bash
sudo pacman -S --needed gtk3 webkit2gtk-4.1
```

## Features

- Exact search when the position and search budget allow a proof.
- A theory layer for published Connect Four results and mathematical cases.
- Depth-limited search for positions that are not proved. These results are labelled as heuristic.
- Opening-book generation, merging, and verification. The standard 7×6 depth-10 book is bundled with the core crate.
- Configurable width, height, and connect length `K`, from 0 to 2,147,483,647. Rules also support gravity, free placement, diagonals, and misère play.
- A command-line interface and an optional Linux GUI.
- No third-party Rust crate dependencies.

Results use one of these certainty labels:

| Status | Meaning |
| --- | --- |
| `PROVEN_EXACT` | Search proved the result and best move. |
| `PROVEN_BY_THEORY` | A theorem or published result proves the outcome. The best move is marked proven only when it is known. |
| `HEURISTIC_DEPTH_LIMITED` | A depth-limited estimate, not a proof. |
| `UNKNOWN_LIMIT_REACHED` | The search budget ended without a proof or recommendation. |

In automatic mode, the solver uses applicable theory and opening books, then tries exact search. If it cannot prove a result, it may return a depth-limited estimate. Matching book files are merged; a partial book does not replace the bundled standard book.

## Usage

Run the CLI from the repository with Cargo:

```bash
cargo run --locked --manifest-path rust/cli/Cargo.toml -- solve
cargo run --locked --manifest-path rust/cli/Cargo.toml -- solve --moves 4,4,3,3,7
cargo run --locked --manifest-path rust/cli/Cargo.toml -- solve --width 5 --height 4 --k 4
cargo run --locked --manifest-path rust/cli/Cargo.toml -- solve --width 8 --height 8 --k 5 --placement free
cargo run --locked --manifest-path rust/cli/Cargo.toml -- solve --width 7 --height 6 --misere
cargo run --locked --manifest-path rust/cli/Cargo.toml -- theory --width 8 --height 8
cargo run --locked --manifest-path rust/cli/Cargo.toml -- selfplay --width 4 --height 4 --mode exact
```

The CLI also accepts time and node limits, supports parallel search, and can generate or verify books. Run `connectk help` for all commands and options.

## Build and Test

Install stable Rust and Cargo. Build the CLI and core engine with:

```bash
cargo build --locked --release --manifest-path rust/Cargo.toml
```

Build the GUI with:

```bash
cargo build --locked --release --manifest-path rust/gui/Cargo.toml
```

Run the engine, CLI, tools, and GUI tests with:

```bash
cargo test --locked --manifest-path rust/Cargo.toml
cargo test --locked --manifest-path rust/gui/Cargo.toml
```

The Rust utilities in `rust/tools` provide build and run commands:

```bash
cargo run --locked --manifest-path rust/tools/Cargo.toml -- build all
cargo run --locked --manifest-path rust/tools/Cargo.toml -- build clean
```

The release profile enables link-time optimization. The local Cargo configuration enables CPU-specific optimizations. Remove `rust/.cargo/config.toml` when building for other CPUs.

## GUI

From the repository, `connectk gui` builds the GUI before starting it. To run it through Cargo:

```bash
cargo run --locked --manifest-path rust/cli/Cargo.toml -- gui
```

The GUI can open in a browser or serve the page without opening a window:

```bash
cargo run --locked --manifest-path rust/cli/Cargo.toml -- gui --browser
cargo run --locked --manifest-path rust/cli/Cargo.toml -- gui --web
cargo run --locked --manifest-path rust/cli/Cargo.toml -- gui --server
```

The GUI server listens on loopback and uses a per-session token. It is intended for local use, not as a public web service. Changing the rules starts a new game. In AI-vs-AI mode, players cannot make moves manually.

## Opening Books

The standard 7×6 depth-10 book is at `rust/core/assets/std7x6-d10.book`. The core crate includes it, and the CLI, engine, and GUI load it automatically. Other books are loaded when their rules match.

Use the Rust tools to generate and check books:

```bash
cargo run --locked --manifest-path rust/tools/Cargo.toml -- generate-books --help
DRY_RUN=1 DEPTH=8 cargo run --locked --manifest-path rust/tools/Cargo.toml -- generate-books standard
SAMPLE=200 DEPTH=8 cargo run --locked --manifest-path rust/tools/Cargo.toml -- generate-books standard
DEPTH=8 cargo run --locked --manifest-path rust/tools/Cargo.toml -- generate-books standard
cargo run --locked --manifest-path rust/tools/Cargo.toml -- generate-books universal-starter
cargo run --locked --manifest-path rust/tools/Cargo.toml -- generate-books verify books/universal.book
```

Generation can be resumed or split into shards. The v2 format stores sections for different rulesets. See [`books/README.md`](books/README.md) for options and verification details. Large books can take time and memory; use `SAMPLE` to estimate the work first.

## Attribution

ConnectK is an independent implementation. Its search design draws inspiration from published Connect Four solver work by Pascal Pons and from Kite. No solver source code from those projects is included.

## License

ConnectK is licensed under Apache-2.0. See [LICENSE](LICENSE).
