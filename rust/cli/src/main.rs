use connectk_core::book::{parse_rules_line, rules_line, BookEntry, OpeningBook};
use connectk_core::hash::BuildFast;
use connectk_core::model::{dirs, Move, Placement, Rules};
use connectk_core::position::{Position, PositionKey};
use connectk_core::solver::facade::ConnectKSolver;
use connectk_core::solver::fast::FastExactSolver;
use connectk_core::solver::generic::ExactSolver;
use connectk_core::solver::standard::StandardSolver;
use connectk_core::solver::{Certainty, SearchControl, SolveMode, SolveRequest};
use std::collections::{HashMap, HashSet};
use std::io::{BufWriter, Write};
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

type Opts = HashMap<String, String>;

fn main() {
    let argv: Vec<String> = std::env::args().collect();
    if argv.len() < 2 {
        print_help();
        return;
    }
    let command = argv[1].as_str();
    if command == "gui" {
        if let Err(error) = launch_gui(&argv[2..]) {
            fail(&error);
        }
        return;
    }
    let opts = match parse_opts(&argv) {
        Ok(o) => o,
        Err(e) => fail(&e),
    };
    let result = match command {
        "solve" => cmd_solve(&opts),
        "selfplay" => cmd_selfplay(&opts),
        "theory" => cmd_theory(&opts),
        "bookgen" => cmd_bookgen(&opts),
        "book-info" => cmd_book_info(&opts),
        "book-merge" => cmd_book_merge(&opts),
        "book-verify" => cmd_book_verify(&opts),
        "help" | "--help" | "-h" => {
            print_help();
            Ok(())
        }
        other => {
            eprintln!("unknown command: {other}");
            print_help();
            std::process::exit(2);
        }
    };
    if let Err(e) = result {
        fail(&e);
    }
}

fn launch_gui(args: &[String]) -> Result<(), String> {
    let cwd = std::env::current_dir().map_err(|error| error.to_string())?;
    if let Some(root) = cwd
        .ancestors()
        .find(|path| path.join("rust/gui/Cargo.toml").is_file())
    {
        let manifest = root.join("rust/gui/Cargo.toml");
        let binary = root.join("rust/gui/target/release/connectk-gui");
        let status = std::process::Command::new("cargo")
            .args(["build", "--locked", "--release", "--manifest-path"])
            .arg(&manifest)
            .current_dir(root)
            .status()
            .map_err(|error| format!("could not start Cargo to build the GUI: {error}"))?;
        if !status.success() {
            return Err(format!("GUI build failed with {status}"));
        }
        let status = std::process::Command::new(binary)
            .args(args)
            .current_dir(root)
            .status()
            .map_err(|error| format!("could not start the GUI: {error}"))?;
        return if status.success() {
            Ok(())
        } else {
            Err(format!("GUI exited with {status}"))
        };
    }
    let gui = std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .map(|directory| directory.join("connectk-gui"))
        .find(|path| path.is_file())
        .ok_or_else(|| {
            "GUI not found; install connectk-gui or run `connectk gui` from the project repository"
                .to_owned()
        })?;
    let status = std::process::Command::new(&gui)
        .args(args)
        .status()
        .map_err(|error| format!("could not start {}: {error}", gui.display()))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("GUI exited with {status}"))
    }
}

fn fail(msg: &str) -> ! {
    eprintln!("error: {msg}");
    std::process::exit(2);
}

fn print_help() {
    println!(
        r#"ConnectK solver CLI

Usage:
  gui      [--native|--browser|--web|--server] (build dependencies, then launch)
  solve    [rule options] [--moves ...] [--mode auto|exact|depth]
           [--depth N] [--time MS] [--nodes N] [--exact-time MS] [--no-theory]
  selfplay [rule options] [--depth N] [--move-time MS]
  theory   [--width W --height H]
  bookgen  [rule options] [--moves ...] --depth N --out FILE [--threads T]
           [--per-pos-time MS] [--limit N] [--format v1|v2] [--shard I/N]
           [--sample N] [--flush-every N] [--dry-run]
           Exact opening or segment book generation is resumable. --moves roots
           the book at a position; --depth counts additional plies. v1 is for
           one-word gravity; v2 supports universal sections for dense rules.
  book-info   --book FILE
  book-merge  --inputs a.book,b.book --out universal.book
  book-verify --book FILE [--sample N] [--per-pos-time MS] [--per-pos-nodes N]

Rule options:
  --width W (default 7)   --height H (default 6)   --k K (default 4)
  --placement gravity|free (default gravity)
  --diagonals true|false (default true)
  --directions H,V,DU,DD (overrides --diagonals)
  --misere (making a line loses)

Moves are 1-based. Gravity: --moves 4,4,5
Free placement: --moves "5:5;6:5" (column:row, row 1 = bottom)

Modes:
  auto   theory where available, exact proof when tractable, otherwise
         depth-limited search (labelled heuristic)
  exact  proof or UNKNOWN (budget: --time / --nodes)
  depth  depth-limited heuristic search only

Exact-solving speedups (solve/selfplay):
  --threads N  parallel exact proving on N threads (default 1)
  --book FILE  v1/v2 book; matching discovered books are merged automatically
               from ./books, parent directories, executable paths, or the
               bundled asset. --no-book disables book loading."#
    );
}

fn parse_opts(argv: &[String]) -> Result<Opts, String> {
    let mut out = Opts::new();
    let mut i = 2;
    while i < argv.len() {
        let a = &argv[i];
        let Some(key) = a.strip_prefix("--") else {
            return Err(format!("unexpected argument: {a}"));
        };
        if let Some(eq) = key.find('=') {
            out.insert(key[..eq].to_string(), key[eq + 1..].to_string());
        } else if key == "misere" || key == "no-theory" || key == "dry-run" || key == "no-book" {
            out.insert(key.to_string(), "true".to_string());
        } else {
            i += 1;
            let Some(v) = argv.get(i) else {
                return Err(format!("missing value for --{key}"));
            };
            out.insert(key.to_string(), v.clone());
        }
        i += 1;
    }
    Ok(out)
}

