use std::env;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

fn main() -> ExitCode {
    match run() {
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<u8, String> {
    let mut args = env::args().skip(1);
    let Some(command) = args.next() else {
        usage();
        return Ok(0);
    };
    let rest: Vec<String> = args.collect();
    if command == "help" || command == "--help" || command == "-h" {
        usage();
        return Ok(0);
    }
    let root = project_root()?;
    match command.as_str() {
        "build" => build(&root, rest),
        "run-cli" => run_cli(&root, rest),
        "run-gui" => run_gui(&root, rest, false),
        "web-gui" => run_gui(&root, rest, true),
        "generate-books" => generate_books(&root, rest),
        other => Err(format!("unknown command '{other}'\n\n{}", usage_text())),
    }
}

fn usage_text() -> &'static str {
    "Usage: connectk-tools <build|run-cli|run-gui|web-gui|generate-books> [arguments]"
}

fn usage() {
    println!("{}\n\nbuild [test|core|cli|gui|all|clean]\nrun-cli [CLI arguments]\nrun-gui [GUI arguments]\nweb-gui [GUI arguments]\ngenerate-books <standard|standard-shard|standard-segment|custom|universal-starter|merge|verify|info> [arguments]", usage_text());
}

fn project_root() -> Result<PathBuf, String> {
    let cwd = env::current_dir().map_err(|error| error.to_string())?;
    for candidate in cwd.ancestors() {
        if candidate.join("rust/Cargo.toml").is_file()
            && candidate.join("rust/gui/Cargo.toml").is_file()
        {
            return Ok(candidate.to_path_buf());
        }
    }
    Err("run this command from the ConnectK repository or one of its subdirectories".into())
}

fn cargo(root: &Path, args: &[&OsStr]) -> Result<(), String> {
    let status = Command::new("cargo")
        .args(args)
        .current_dir(root)
        .status()
        .map_err(|error| format!("could not start Cargo: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("Cargo exited with {status}"))
    }
}

fn cargo_args<'a>(words: &'a [&'a str]) -> Vec<&'a OsStr> {
    words.iter().map(OsStr::new).collect()
}

fn build(root: &Path, args: Vec<String>) -> Result<u8, String> {
    let target = args.first().map(String::as_str).unwrap_or("cli");
    if args.len() > 1 {
        return Err("build accepts at most one target".into());
    }
    match target {
        "clean" => {
            cargo(
                root,
                &cargo_args(&["clean", "--manifest-path", "rust/Cargo.toml"]),
            )?;
            cargo(
                root,
                &cargo_args(&["clean", "--manifest-path", "rust/gui/Cargo.toml"]),
            )?;
        }
        "test" => {
            cargo(
                root,
                &cargo_args(&["test", "--locked", "--manifest-path", "rust/Cargo.toml"]),
            )?;
            cargo(
                root,
                &cargo_args(&["test", "--locked", "--manifest-path", "rust/gui/Cargo.toml"]),
            )?;
        }
        "core" => cargo(
            root,
            &cargo_args(&[
                "build",
                "--locked",
                "--release",
                "--package",
                "connectk-core",
                "--manifest-path",
                "rust/Cargo.toml",
            ]),
        )?,
        "cli" => cargo(
            root,
            &cargo_args(&[
                "build",
                "--locked",
                "--release",
                "--package",
                "connectk-cli",
                "--manifest-path",
                "rust/Cargo.toml",
            ]),
        )?,
        "gui" => cargo(
            root,
            &cargo_args(&[
                "build",
                "--locked",
                "--release",
                "--manifest-path",
                "rust/gui/Cargo.toml",
            ]),
        )?,
        "all" => {
            cargo(
                root,
                &cargo_args(&[
                    "build",
                    "--locked",
                    "--release",
                    "--manifest-path",
                    "rust/Cargo.toml",
                ]),
            )?;
            cargo(
                root,
                &cargo_args(&[
                    "build",
                    "--locked",
                    "--release",
                    "--manifest-path",
                    "rust/gui/Cargo.toml",
                ]),
            )?;
        }
        _ => return Err(format!("unknown build target '{target}'")),
    }
    Ok(0)
}

fn run_cli(root: &Path, args: Vec<String>) -> Result<u8, String> {
    let binary = root.join("rust/target/release/connectk");
    if !binary.is_file() {
        build(root, vec!["cli".into()])?;
    }
    run_binary(&binary, &args, root)
}

fn run_gui(root: &Path, mut args: Vec<String>, web: bool) -> Result<u8, String> {
    let binary = root.join("rust/gui/target/release/connectk-gui");
    if !binary.is_file() {
        build(root, vec!["gui".into()])?;
    }
    if web
        && !args.iter().any(|arg| {
            ["--native", "--browser", "--web", "--server", "--no-window"].contains(&arg.as_str())
        })
    {
        args.insert(0, "--web".into());
    }
    run_binary(&binary, &args, root)
}

fn run_binary(binary: &Path, args: &[String], root: &Path) -> Result<u8, String> {
    let status = Command::new(binary)
        .args(args)
        .current_dir(root)
        .status()
        .map_err(|error| format!("could not start {}: {error}", binary.display()))?;
    Ok(if status.success() { 0 } else { 1 })
}

fn env_value(name: &str, default: &str) -> String {
    env::var(name).unwrap_or_else(|_| default.to_owned())
}

fn generate_books(root: &Path, args: Vec<String>) -> Result<u8, String> {
    let command = args.first().map(String::as_str).unwrap_or("help");
    if command == "help" || command == "--help" || command == "-h" {
        println!("Generate resumable exact opening books.\n\nUsage: connectk-tools generate-books <standard|standard-shard|standard-segment|custom|universal-starter|merge|verify|info> [arguments]\n\nConfigure generation with environment variables: DEPTH, THREADS, PER_POS_TIME, PER_POS_NODES, FLUSH_EVERY, LIMIT, SAMPLE, SHARD, DRY_RUN, BUILD, OUT, WIDTH, HEIGHT, K, PLACEMENT, DIAGONALS, MISERE, MOVES, FORMAT.");
        return Ok(0);
    }
    let bin = root.join("rust/target/release/connectk");
    if env_value("BUILD", "1") != "0" {
        build(root, vec!["cli".into()])?;
    }
    if !bin.is_file() {
        return Err(format!("CLI binary not found: {}", bin.display()));
    }
    let threads = env_value(
        "THREADS",
        &std::thread::available_parallelism()
            .map(usize::from)
            .unwrap_or(1)
            .to_string(),
    );
    let mut common = vec![
        "--threads".to_owned(),
        threads,
        "--per-pos-time".into(),
        env_value("PER_POS_TIME", "60000"),
        "--per-pos-nodes".into(),
        env_value("PER_POS_NODES", "0"),
        "--flush-every".into(),
        env_value("FLUSH_EVERY", "1"),
    ];
    let shard = env_value("SHARD", "");
    if !shard.is_empty() {
        common.extend(["--shard".into(), shard.clone()]);
    }
    let sample = env_value("SAMPLE", "0");
    if sample != "0" {
        common.extend(["--sample".into(), sample.clone()]);
    }
    let limit = env_value("LIMIT", "0");
    if limit != "0" {
        common.extend(["--limit".into(), limit]);
    }
    if env_value("DRY_RUN", "0") == "1" {
        common.push("--dry-run".into());
    }
    let out = env_value("OUT", "");
    let depth = env_value("DEPTH", "8");
    let moves = env_value("MOVES", "");
    let mut jobs: Vec<Vec<String>> = Vec::new();
    match command {
        "standard" | "standard-shard" => {
            if command == "standard-shard" && shard.is_empty() {
                return Err("set SHARD=I/N, for example SHARD=1/8".into());
            }
            let output = if !out.is_empty() {
                out
            } else if command == "standard" {
                format!("books/std7x6-d{depth}.book")
            } else {
                format!(
                    "books/std7x6-d{depth}-shard-{}.book",
                    shard.replace('/', "of")
                )
            };
            jobs.push(bookgen(
                BookSpec {
                    format: "v1",
                    output: &output,
                    depth: &depth,
                    width: "7",
                    height: "6",
                    k: "4",
                    placement: "gravity",
                    diagonals: "true",
                    misere: "0",
                    moves: &moves,
                },
                &common,
            ));
        }
        "standard-segment" => {
            if moves.is_empty() {
                return Err("set MOVES=4,4,3,3 (the segment root)".into());
            }
            let output = if out.is_empty() {
                "books/universal.book".into()
            } else {
                out
            };
            jobs.push(bookgen(
                BookSpec {
                    format: "v2",
                    output: &output,
                    depth: &depth,
                    width: "7",
                    height: "6",
                    k: "4",
                    placement: "gravity",
                    diagonals: "true",
                    misere: "0",
                    moves: &moves,
                },
                &common,
            ));
        }
        "custom" => {
            let format = env_value("FORMAT", "v2");
            let output = if !out.is_empty() {
                out
            } else if format == "v2" {
                "books/universal.book".into()
            } else {
                "books/custom.book".into()
            };
            let width = env_value("WIDTH", "7");
            let height = env_value("HEIGHT", "6");
            let k = env_value("K", "4");
            let placement = env_value("PLACEMENT", "gravity");
            let diagonals = env_value("DIAGONALS", "true");
            let misere = env_value("MISERE", "0");
            jobs.push(bookgen(
                BookSpec {
                    format: &format,
                    output: &output,
                    depth: &depth,
                    width: &width,
                    height: &height,
                    k: &k,
                    placement: &placement,
                    diagonals: &diagonals,
                    misere: &misere,
                    moves: &moves,
                },
                &common,
            ));
        }
        "universal-starter" => {
            let output = if out.is_empty() {
                "books/universal.book".into()
            } else {
                out
            };
            for (d, w, h, k, placement, diagonals, misere) in [
                (
                    env_value("DEPTH_5X4", "12"),
                    "5",
                    "4",
                    "4",
                    "gravity",
                    "true",
                    "0",
                ),
                (
                    env_value("DEPTH_6X4", "12"),
                    "6",
                    "4",
                    "4",
                    "gravity",
                    "true",
                    "0",
                ),
                (
                    env_value("DEPTH_6X4_NODIAG", "12"),
                    "6",
                    "4",
                    "4",
                    "gravity",
                    "false",
                    "0",
                ),
                ("4".into(), "3", "3", "3", "free", "true", "0"),
                ("4".into(), "4", "4", "4", "gravity", "true", "1"),
            ] {
                jobs.push(bookgen(
                    BookSpec {
                        format: "v2",
                        output: &output,
                        depth: &d,
                        width: w,
                        height: h,
                        k,
                        placement,
                        diagonals,
                        misere,
                        moves: "",
                    },
                    &common,
                ));
            }
        }
        "merge" => {
            let mut inputs = args.iter().skip(1).cloned().collect::<Vec<_>>();
            if inputs.is_empty() {
                if let Ok(entries) = std::fs::read_dir(root.join("books")) {
                    inputs.extend(
                        entries
                            .flatten()
                            .map(|entry| entry.path())
                            .filter(|path| path.extension() == Some(OsStr::new("book")))
                            .map(|path| path.to_string_lossy().into_owned()),
                    );
                    inputs.retain(|path| {
                        root.join(path)
                            != if out.is_empty() {
                                root.join("books/universal.book")
                            } else {
                                root.join(&out)
                            }
                    });
                }
            }
            if inputs.is_empty() {
                return Err("no input books found".into());
            }
            let output = if out.is_empty() {
                "books/universal.book"
            } else {
                &out
            };
            jobs.push(vec![
                "book-merge".into(),
                "--inputs".into(),
                inputs.join(","),
                "--out".into(),
                output.into(),
            ]);
        }
        "verify" => {
            let file = args
                .get(1)
                .map(String::as_str)
                .unwrap_or(if out.is_empty() {
                    "books/universal.book"
                } else {
                    &out
                });
            jobs.push(vec![
                "book-verify".into(),
                "--book".into(),
                file.into(),
                "--sample".into(),
                sample,
                "--per-pos-time".into(),
                env_value("PER_POS_TIME", "60000"),
                "--per-pos-nodes".into(),
                env_value("PER_POS_NODES", "0"),
            ]);
        }
        "info" => {
            let file = args
                .get(1)
                .map(String::as_str)
                .unwrap_or(if out.is_empty() {
                    "books/universal.book"
                } else {
                    &out
                });
            jobs.push(vec!["book-info".into(), "--book".into(), file.into()]);
        }
        _ => return Err(format!("unknown book command '{command}'")),
    }
    for job in jobs {
        let status = Command::new(&bin)
            .args(&job)
            .current_dir(root)
            .status()
            .map_err(|error| error.to_string())?;
        if !status.success() {
            return Ok(status.code().unwrap_or(1) as u8);
        }
    }
    Ok(0)
}

struct BookSpec<'a> {
    format: &'a str,
    output: &'a str,
    depth: &'a str,
    width: &'a str,
    height: &'a str,
    k: &'a str,
    placement: &'a str,
    diagonals: &'a str,
    misere: &'a str,
    moves: &'a str,
}

fn bookgen(spec: BookSpec<'_>, common: &[String]) -> Vec<String> {
    let mut args = vec![
        "bookgen".into(),
        "--format".into(),
        spec.format.into(),
        "--width".into(),
        spec.width.into(),
        "--height".into(),
        spec.height.into(),
        "--k".into(),
        spec.k.into(),
        "--placement".into(),
        spec.placement.into(),
        "--diagonals".into(),
        spec.diagonals.into(),
        "--depth".into(),
        spec.depth.into(),
        "--out".into(),
        spec.output.into(),
    ];
    if spec.misere == "1" || spec.misere == "true" {
        args.push("--misere".into());
    }
    if !spec.moves.is_empty() {
        args.extend(["--moves".into(), spec.moves.into()]);
    }
    args.extend(common.iter().cloned());
    args
}
