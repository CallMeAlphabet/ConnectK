//! Parallel exact proving: root-move splitting with null-window
//! refutations.
//!
//! Strategy (a simplification of the Java frontier splitter with the
//! same proof semantics — PROVEN_EXACT or UNKNOWN, never a guess):
//! 1. Solve the first (centre-most) root move exactly; its value is
//!    the initial alpha.
//! 2. In parallel, null-window test every other root move against
//!    alpha ("can this move be better?") — refutations are far cheaper
//!    than exact proofs and dominate the work.
//! 3. Sequentially exact-solve the few survivors, updating alpha.
//!
//! Deviations from the Java version (documented, deliberate): the Java
//! frontier protocol is replaced by this simpler root split, and node
//! limits apply per worker rather than globally (time limits and
//! cancellation are shared). Non-fast rules fall back to the generic
//! solver sequentially.

use super::fast::FastExactSolver;
use super::{center_out_order, Certainty, SearchControl, SearchResult};
use crate::book::OpeningBook;
use crate::model::{Move, Outcome};
use crate::position::Position;
use std::sync::Arc;
use std::time::Instant;

pub struct ParallelExactSolver {
    threads: usize,
    book: Option<Arc<OpeningBook>>,
}

impl ParallelExactSolver {
    pub fn new(threads: usize, book: Option<Arc<OpeningBook>>) -> ParallelExactSolver {
        ParallelExactSolver {
            threads: threads.max(1),
            book,
        }
    }

    pub fn solve(&self, position: &Position, control: SearchControl) -> SearchResult {
        let start = Instant::now();
        let rules = position.rules();
        if !FastExactSolver::supports(rules) || position.is_terminal() {
            return super::generic::ExactSolver::new().solve(position, control);
        }
        let cells = rules.cells();
        let legal = position.legal_moves();
        if legal.is_empty() {
            return super::generic::ExactSolver::new().solve(position, control);
        }
        // Centre-out root order.
        let order = center_out_order(rules.width);
        let mut root_moves: Vec<Move> = Vec::with_capacity(legal.len());
        for col in order {
            for m in &legal {
                if m.column == col {
                    root_moves.push(*m);
                }
            }
        }

        let mut total_nodes: u64 = 0;
        // Phase 1: exact-solve the first move's child.
        let first = root_moves[0];
        let mut child = position.clone();
        child.apply_resolved(first);
        let mut solver = FastExactSolver::new();
        solver.set_opening_book(self.book.clone());
        let first_result = solver.solve(&child, control.clone());
        total_nodes += first_result.nodes;
        if first_result.certainty != Certainty::ProvenExact {
            return unknown(start, total_nodes, &control);
        }
        let mut alpha = -first_result.score;
        let mut best_move = first;
        let mut best_line = first_result.principal_line.clone();
        let rest = &root_moves[1..];
        if rest.is_empty() {
            return proven(position, alpha, best_move, best_line, total_nodes, start);
        }

        // Phase 2 (parallel): null-window refutation tests vs alpha0.
        let alpha0 = alpha;
        let workers = self.threads.min(rest.len());
        let mut tests: Vec<Option<(i32, u64)>> = vec![None; rest.len()]; // (childBound, nodes)
        let chunks: Vec<Vec<usize>> = (0..workers)
            .map(|w| (0..rest.len()).filter(|i| i % workers == w).collect())
            .collect();
        let base = position.clone();
        let results: Vec<Vec<(usize, i32, u64, bool)>> = std::thread::scope(|scope| {
            let mut handles = Vec::new();
            for chunk in &chunks {
                let base = base.clone();
                let control = control.clone();
                let book = self.book.clone();
                let rest_moves: Vec<Move> = chunk.iter().map(|&i| rest[i]).collect();
                let idxs: Vec<usize> = chunk.clone();
                handles.push(scope.spawn(move || {
                    let mut solver = FastExactSolver::new();
                    solver.set_opening_book(book);
                    let mut out = Vec::new();
                    for (idx, mv) in idxs.into_iter().zip(rest_moves) {
                        let mut child = base.clone();
                        child.apply_resolved(mv);
                        // Move beats alpha0 iff childValue <= -alpha0-1.
                        let b = solver.search_bound(&child, -alpha0 - 1, -alpha0, control.clone());
                        out.push((idx, b.value, b.nodes, b.aborted));
                    }
                    out
                }));
            }
            handles
                .into_iter()
                .map(|h| h.join().expect("worker thread panicked"))
                .collect()
        });
        for part in results {
            for (idx, value, nodes, aborted) in part {
                total_nodes += nodes;
                if aborted {
                    return unknown(start, total_nodes, &control);
                }
                tests[idx] = Some((value, nodes));
            }
        }

        // Phase 3 (sequential): exact-solve survivors (childValue proven
        // <= -alpha0-1, i.e. the move might beat alpha0).
        for (i, &mv) in rest.iter().enumerate() {
            let (value, _) = tests[i].unwrap();
            if value > -alpha0 - 1 {
                continue; // refuted: move value <= alpha0 <= alpha
            }
            let mut child = position.clone();
            child.apply_resolved(mv);
            let mut solver = FastExactSolver::new();
            solver.set_opening_book(self.book.clone());
            let r = solver.solve(&child, control.clone());
            total_nodes += r.nodes;
            if r.certainty != Certainty::ProvenExact {
                return unknown(start, total_nodes, &control);
            }
            let move_value = -r.score;
            if move_value > alpha {
                alpha = move_value;
                best_move = mv;
                best_line = r.principal_line.clone();
            }
        }
        let _ = cells;
        proven(position, alpha, best_move, best_line, total_nodes, start)
    }
}

fn proven(
    position: &Position,
    score: i32,
    best_move: Move,
    child_line: Vec<Move>,
    nodes: u64,
    start: Instant,
) -> SearchResult {
    let cells = position.rules().cells();
    let plies = if score == 0 {
        cells - position.moves_played() as i32
    } else {
        cells - score.abs() + 1 - position.moves_played() as i32
    };
    let mut line = vec![best_move];
    line.extend(child_line);
    SearchResult::new(
        Certainty::ProvenExact,
        Some(Outcome::from_score_sign(score)),
        Some(best_move),
        true,
        score,
        Some(plies),
        plies,
        nodes,
        start.elapsed().as_millis() as u64,
        line,
        "Proved by parallel exact search (root split with null-window refutations).".into(),
    )
}

fn unknown(start: Instant, nodes: u64, control: &SearchControl) -> SearchResult {
    let why = if control.is_cancelled() {
        "CANCELLED"
    } else if control.past_deadline() {
        "TIME_LIMIT"
    } else {
        "NODE_LIMIT"
    };
    SearchResult::unknown(
        nodes,
        start.elapsed().as_millis() as u64,
        format!("Exact proof aborted ({why}) after {nodes} nodes; no conclusion claimed."),
    )
}