fn int_opt(o: &Opts, key: &str, def: i32) -> Result<i32, String> {
    match o.get(key) {
        None => Ok(def),
        Some(v) => v
            .trim()
            .parse()
            .map_err(|_| format!("--{key} must be an integer, got '{v}'")),
    }
}

fn long_opt(o: &Opts, key: &str, def: i64) -> Result<i64, String> {
    match o.get(key) {
        None => Ok(def),
        Some(v) => v
            .trim()
            .parse()
            .map_err(|_| format!("--{key} must be an integer, got '{v}'")),
    }
}

struct RuleParams {
    width: i32,
    height: i32,
    connect: i32,
    placement: Placement,
    directions: u8,
    misere: bool,
}

fn parse_rule_params(o: &Opts) -> Result<RuleParams, String> {
    let w = int_opt(o, "width", 7)?;
    let h = int_opt(o, "height", 6)?;
    let k = int_opt(o, "k", 4)?;
    let placement = match o.get("placement").map(|s| s.to_lowercase()).as_deref() {
        None | Some("gravity") => Placement::Gravity,
        Some("free") => Placement::Free,
        _ => return Err("--placement must be gravity|free".into()),
    };
    let directions = if let Some(list) = o.get("directions") {
        let mut d = 0u8;
        for part in list.split(',') {
            match part.trim().to_uppercase().as_str() {
                "H" => d |= dirs::HORIZONTAL,
                "V" => d |= dirs::VERTICAL,
                "DU" => d |= dirs::DIAGONAL_UP,
                "DD" => d |= dirs::DIAGONAL_DOWN,
                other => {
                    return Err(format!("unknown direction '{other}' (use H,V,DU,DD)"));
                }
            }
        }
        d
    } else {
        let diag = !matches!(
            o.get("diagonals").map(|s| s.to_lowercase()).as_deref(),
            Some("false")
        );
        if diag {
            dirs::ALL
        } else {
            dirs::ORTHOGONAL
        }
    };
    Ok(RuleParams {
        width: w,
        height: h,
        connect: k,
        placement,
        directions,
        misere: o.contains_key("misere"),
    })
}

fn build_rules(o: &Opts) -> Result<Rules, String> {
    let p = parse_rule_params(o)?;
    Rules::new(
        p.width,
        p.height,
        p.connect,
        p.placement,
        p.directions,
        p.misere,
    )
}

fn has_moves(o: &Opts) -> bool {
    o.get("moves")
        .map(|m| !m.trim().is_empty())
        .unwrap_or(false)
}

fn symbolic_empty_theory(
    o: &Opts,
) -> Result<Option<(RuleParams, connectk_core::theory::SymbolicTheoryResult)>, String> {
    if has_moves(o) {
        return Ok(None);
    }
    let p = parse_rule_params(o)?;
    let result = connectk_core::theory::lookup_empty_symbolic(
        p.width,
        p.height,
        p.connect,
        p.placement,
        p.directions,
        p.misere,
    );
    Ok(result.map(|r| (p, r)))
}

fn print_symbolic_theory(p: &RuleParams, r: &connectk_core::theory::SymbolicTheoryResult) {
    println!(
        "Rules: Connect-{} {}x{} {} directions={}{} (symbolic; dense board not allocated)",
        p.connect,
        p.width,
        p.height,
        p.placement,
        p.directions,
        if p.misere { " misere" } else { "" }
    );
    println!("Side to move: ONE (X) after 0 plies");
    print!(
        "PROVEN (symbolic theory) | outcome for side to move: {}",
        r.outcome
    );
    if let Some(m) = r.proven_best_move {
        print!(" | best move: {} (proven optimal)", m.notation());
    }
    if let Some(plies) = r.plies_to_end {
        print!(" | ends in {plies} plies");
    }
    println!();
    println!("  source: {}", r.source);
}

fn build_position(rules: &Rules, o: &Opts) -> Result<Position, String> {
    let mut p = Position::initial(rules);
    let Some(moves) = o.get("moves") else {
        return Ok(p);
    };
    if moves.trim().is_empty() {
        return Ok(p);
    }
    if rules.placement == Placement::Gravity {
        for part in moves.split(',') {
            let col: i32 = part
                .trim()
                .parse()
                .map_err(|_| format!("bad column '{part}'"))?;
            p.play_column(col - 1)?;
        }
    } else {
        for part in moves.split(';') {
            let mut it = part.trim().split(':');
            let (Some(c), Some(r)) = (it.next(), it.next()) else {
                return Err(format!("free move must be column:row, got '{part}'"));
            };
            let col: i32 = c.trim().parse().map_err(|_| format!("bad move '{part}'"))?;
            let row: i32 = r.trim().parse().map_err(|_| format!("bad move '{part}'"))?;
            p.play(Move::cell(col - 1, row - 1))?;
        }
    }
    Ok(p)
}

fn build_request(o: &Opts) -> Result<SolveRequest, String> {
    let mode = match o.get("mode").map(|s| s.to_lowercase()).as_deref() {
        None | Some("auto") => SolveMode::Auto,
        Some("exact") => SolveMode::Exact,
        Some("depth") | Some("depth-limited") | Some("heuristic") => SolveMode::DepthLimited,
        _ => return Err("--mode must be auto|exact|depth".into()),
    };
    let req = SolveRequest {
        mode,
        max_depth: int_opt(o, "depth", 10)?,
        time_limit_millis: long_opt(o, "time", 5000)? as u64,
        node_limit: long_opt(o, "nodes", 20_000_000)? as u64,
        exact_time_limit_millis: long_opt(o, "exact-time", 1500)? as u64,
        exact_node_limit: long_opt(o, "exact-nodes", long_opt(o, "nodes", 5_000_000)?)? as u64,
        use_theory: !o.contains_key("no-theory"),
    };
    req.validate()?;
    Ok(req)
}

