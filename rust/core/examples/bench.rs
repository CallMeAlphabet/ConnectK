//! Benchmark harness for the exact solvers, using Pascal Pons's public
//! Connect-4 test sets (the protocol used by Fhourstones and Kite).
//! File format: `<move-sequence> <score>` per line, 1-based columns on
//! the standard 7×6 board, Pons score convention
//! (ours -> Pons: sign * ceil(|ours|/2)).
//!
//! Usage: cargo run --release --example bench -- <file> [maxPositions]
//!        [perPositionMillis] [generic|fast]

use connectk_core::model::Rules;
use connectk_core::position::Position;
use connectk_core::solver::fast::FastExactSolver;
use connectk_core::solver::generic::ExactSolver;
use connectk_core::solver::{Certainty, SearchControl, SearchResult};
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
        eprintln!("usage: bench <file> [maxPositions] [perPositionMillis] [generic|fast]");
        std::process::exit(2);
    }
    let file = &args[1];
    let max_positions: usize = args
        .get(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(usize::MAX);
    let per_position_millis: u64 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(10_000);
    let fast = args
        .get(4)
        .map(|s| s.eq_ignore_ascii_case("fast"))
        .unwrap_or(false);

    let cases = load(file, max_positions);
    println!(
        "Benchmark: {} ({} positions, {} ms/position budget, solver={})",
        std::path::Path::new(file)
            .file_name()
            .unwrap()
            .to_string_lossy(),
        cases.len(),
        per_position_millis,
        if fast { "fast" } else { "generic" }
    );

    // No JIT warm-up needed for native code; one discarded solve pages
    // in the transposition table allocation.
    let mut fast_solver = FastExactSolver::new();
    let solve = |fast_solver: &mut FastExactSolver, p: &Position| -> SearchResult {
        if fast {
            fast_solver.solve(p, SearchControl::new(0, per_position_millis))
        } else {
            ExactSolver::new().solve(p, SearchControl::new(0, per_position_millis))
        }
    };
    if let Some((p, _)) = cases.first() {
        let _ = solve(&mut fast_solver, p);
    }

    let (mut total_nodes, mut total_ms, mut solved, mut correct, mut timeouts) =
        (0u64, 0u64, 0, 0, 0);
    let wall = Instant::now();
    for (pos, expected) in &cases {
        let r = solve(&mut fast_solver, pos);
        total_nodes += r.nodes;
        total_ms += r.elapsed_millis;
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
}
