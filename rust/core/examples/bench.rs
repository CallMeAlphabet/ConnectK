use connectk_core::book::OpeningBook;
use connectk_core::model::Rules;
use connectk_core::position::Position;
use connectk_core::solver::fast::FastExactSolver;
use connectk_core::solver::generic::ExactSolver;
use connectk_core::solver::standard::StandardSolver;
use connectk_core::solver::{Certainty, SearchControl, SearchResult};
use std::sync::Arc;
use std::time::Instant;

fn to_pons_score(score: i32) -> i32 {
    let a = score.abs();
    let v = (a + 1) / 2;
    if score < 0 {
        -v
    } else {
        v
    }
}

fn load(path: &str, max_positions: usize) -> Vec<(Position, i32)> {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| {
        eprintln!("cannot read {path}: {e}");
        std::process::exit(2);
    });
    let rules = Rules::standard_connect4();
    let mut out = Vec::new();
    for line in text.lines() {
        if out.len() >= max_positions {
            break;
        }
        let mut parts = line.split_whitespace();
        let (Some(moves), Some(score)) = (parts.next(), parts.next()) else {
            continue;
        };
        if moves.is_empty() || !moves.chars().all(|c| ('1'..='7').contains(&c)) {
            continue;
        }
        let Ok(expected) = score.parse::<i32>() else {
            continue;
        };
        let mut pos = Position::initial(&rules);
        let mut ok = true;
        for ch in moves.chars() {
            if pos.play_column(ch as i32 - '1' as i32).is_err() {
                ok = false;
                break;
            }
        }
        if ok {
            out.push((pos, expected));
        }
    }
    out
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!(
            "usage: bench <file> [maxPositions] [perPositionMillis] [generic|fast|standard] [bookFile|-] [persistent]"
        );
        std::process::exit(2);
    }
    let file = &args[1];
    let max_positions: usize = args
        .get(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(usize::MAX);
    let per_position_millis: u64 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(10_000);
    let solver_name = args
        .get(4)
        .map(|s| s.to_lowercase())
        .unwrap_or_else(|| "generic".into());

    if !matches!(solver_name.as_str(), "generic" | "fast" | "standard") {
        eprintln!("solver must be generic, fast, or standard");
        std::process::exit(2);
    }
    let persistent_tt = args.get(6).map(|s| s == "persistent").unwrap_or(false);
    let rules = Rules::standard_connect4();
    let book = match args.get(5).map(|s| s.as_str()) {
        None | Some("auto") => match OpeningBook::discover_all_for_rules(&rules) {
            Some((paths, book)) => {
                let source = paths
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                println!(
                    "Opening book (auto): {} entries at depth {} from {}",
                    book.size(),
                    book.depth(),
                    source
                );
                Some(Arc::new(book))
            }
            None => {
                println!("Opening book (auto): none discovered");
                None
            }
        },
        Some("-") | Some("none") => {
            println!("Opening book: none (disabled)");
            None
        }
        Some(path) => {
            let book = OpeningBook::load_for_rules(std::path::Path::new(path), &rules)
                .unwrap_or_else(|e| panic!("cannot load book {path}: {e}"))
                .unwrap_or_else(|| panic!("book {path} has no standard 7x6 section"));
            println!(
                "Opening book: {} entries at depth {} from {}",
                book.size(),
                book.depth(),
                path
            );
            Some(Arc::new(book))
        }
    };
    if persistent_tt {
        println!("Standard solver transposition table: persistent across benchmark positions");
    }

    let cases = load(file, max_positions);
    println!(
        "Benchmark: {} ({} positions, {} ms/position budget, solver={})",
        std::path::Path::new(file)
            .file_name()
            .unwrap()
            .to_string_lossy(),
        cases.len(),
        per_position_millis,
        solver_name
    );

    let mut fast_solver = FastExactSolver::new();
    fast_solver.set_opening_book(book.clone());
    let mut standard_solver = StandardSolver::new(1 << 21);
    standard_solver.set_opening_book(book.clone());
    standard_solver.set_persistent_tt(persistent_tt);
    let mut generic_solver = ExactSolver::new();
    generic_solver.set_opening_book(book);
    let solve = |fast_solver: &mut FastExactSolver,
                 standard_solver: &mut StandardSolver,
                 generic_solver: &mut ExactSolver,
                 p: &Position|
     -> SearchResult {
        match solver_name.as_str() {
            "fast" => fast_solver.solve(p, SearchControl::new(0, per_position_millis)),
            "standard" => {
                standard_solver.evaluate_score(p, SearchControl::new(0, per_position_millis))
            }
            _ => generic_solver.solve(p, SearchControl::new(0, per_position_millis)),
        }
    };
    if let Some((p, _)) = cases.first() {
        let _ = solve(
            &mut fast_solver,
            &mut standard_solver,
            &mut generic_solver,
            p,
        );
    }

    let (mut total_nodes, mut total_ms, mut solved, mut correct, mut timeouts) =
        (0u64, 0u64, 0, 0, 0);
    let (mut max_nodes, mut max_ms) = (0u64, 0u64);
    let wall = Instant::now();
    for (pos, expected) in &cases {
        let r = solve(
            &mut fast_solver,
            &mut standard_solver,
            &mut generic_solver,
            pos,
        );
        total_nodes += r.nodes;
        total_ms += r.elapsed_millis;
        max_nodes = max_nodes.max(r.nodes);
        max_ms = max_ms.max(r.elapsed_millis);
        if r.certainty == Certainty::ProvenExact {
            solved += 1;
            if to_pons_score(r.score) == *expected {
                correct += 1;
            } else {
                println!(
                    "  MISMATCH: ours={} (Pons {}) expected Pons {}",
                    r.score,
                    to_pons_score(r.score),
                    expected
                );
            }
        } else {
            timeouts += 1;
        }
    }
    let wall_s = wall.elapsed().as_secs_f64();
    println!(
        "solved {solved}/{}, correct {correct}, timeouts {timeouts}",
        cases.len()
    );
    println!("total nodes {total_nodes} | solver time {total_ms} ms | wall {wall_s:.2} s");
    if total_ms > 0 {
        println!(
            "throughput: {:.0} nodes/s (solver-reported time)",
            total_nodes as f64 * 1000.0 / total_ms as f64
        );
    }
    if solved > 0 {
        println!(
            "mean per solved position: {:.0} nodes, {:.3} ms",
            total_nodes as f64 / solved as f64,
            total_ms as f64 / solved as f64
        );
    }
    println!(
        "RESULT solver={} positions={} solved={} correct={} timeouts={} nodes={} solver_ms={} wall_ms={:.0} max_nodes={} max_ms={}",
        solver_name,
        cases.len(),
        solved,
        correct,
        timeouts,
        total_nodes,
        total_ms,
        wall_s * 1000.0,
        max_nodes,
        max_ms
    );
    if correct != cases.len() || solved != cases.len() {
        std::process::exit(1);
    }
}