fn print_book(book: &OpeningBook, source: &str, auto: bool) {
    println!(
        "Opening book{}: {} entries{} from {}",
        if auto { " (auto)" } else { "" },
        book.size(),
        if book.depth() >= 0 {
            format!(" at depth {}", book.depth())
        } else {
            String::new()
        },
        source
    );
}

fn discover_book(rules: &Rules) -> Option<Arc<OpeningBook>> {
    let (paths, book) = OpeningBook::discover_all_for_rules(rules)?;
    let source = paths
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join(", ");
    print_book(&book, &source, true);
    Some(Arc::new(book))
}

fn make_solver(o: &Opts, rules: &Rules) -> Result<ConnectKSolver, String> {
    let threads = int_opt(o, "threads", 1)?.max(1) as usize;
    let book = if o.contains_key("no-book") {
        None
    } else {
        match o.get("book") {
            Some(file) => {
                let book = OpeningBook::load_for_rules(std::path::Path::new(file), rules)
                    .map_err(|e| format!("cannot load opening book '{file}': {e}"))?
                    .ok_or_else(|| format!("opening book '{file}' has no section for {rules}"))?;
                print_book(&book, file, false);
                Some(Arc::new(book))
            }
            None => discover_book(rules),
        }
    };
    Ok(ConnectKSolver::with_book_and_threads(book, threads))
}

fn cmd_solve(o: &Opts) -> Result<(), String> {
    let request = build_request(o)?;
    let rules = match build_rules(o) {
        Ok(rules) => rules,
        Err(error) => {
            if request.use_theory && request.mode != SolveMode::DepthLimited {
                if let Some((params, result)) = symbolic_empty_theory(o)? {
                    print_symbolic_theory(&params, &result);
                    return Ok(());
                }
            }
            return Err(error);
        }
    };
    let p = build_position(&rules, o)?;
    println!("Rules: {rules}");
    println!("{p}");
    println!(
        "Side to move: {} ({}) after {} plies",
        p.side_to_move(),
        p.side_to_move().symbol(),
        p.moves_played()
    );
    let result = make_solver(o, &rules)?.solve(&p, &request);
    println!("{result}");
    Ok(())
}

fn cmd_selfplay(o: &Opts) -> Result<(), String> {
    let rules = build_rules(o)?;
    let mut p = Position::initial(&rules);
    let depth = int_opt(o, "depth", 8)?;
    let move_time = long_opt(o, "move-time", 300)?;
    let solver = make_solver(o, &rules)?;
    let mut played: Vec<String> = Vec::new();
    println!("Rules: {rules}  (both sides: depth {depth}, {move_time} ms/move)");
    while !p.is_terminal() {
        let req = SolveRequest {
            mode: SolveMode::Auto,
            max_depth: depth,
            time_limit_millis: move_time as u64,
            node_limit: 0,
            exact_time_limit_millis: 500.min(move_time) as u64,
            exact_node_limit: 2_000_000,
            use_theory: true,
        };
        let r = solver.solve(&p, &req);
        let Some(mv) = r.best_move else {
            println!("No move available ({}); stopping.", r.certainty);
            break;
        };
        let mover = p.side_to_move();
        let played_move = p.play(mv)?;
        played.push(played_move.notation());
        println!(
            "{:3}. {} plays {:<6} [{}, score {}, depth {}, {} nodes]",
            p.moves_played(),
            mover,
            played_move.notation(),
            r.certainty,
            r.score,
            r.depth_reached,
            r.nodes
        );
    }
    println!("{p}");
    println!("Moves: {}", played.join(" "));
    match p.winner() {
        Some(w) => println!("Result: {} ({}) wins", w, w.symbol()),
        None => println!(
            "Result: {}",
            if p.is_draw() { "draw" } else { "unfinished" }
        ),
    }
    Ok(())
}

fn cmd_theory(o: &Opts) -> Result<(), String> {
    let rules = match build_rules(o) {
        Ok(rules) => rules,
        Err(error) => {
            if let Some((params, result)) = symbolic_empty_theory(o)? {
                print_symbolic_theory(&params, &result);
                return Ok(());
            }
            return Err(error);
        }
    };
    let p = build_position(&rules, o)?;
    match connectk_core::theory::lookup(&p) {
        Some(t) => {
            println!(
                "Mathematical result for {rules} after {} plies:",
                p.moves_played()
            );
            println!("  outcome for side to move: {}", t.outcome);
            if let Some(pl) = t.plies_to_end {
                println!("  game ends in {pl} plies under perfect play");
            }
            if let Some(m) = t.proven_best_move {
                println!("  proven optimal move: {}", m.notation());
            }
            println!("  source: {}", t.source);
        }
        None => println!(
            "No mathematical result in the theory database for: {rules} after {} plies.",
            p.moves_played()
        ),
    }
    Ok(())
}

fn truncate_incomplete_last_line(path: &std::path::Path) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    if bytes.is_empty() || bytes.last() == Some(&b'\n') {
        return Ok(());
    }
    let keep = bytes
        .iter()
        .rposition(|b| *b == b'\n')
        .map(|i| i + 1)
        .unwrap_or(0);
    let file = std::fs::OpenOptions::new()
        .write(true)
        .open(path)
        .map_err(|e| format!("cannot open {}: {e}", path.display()))?;
    file.set_len(keep as u64)
        .map_err(|e| format!("cannot truncate {}: {e}", path.display()))?;
    eprintln!(
        "warning: discarded an incomplete final line from {} before resuming",
        path.display()
    );
    Ok(())
}

fn parse_shard(o: &Opts) -> Result<(usize, usize), String> {
    let Some(v) = o.get("shard") else {
        return Ok((0, 1));
    };
    let Some((i, n)) = v.split_once('/') else {
        return Err("--shard must be I/N, for example --shard 1/8".into());
    };
    let i: usize = i
        .trim()
        .parse()
        .map_err(|_| "--shard must be I/N with integer parts".to_string())?;
    let n: usize = n
        .trim()
        .parse()
        .map_err(|_| "--shard must be I/N with integer parts".to_string())?;
    if n == 0 || i == 0 || i > n {
        return Err("--shard requires 1 <= I <= N".into());
    }
    Ok((i - 1, n))
}

fn select_sample(todo: Vec<Position>, sample: usize) -> Vec<Position> {
    if sample == 0 || sample >= todo.len() {
        return todo;
    }
    let total = todo.len();
    let mut out = Vec::with_capacity(sample);
    let mut used = std::collections::HashSet::new();
    for j in 0..sample {
        let idx = ((j as u128 * total as u128) / sample as u128) as usize;
        if used.insert(idx) {
            out.push(todo[idx].clone());
        }
    }
    out
}

struct ExactWorker {
    standard: StandardSolver,
    fast: FastExactSolver,
    generic: ExactSolver,
    use_standard: bool,
    use_fast: bool,
}

impl ExactWorker {
    fn new(rules: &Rules) -> ExactWorker {
        let mut standard = StandardSolver::new(0);
        standard.set_persistent_tt(true);
        ExactWorker {
            standard,
            fast: FastExactSolver::new(),
            generic: ExactSolver::new(),
            use_standard: StandardSolver::supports(rules),
            use_fast: FastExactSolver::supports(rules),
        }
    }

    fn solve(
        &mut self,
        pos: &Position,
        nodes: u64,
        millis: u64,
    ) -> connectk_core::solver::SearchResult {
        if self.use_standard {
            self.standard.solve(pos, SearchControl::new(nodes, millis))
        } else if self.use_fast {
            self.fast.solve(pos, SearchControl::new(nodes, millis))
        } else {
            self.generic.solve(pos, SearchControl::new(nodes, millis))
        }
    }
}

fn words_hex_cli(words: &[u64]) -> String {
    words
        .iter()
        .map(|w| format!("{w:016x}"))
        .collect::<Vec<_>>()
        .join(",")
}

fn book_entry_line_v2(rules: &Rules, key: &PositionKey, entry: BookEntry) -> String {
    if rules.word_count() == 1 && rules.placement == Placement::Gravity {
        format!(
            "{:016x} {:016x} {} {}",
            key.current[0],
            key.mask[0],
            entry.score,
            entry.best_move.map(|m| m.column).unwrap_or(-1)
        )
    } else {
        let best = entry
            .best_move
            .map(|m| format!("{},{}", m.column, m.row))
            .unwrap_or_else(|| "-".to_string());
        format!(
            "cur={} mask={} score={} best={}",
            words_hex_cli(&key.current),
            words_hex_cli(&key.mask),
            entry.score,
            best
        )
    }
}

fn infer_rules_from_book_filename(path: &std::path::Path) -> Option<Rules> {
    let name = path.file_name()?.to_string_lossy().into_owned();
    let stem = name.strip_suffix(".book")?;
    let (diagonals, rest) = if let Some(rest) = stem.strip_prefix("std") {
        (true, rest)
    } else {
        (false, stem.strip_prefix("nodiag")?)
    };
    fn digits(text: &str) -> Option<(&str, &str)> {
        let end = text.find(|c: char| !c.is_ascii_digit())?;
        Some((&text[..end], &text[end..]))
    }
    let (w, rest) = digits(rest)?;
    let rest = rest.strip_prefix('x')?;
    let (h, rest) = digits(rest)?;
    let (k, _rest) = if let Some(rest) = rest.strip_prefix("-k") {
        let (k, rest) = digits(rest)?;
        (k.parse().ok()?, rest)
    } else {
        (4, rest)
    };
    Rules::new(
        w.parse().ok()?,
        h.parse().ok()?,
        k,
        Placement::Gravity,
        if diagonals {
            dirs::ALL
        } else {
            dirs::ORTHOGONAL
        },
        false,
    )
    .ok()
}

fn section_rules_or_infer(
    book: OpeningBook,
    path: &std::path::Path,
) -> Result<OpeningBook, String> {
    if book.rules().is_some() {
        return Ok(book);
    }
    let rules = infer_rules_from_book_filename(path).ok_or_else(|| {
        format!(
            "book {} has no rules metadata and its filename does not encode std/nodiag rules",
            path.display()
        )
    })?;
    Ok(book.with_rules(rules))
}

type BookKey = (u64, u64);
type KeySet = HashSet<BookKey, BuildFast>;
type GeneralKeySet = HashSet<PositionKey, BuildFast>;

fn key_set_with_capacity(cap: usize) -> KeySet {
    HashSet::with_capacity_and_hasher(cap, BuildFast::default())
}

fn cmd_book_info(o: &Opts) -> Result<(), String> {
    let Some(file) = o.get("book") else {
        return Err("--book FILE is required for book-info".into());
    };
    let path = std::path::Path::new(file);
    let sections = OpeningBook::load_sections(path)?;
    let bytes = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    println!(
        "Book: {file} ({bytes} bytes, {} section(s))",
        sections.len()
    );
    let mut total = 0usize;
    for (i, section) in sections.iter().enumerate() {
        total += section.size();
        let rules = section
            .rules()
            .map(rules_line)
            .unwrap_or_else(|| "legacy section without rules metadata".to_string());
        println!(
            "  [{}] {} | depth={} | {} entries | plies {}..={}",
            i + 1,
            rules,
            section.depth(),
            section.size(),
            section.min_ply(),
            section.max_ply()
        );
    }
    println!("Total: {total} exact entries");
    Ok(())
}

fn cmd_book_merge(o: &Opts) -> Result<(), String> {
    let Some(inputs) = o.get("inputs").or_else(|| o.get("book")) else {
        return Err("--inputs a.book,b.book is required for book-merge".into());
    };
    let Some(out) = o.get("out") else {
        return Err("--out FILE is required for book-merge".into());
    };
    let mut by_rules: std::collections::BTreeMap<String, Vec<OpeningBook>> =
        std::collections::BTreeMap::new();
    for part in inputs
        .split(',')
        .map(|x| x.trim())
        .filter(|x| !x.is_empty())
    {
        let path = std::path::Path::new(part);
        for section in OpeningBook::load_sections(path)? {
            let section = section_rules_or_infer(section, path)?;
            let key = rules_line(section.rules().expect("rules checked above"));
            by_rules.entry(key).or_default().push(section);
        }
    }
    if by_rules.is_empty() {
        return Err("no book sections found to merge".into());
    }
    let mut merged = Vec::new();
    let mut total = 0usize;
    for (rules, sections) in by_rules {
        let book = OpeningBook::merged(sections)?;
        println!(
            "  {rules}: {} entries (depth {})",
            book.size(),
            book.depth()
        );
        total += book.size();
        merged.push(book);
    }
    OpeningBook::save_universal(std::path::Path::new(out), &merged)?;
    println!("Merged {total} exact entries into {out}");
    Ok(())
}

fn cmd_book_verify(o: &Opts) -> Result<(), String> {
    let Some(file) = o.get("book") else {
        return Err("--book FILE is required for book-verify".into());
    };
    let path = std::path::Path::new(file);
    let sample = int_opt(o, "sample", 16)?.max(0) as usize;
    let per_pos_time = long_opt(o, "per-pos-time", 60_000)? as u64;
    let per_pos_nodes = long_opt(o, "per-pos-nodes", 0)? as u64;

    let mut verified = 0usize;
    let mut unverified = 0usize;
    let mut failures = 0usize;

    for section in OpeningBook::load_sections(path)? {
        let section = section_rules_or_infer(section, path)?;
        let rules = section.rules().expect("rules checked above").clone();
        let entries = section.entries();
        let indices: Vec<usize> = if sample == 0 || sample >= entries.len() {
            (0..entries.len()).collect()
        } else {
            (0..sample)
                .map(|j| ((j as u128 * entries.len() as u128) / sample as u128) as usize)
                .collect()
        };
        println!(
            "Verifying {} of {} entries for {} ({} ms/position)...",
            indices.len(),
            entries.len(),
            rules_line(&rules),
            per_pos_time
        );
        let mut worker = ExactWorker::new(&rules);
        for idx in indices {
            let (key, entry) = &entries[idx];
            let pos = match Position::from_words(&rules, &key.current, &key.mask) {
                Ok(pos) => pos,
                Err(e) => {
                    println!("  FAIL entry {idx}: cannot reconstruct position: {e}\n{key:?}");
                    failures += 1;
                    continue;
                }
            };
            let result = worker.solve(&pos, per_pos_nodes, per_pos_time);
            if result.certainty != Certainty::ProvenExact {
                unverified += 1;
                continue;
            }
            if result.score != entry.score {
                println!(
                    "  FAIL entry {idx}: book score {}, independent exact score {}\n{pos}",
                    entry.score, result.score
                );
                failures += 1;
                continue;
            }
            if let Some(best) = entry.best_move {
                let mut child = pos.clone();
                if let Err(e) = child.play(best) {
                    println!(
                        "  FAIL entry {idx}: stored best move {} is illegal: {e}\n{pos}",
                        best.notation()
                    );
                    failures += 1;
                    continue;
                }
                let child_result = worker.solve(&child, per_pos_nodes, per_pos_time);
                if child_result.certainty != Certainty::ProvenExact {
                    unverified += 1;
                    continue;
                }
                if entry.score != -child_result.score {
                    println!(
                        "  FAIL entry {idx}: stored best move {} achieves {}, book claims {}\n{pos}",
                        best.notation(),
                        -child_result.score,
                        entry.score
                    );
                    failures += 1;
                    continue;
                }
            }
            verified += 1;
        }
    }

    println!(
        "book-verify finished: {verified} verified, {unverified} not independently proven within budget, {failures} failure(s)."
    );
    if failures > 0 {
        Err(format!("book verification found {failures} failure(s)"))
    } else {
        Ok(())
    }
}

fn cmd_bookgen(o: &Opts) -> Result<(), String> {
    let format = o
        .get("format")
        .map(|v| v.to_ascii_lowercase())
        .unwrap_or_else(|| "v1".to_string());
    if format == "v2" || format == "universal" {
        return cmd_bookgen_v2(o);
    }
    if format != "v1" {
        return Err("--format must be v1 or v2".into());
    }
    let rules = build_rules(o)?;
    if !FastExactSolver::supports(&rules) {
        return Err(format!(
            "bookgen requires fast-path rules (gravity, one-word bitboard, not misere): {rules}"
        ));
    }
    let depth = int_opt(o, "depth", -1)?;
    if depth < 0 {
        return Err("--depth N is required for bookgen".into());
    }

    let mut root = build_position(&rules, o)?;
    if root.is_terminal() {
        return Err("bookgen root position is already terminal".into());
    }
    let root_ply = root.moves_played() as i32;
    let target_depth = root_ply + depth;
    let Some(out) = o.get("out") else {
        return Err("--out FILE is required for bookgen".into());
    };
    let threads = (int_opt(o, "threads", 1)?).max(1) as usize;
    let per_pos_time = long_opt(o, "per-pos-time", 60_000)? as u64;
    let per_pos_nodes = long_opt(o, "per-pos-nodes", 0)? as u64;
    let limit = int_opt(o, "limit", i32::MAX)? as usize;
    let (shard_index, shard_count) = parse_shard(o)?;
    let sample = int_opt(o, "sample", 0)?.max(0) as usize;
    let flush_every = (int_opt(o, "flush-every", 1)?).max(1) as usize;
    let dry_run = o.contains_key("dry-run");
    let path = std::path::PathBuf::from(out);

    let mut done: KeySet = key_set_with_capacity(1 << 16);
    truncate_incomplete_last_line(&path)?;
    let exists = path.exists()
        && std::fs::metadata(&path)
            .map(|m| m.len() > 0)
            .unwrap_or(false);
    if exists {
        let text = std::fs::read_to_string(&path).map_err(|e| format!("cannot read {out}: {e}"))?;
        for line in text.lines() {
            let line = line.trim();
            if let Some(rest) = line.strip_prefix('#') {
                if let Some(i) = rest.find("rules:") {
                    let file_rules = parse_rules_line(&rest[i + 6..])?;
                    if file_rules != rules {
                        return Err(format!(
                            "existing book {out} is for {}, requested {}",
                            rules_line(&file_rules),
                            rules_line(&rules)
                        ));
                    }
                }
                if let Some(i) = rest.find("depth=") {
                    let tail = rest[i + 6..].trim();
                    let num: String = tail
                        .chars()
                        .take_while(|c| c.is_ascii_digit() || *c == '-')
                        .collect();
                    if let Ok(file_depth) = num.parse::<i32>() {
                        if file_depth != target_depth {
                            return Err(format!(
                                "existing book {out} has depth {file_depth}, requested {target_depth}"
                            ));
                        }
                    }
                }
                continue;
            }
            if line.is_empty() {
                continue;
            }
            let mut parts = line.split_whitespace();
            if let (Some(c), Some(m)) = (parts.next(), parts.next()) {
                if let (Ok(cur), Ok(mask)) =
                    (u64::from_str_radix(c, 16), u64::from_str_radix(m, 16))
                {
                    done.insert((cur, mask));
                }
            }
        }
        println!("Resuming: {} entries already in {out}", done.len());
    }

    let t0 = Instant::now();
    let mut todo: Vec<Position> = Vec::new();
    let mut visited: KeySet = key_set_with_capacity(1 << 18);
    let mut frontier_seen = 0usize;
    enumerate(
        &mut root,
        depth,
        &mut visited,
        &done,
        &mut todo,
        limit,
        shard_index,
        shard_count,
        &mut frontier_seen,
    );
    let enumerated = todo.len();
    if sample > 0 {
        todo = select_sample(todo, sample);
    }
    println!(
        "Enumerated {} new canonical positions at ply {} ({} plies beyond the root at ply {}; frontier seen {}, shard {}/{}, selected {}) in {:.1} s; solving with {} thread(s), budget {} ms/position...",
        enumerated,
        target_depth,
        depth,
        root_ply,
        frontier_seen,
        shard_index + 1,
        shard_count,
        todo.len(),
        t0.elapsed().as_secs_f64(),
        threads,
        per_pos_time
    );
    if dry_run {
        println!("Dry run: no positions solved and no book written.");
        return Ok(());
    }

    let total = todo.len();
    let solved = AtomicI64::new(0);
    let unsolved = AtomicI64::new(0);
    let total_nodes = AtomicU64::new(0);

    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| format!("cannot open {out}: {e}"))?;
    let mut writer = BufWriter::new(file);
    if !exists {
        writeln!(writer, "# connectk-book v1 depth={target_depth}").map_err(|e| e.to_string())?;
        writeln!(writer, "# rules: {}", rules_line(&rules)).map_err(|e| e.to_string())?;
        if root_ply > 0 {
            let history = root
                .move_history()
                .iter()
                .map(|m| m.notation())
                .collect::<Vec<_>>()
                .join(" ");
            writeln!(writer, "# segment root ply {root_ply}: {history}")
                .map_err(|e| e.to_string())?;
        }
        writer.flush().map_err(|e| e.to_string())?;
    }

    let width = rules.width;

    let (job_tx, job_rx) = std::sync::mpsc::sync_channel::<Position>(threads * 2);
    let (res_tx, res_rx) = std::sync::mpsc::sync_channel::<String>(threads * 4);
    let job_rx = Arc::new(std::sync::Mutex::new(job_rx));

    std::thread::scope(|scope| -> Result<(), String> {
        let mut workers = Vec::new();
        for _ in 0..threads {
            let job_rx = job_rx.clone();
            let res_tx = res_tx.clone();
            let solved = &solved;
            let unsolved = &unsolved;
            let total_nodes = &total_nodes;
            let use_standard = StandardSolver::supports(&rules);
            workers.push(scope.spawn(move || {
                let mut fast_solver = FastExactSolver::new();
                let mut standard_solver = StandardSolver::new(0);
                standard_solver.set_persistent_tt(true);
                loop {
                    let pos = {
                        let rx = job_rx.lock().unwrap();
                        rx.recv()
                    };
                    let Ok(pos) = pos else { break };
                    let r = if use_standard {
                        standard_solver.solve(&pos, SearchControl::new(per_pos_nodes, per_pos_time))
                    } else {
                        fast_solver.solve(&pos, SearchControl::new(per_pos_nodes, per_pos_time))
                    };
                    total_nodes.fetch_add(r.nodes, Ordering::Relaxed);
                    if r.certainty != Certainty::ProvenExact {
                        unsolved.fetch_add(1, Ordering::Relaxed);
                        continue;
                    }
                    solved.fetch_add(1, Ordering::Relaxed);
                    let (cur, mask, flipped) = FastExactSolver::encode_canonical(&pos);
                    let best_col = r.best_move.map(|m| m.column).unwrap_or(-1);
                    let canonical_col = if best_col < 0 {
                        -1
                    } else if flipped {
                        width - 1 - best_col
                    } else {
                        best_col
                    };
                    let line = format!("{cur:016x} {mask:016x} {} {}", r.score, canonical_col);
                    if res_tx.send(line).is_err() {
                        break;
                    }
                }
            }));
        }
        drop(res_tx);
        let feeder = scope.spawn(move || {
            for pos in todo {
                if job_tx.send(pos).is_err() {
                    break;
                }
            }
        });

        let mut received = 0usize;
        while let Ok(line) = res_rx.recv() {
            if !line.is_empty() {
                writeln!(writer, "{line}").map_err(|e| e.to_string())?;
                if received.is_multiple_of(flush_every) {
                    writer.flush().map_err(|e| e.to_string())?;
                }
            }
            received += 1;
            let n = received;
            let done_count =
                (solved.load(Ordering::Relaxed) + unsolved.load(Ordering::Relaxed)) as usize;
            if n.is_multiple_of(200) || done_count == total {
                println!(
                    "  {}/{} done ({} solved, {} unsolved, {} nodes, {:.0} s elapsed)",
                    done_count.max(n),
                    total,
                    solved.load(Ordering::Relaxed),
                    unsolved.load(Ordering::Relaxed),
                    total_nodes.load(Ordering::Relaxed),
                    t0.elapsed().as_secs_f64()
                );
            }
        }
        let _ = feeder.join();
        for w in workers {
            let _ = w.join();
        }
        Ok(())
    })?;
    writer.flush().map_err(|e| e.to_string())?;

    println!(
        "bookgen finished: {} solved, {} unsolved (retry by re-running), {} nodes total, {:.0} s.",
        solved.load(Ordering::Relaxed),
        unsolved.load(Ordering::Relaxed),
        total_nodes.load(Ordering::Relaxed),
        t0.elapsed().as_secs_f64()
    );
    println!(
        "Book file: {}",
        path.canonicalize().unwrap_or(path).display()
    );
    Ok(())
}

fn cmd_bookgen_v2(o: &Opts) -> Result<(), String> {
    let rules = build_rules(o)?;
    let depth = int_opt(o, "depth", -1)?;
    if depth < 0 {
        return Err("--depth N is required for bookgen".into());
    }
    let mut root = build_position(&rules, o)?;
    if root.is_terminal() {
        return Err("bookgen root position is already terminal".into());
    }
    let root_ply = root.moves_played() as i32;
    let target_depth = root_ply + depth;
    let Some(out) = o.get("out") else {
        return Err("--out FILE is required for bookgen".into());
    };
    let threads = (int_opt(o, "threads", 1)?).max(1) as usize;
    let per_pos_time = long_opt(o, "per-pos-time", 60_000)? as u64;
    let per_pos_nodes = long_opt(o, "per-pos-nodes", 0)? as u64;
    let limit = int_opt(o, "limit", i32::MAX)? as usize;
    let (shard_index, shard_count) = parse_shard(o)?;
    let sample = int_opt(o, "sample", 0)?.max(0) as usize;
    let flush_every = (int_opt(o, "flush-every", 1)?).max(1) as usize;
    let dry_run = o.contains_key("dry-run");
    let path = std::path::PathBuf::from(out);

    truncate_incomplete_last_line(&path)?;
    let exists = path.exists()
        && std::fs::metadata(&path)
            .map(|m| m.len() > 0)
            .unwrap_or(false);
    let mut done: GeneralKeySet = HashSet::with_capacity_and_hasher(1 << 16, BuildFast::default());
    if exists {
        let sections = OpeningBook::load_sections(&path)?;
        if sections.is_empty() {
            let text =
                std::fs::read_to_string(&path).map_err(|e| format!("cannot read {out}: {e}"))?;
            if !text.contains("# connectk-book v2") {
                return Err(format!("existing book {out} is not a v2 universal book"));
            }
            println!("Resuming universal book: no completed entries yet in {out}");
        }
        let mut saw_rules = sections.is_empty();
        for section in sections {
            let Some(section_rules) = section.rules() else {
                return Err(format!(
                    "existing book {out} has a section without rules metadata; use --format v1 or a new universal file"
                ));
            };
            saw_rules = true;
            if section_rules == &rules {
                done.extend(section.canonical_keys());
            }
        }
        if !saw_rules {
            return Err(format!("existing book {out} is not a v2 universal book"));
        }
        println!(
            "Resuming universal book: {} entries already present for these rules in {out}",
            done.len()
        );
    }

    let t0 = Instant::now();
    let mut todo: Vec<Position> = Vec::new();
    let mut visited: GeneralKeySet =
        HashSet::with_capacity_and_hasher(1 << 18, BuildFast::default());
    let mut frontier_seen = 0usize;
    enumerate_general(
        &mut root,
        depth,
        &mut visited,
        &done,
        &mut todo,
        limit,
        shard_index,
        shard_count,
        &mut frontier_seen,
    );
    let enumerated = todo.len();
    if sample > 0 {
        todo = select_sample(todo, sample);
    }
    println!(
        "Enumerated {} new canonical positions at ply {} ({} plies beyond the root at ply {}; frontier seen {}, shard {}/{}, selected {}) in {:.1} s; solving with {} thread(s), budget {} ms/position...",
        enumerated,
        target_depth,
        depth,
        root_ply,
        frontier_seen,
        shard_index + 1,
        shard_count,
        todo.len(),
        t0.elapsed().as_secs_f64(),
        threads,
        per_pos_time
    );
    if dry_run {
        println!("Dry run: no positions solved and no book written.");
        return Ok(());
    }

    let total = todo.len();
    let solved = AtomicI64::new(0);
    let unsolved = AtomicI64::new(0);
    let total_nodes = AtomicU64::new(0);

    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| format!("cannot open {out}: {e}"))?;
    let mut writer = BufWriter::new(file);
    if !exists {
        writeln!(writer, "# connectk-book v2 universal").map_err(|e| e.to_string())?;
    }
    writeln!(writer, "# section rules: {}", rules_line(&rules)).map_err(|e| e.to_string())?;
    writeln!(writer, "# depth={target_depth}").map_err(|e| e.to_string())?;
    if root_ply > 0 {
        let history = root
            .move_history()
            .iter()
            .map(|m| m.notation())
            .collect::<Vec<_>>()
            .join(" ");
        writeln!(writer, "# segment root ply {root_ply}: {history}").map_err(|e| e.to_string())?;
    }
    writer.flush().map_err(|e| e.to_string())?;

    let width = rules.width;
    let (job_tx, job_rx) = std::sync::mpsc::sync_channel::<Position>(threads * 2);
    let (res_tx, res_rx) = std::sync::mpsc::sync_channel::<String>(threads * 4);
    let job_rx = Arc::new(std::sync::Mutex::new(job_rx));
    let rules_ref = &rules;

    std::thread::scope(|scope| -> Result<(), String> {
        let mut workers = Vec::new();
        for _ in 0..threads {
            let job_rx = job_rx.clone();
            let res_tx = res_tx.clone();
            let solved = &solved;
            let unsolved = &unsolved;
            let total_nodes = &total_nodes;
            workers.push(scope.spawn(move || {
                let mut worker = ExactWorker::new(rules_ref);
                loop {
                    let pos = {
                        let rx = job_rx.lock().unwrap();
                        rx.recv()
                    };
                    let Ok(pos) = pos else { break };
                    let result = worker.solve(&pos, per_pos_nodes, per_pos_time);
                    total_nodes.fetch_add(result.nodes, Ordering::Relaxed);
                    if result.certainty != Certainty::ProvenExact {
                        unsolved.fetch_add(1, Ordering::Relaxed);
                        continue;
                    }
                    solved.fetch_add(1, Ordering::Relaxed);
                    let key = pos.canonical_key();
                    let flipped = key != pos.raw_key();
                    let best_move = result.best_move.map(|m| {
                        if flipped {
                            Move::cell(width - 1 - m.column, m.row)
                        } else {
                            m
                        }
                    });
                    let line = book_entry_line_v2(
                        rules_ref,
                        &key,
                        BookEntry {
                            score: result.score,
                            best_move,
                        },
                    );
                    if res_tx.send(line).is_err() {
                        break;
                    }
                }
            }));
        }
        drop(res_tx);
        let feeder = scope.spawn(move || {
            for pos in todo {
                if job_tx.send(pos).is_err() {
                    break;
                }
            }
        });

        let mut received = 0usize;
        while let Ok(line) = res_rx.recv() {
            if !line.is_empty() {
                writeln!(writer, "{line}").map_err(|e| e.to_string())?;
                if received.is_multiple_of(flush_every) {
                    writer.flush().map_err(|e| e.to_string())?;
                }
            }
            received += 1;
            let done_count =
                (solved.load(Ordering::Relaxed) + unsolved.load(Ordering::Relaxed)) as usize;
            if received.is_multiple_of(200) || done_count == total {
                println!(
                    "  {}/{} done ({} solved, {} unsolved, {} nodes, {:.0} s elapsed)",
                    done_count.max(received),
                    total,
                    solved.load(Ordering::Relaxed),
                    unsolved.load(Ordering::Relaxed),
                    total_nodes.load(Ordering::Relaxed),
                    t0.elapsed().as_secs_f64()
                );
            }
        }
        let _ = feeder.join();
        for w in workers {
            let _ = w.join();
        }
        Ok(())
    })?;
    writer.flush().map_err(|e| e.to_string())?;

    println!(
        "bookgen finished: {} solved, {} unsolved (retry by re-running), {} nodes total, {:.0} s.",
        solved.load(Ordering::Relaxed),
        unsolved.load(Ordering::Relaxed),
        total_nodes.load(Ordering::Relaxed),
        t0.elapsed().as_secs_f64()
    );
    println!(
        "Book file: {}",
        path.canonicalize().unwrap_or(path).display()
    );
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn enumerate_general(
    p: &mut Position,
    depth_left: i32,
    visited: &mut GeneralKeySet,
    done: &GeneralKeySet,
    out: &mut Vec<Position>,
    limit: usize,
    shard_index: usize,
    shard_count: usize,
    frontier_seen: &mut usize,
) {
    if out.len() >= limit || p.is_terminal() {
        return;
    }
    let key = p.canonical_key();
    if !visited.insert(key.clone()) {
        return;
    }
    if depth_left == 0 {
        let frontier_index = *frontier_seen;
        *frontier_seen += 1;
        if frontier_index % shard_count == shard_index && !done.contains(&key) {
            out.push(p.clone());
        }
        return;
    }
    for m in p.legal_moves() {
        p.play(m).expect("legal move");
        enumerate_general(
            p,
            depth_left - 1,
            visited,
            done,
            out,
            limit,
            shard_index,
            shard_count,
            frontier_seen,
        );
        p.undo().expect("undo");
        if out.len() >= limit {
            return;
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn enumerate(
    p: &mut Position,
    depth_left: i32,
    visited: &mut KeySet,
    done: &KeySet,
    out: &mut Vec<Position>,
    limit: usize,
    shard_index: usize,
    shard_count: usize,
    frontier_seen: &mut usize,
) {
    if out.len() >= limit || p.is_terminal() {
        return;
    }
    let (cur, mask, _) = FastExactSolver::encode_canonical(p);
    if !visited.insert((cur, mask)) {
        return;
    }
    if depth_left == 0 {
        let frontier_index = *frontier_seen;
        *frontier_seen += 1;
        if frontier_index % shard_count == shard_index && !done.contains(&(cur, mask)) {
            out.push(p.clone());
        }
        return;
    }
    for m in p.legal_moves() {
        p.play(m).expect("legal move");
        enumerate(
            p,
            depth_left - 1,
            visited,
            done,
            out,
            limit,
            shard_index,
            shard_count,
            frontier_seen,
        );
        p.undo().expect("undo");
        if out.len() >= limit {
            return;
        }
    }
}
