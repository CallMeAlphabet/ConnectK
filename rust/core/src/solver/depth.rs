//! Depth-limited solver for positions that cannot (yet) be proved:
//! iterative-deepening negamax with alpha-beta pruning, a depth-aware
//! transposition table, history/move-ordering heuristics and the
//! static evaluator at the horizon. Results are always labelled
//! HEURISTIC_DEPTH_LIMITED.

use super::{Abort, Certainty, SearchControl, SearchResult};
use crate::bits;
use crate::eval::{self, Evaluator, WIN_SCORE};
use crate::hash::{fast_map_with_capacity, FastMap};
use crate::model::{Move, Outcome, Placement, Rules};
use crate::position::{Position, PositionKey};
use std::time::Instant;

#[derive(Clone, Copy, PartialEq)]
enum Bound {
    Exact,
    Lower,
    Upper,
}

#[derive(Clone, Copy)]
struct Entry {
    depth: i32,
    score: i32,
    bound: Bound,
    best_move: Option<Move>,
}

const MAX_TABLE_ENTRIES: usize = 1_000_000;
const INF: i32 = WIN_SCORE * 2;

pub struct DepthLimitedSolver {
    table: FastMap<PositionKey, Entry>,
    iteration_best: FastMap<PositionKey, Move>,
    rules: Rules,
    pos: Position,
    evaluator: Evaluator,
    history: Vec<i32>,
    nodes: u64,
    candidate_restriction_used: bool,
    control: SearchControl,
    start: Instant,
}

impl Default for DepthLimitedSolver {
    fn default() -> Self {
        Self::new()
    }
}

impl DepthLimitedSolver {
    pub fn new() -> DepthLimitedSolver {
        let rules = Rules::standard_connect4();
        DepthLimitedSolver {
            table: fast_map_with_capacity(1 << 15),
            iteration_best: fast_map_with_capacity(1024),
            pos: Position::initial(&rules),
            evaluator: Evaluator::new(&rules),
            rules,
            history: Vec::new(),
            nodes: 0,
            candidate_restriction_used: false,
            control: SearchControl::none(),
            start: Instant::now(),
        }
    }

    pub fn solve(
        &mut self,
        position: &Position,
        max_depth: i32,
        control: SearchControl,
    ) -> SearchResult {
        self.rules = position.rules().clone();
        self.pos = position.clone();
        self.evaluator = Evaluator::new(&self.rules);
        self.history = vec![0; self.rules.bit_count() as usize];
        self.nodes = 0;
        self.candidate_restriction_used = false;
        self.control = control;
        self.start = Instant::now();
        self.table.clear();

        if position.is_terminal() {
            let score = exact_terminal_score(position);
            return SearchResult::new(
                Certainty::ProvenExact,
                Some(Outcome::from_score_sign(score)),
                None,
                true,
                score,
                Some(0),
                0,
                0,
                0,
                Vec::new(),
                "Position is already terminal.".into(),
            );
        }

        let mut completed_best: Option<Move> = None;
        let mut completed_score = 0;
        let mut completed_depth = 0;
        let mut completed_pv: Vec<Move> = Vec::new();

        for depth in 1..=max_depth {
            self.iteration_best = fast_map_with_capacity(1024);
            let root = match self.search_root(depth) {
                Ok(r) => r,
                Err(_) => break, // keep the last fully completed iteration
            };
            completed_best = root.1;
            completed_score = root.0;
            completed_depth = depth;
            let snapshot = std::mem::replace(
                &mut self.iteration_best,
                fast_map_with_capacity(16),
            );
            completed_pv = reconstruct_variation(position, &snapshot, self.rules.cells());
            self.iteration_best = snapshot;
            self.control.report(self.nodes, depth, completed_best);
            if eval::is_winning_score(completed_score) {
                break;
            }
            if self.control.past_deadline() {
                break;
            }
        }

        let elapsed = self.elapsed_millis();
        let Some(best) = completed_best else {
            // Budget died during depth 1: statically ordered fallback.
            let legal = position.legal_moves();
            let fallback = legal.first().copied();
            let ev = self.evaluator.evaluate(position);
            return SearchResult::new(
                Certainty::UnknownLimitReached,
                Some(Outcome::from_score_sign(ev)),
                fallback,
                false,
                ev,
                None,
                0,
                self.nodes,
                elapsed,
                Vec::new(),
                "Budget exhausted before depth 1 completed; move is a static fallback, \
                 evaluation only."
                    .into(),
            );
        };

        let plies =
            if eval::is_winning_score(completed_score) || eval::is_losing_score(completed_score) {
                let terminal_moves = (WIN_SCORE - completed_score.abs()) / 10;
                Some((terminal_moves - position.moves_played() as i32).max(0))
            } else {
                None
            };
        let explanation = format!(
            "Iterative-deepening alpha-beta to depth {} plies with heuristic evaluation \
             ({} potential lines).{} This is a playing strength estimate, not a \
             mathematical proof.",
            completed_depth,
            self.evaluator.window_count(),
            if self.candidate_restriction_used {
                " Candidate moves were restricted to the neighbourhood of existing stones."
            } else {
                ""
            }
        );
        SearchResult::new(
            Certainty::HeuristicDepthLimited,
            Some(Outcome::from_score_sign(completed_score)),
            Some(best),
            false,
            completed_score,
            plies,
            completed_depth,
            self.nodes,
            elapsed,
            completed_pv,
            explanation,
        )
    }

    pub fn nodes_explored(&self) -> u64 {
        self.nodes
    }

    fn elapsed_millis(&self) -> u64 {
        self.start.elapsed().as_millis() as u64
    }

    fn search_root(&mut self, depth: i32) -> Result<(i32, Option<Move>), Abort> {
        self.nodes += 1;
        self.control.check_depth(self.nodes)?;
        let tt_move = self
            .table
            .get(&self.pos.raw_key())
            .and_then(|e| e.best_move);
        let moves = self.ordered_candidates(tt_move, true);
        let mut alpha = -INF;
        let mut best = -INF;
        let mut best_move: Option<Move> = None;
        for mv in moves {
            self.pos.apply_resolved(mv);
            let r = self.negamax(depth - 1, -INF, -alpha, 1);
            self.pos.unapply_resolved(mv);
            let score = -r?;
            if score > best {
                best = score;
                best_move = Some(mv);
            }
            if best > alpha {
                alpha = best;
            }
        }
        if let Some(bm) = best_move {
            self.iteration_best.insert(self.pos.raw_key(), bm);
        }
        Ok((best, best_move))
    }

    fn negamax(
        &mut self,
        depth: i32,
        mut alpha: i32,
        mut beta: i32,
        ply: i32,
    ) -> Result<i32, Abort> {
        self.nodes += 1;
        self.control.check_depth(self.nodes)?;
        if self.pos.is_terminal() {
            return Ok(eval::terminal_score(&self.pos));
        }
        if depth == 0 {
            return Ok(self.evaluator.evaluate(&self.pos));
        }
        let alpha_orig = alpha;
        let key = self.pos.raw_key();
        let mut tt_move: Option<Move> = None;
        if let Some(hit) = self.table.get(&key).copied() {
            tt_move = hit.best_move;
            if hit.depth >= depth {
                match hit.bound {
                    Bound::Exact => return Ok(hit.score),
                    Bound::Lower => alpha = alpha.max(hit.score),
                    Bound::Upper => beta = beta.min(hit.score),
                }
                if alpha >= beta {
                    return Ok(hit.score);
                }
            }
        }
        let moves = self.ordered_candidates(tt_move, false);
        let mut best = -INF;
        let mut best_move: Option<Move> = None;
        for mv in moves {
            self.pos.apply_resolved(mv);
            let r = self.negamax(depth - 1, -beta, -alpha, ply + 1);
            self.pos.unapply_resolved(mv);
            let score = -r?;
            if score > best {
                best = score;
                best_move = Some(mv);
            }
            if best > alpha {
                alpha = best;
            }
            if alpha >= beta {
                let idx = self.rules.index(mv.column, mv.row) as usize;
                self.history[idx] = self.history[idx].wrapping_add(depth * depth);
                break;
            }
        }
        // Bound classified against alpha_orig and the (possibly
        // TT-tightened) beta, exactly as the Java implementation.
        let bound = if best <= alpha_orig {
            Bound::Upper
        } else if best >= beta {
            Bound::Lower
        } else {
            Bound::Exact
        };
        if self.table.len() < MAX_TABLE_ENTRIES {
            self.table.insert(
                key.clone(),
                Entry {
                    depth,
                    score: best,
                    bound,
                    best_move,
                },
            );
        }
        if let Some(bm) = best_move {
            self.iteration_best.insert(key, bm);
        }
        Ok(best)
    }

    fn ordered_candidates(&mut self, tt_move: Option<Move>, root: bool) -> Vec<Move> {
        let legal = self.pos.legal_moves();
        if legal.len() <= 1 {
            return legal;
        }
        let mut candidates = legal;
        if self.rules.placement == Placement::Free && candidates.len() > 40 {
            candidates = self.neighbourhood_candidates(candidates);
            self.candidate_restriction_used = true;
        }
        let side = self.pos.side_to_move();
        let own_spots = self.pos.winning_spots(side);
        let opp_spots = self.pos.winning_spots(side.other());
        let w = self.rules.width;
        let h = self.rules.height;
        let mut scored: Vec<(Move, i64)> = Vec::with_capacity(candidates.len());
        for mv in candidates {
            let mut s: i64 = 0;
            let idx = self.rules.index(mv.column, mv.row);
            if Some(mv) == tt_move {
                s += 10_000_000;
            }
            let completes = bits::test(&own_spots, idx);
            if completes {
                s += if self.rules.misere { -9_000_000 } else { 9_000_000 };
            }
            if !self.rules.misere && bits::test(&opp_spots, idx) {
                s += 8_000_000;
            }
            s += self.history[idx as usize] as i64;
            s += (mv.column.min(w - 1 - mv.column) + 1) as i64 * 10;
            s += (mv.row.min(h - 1 - mv.row) + 1) as i64 * 4;
            self.pos.apply_resolved(mv);
            if !self.pos.is_terminal() {
                s += self.pos.count_winning_spots(side) as i64 * 1_000;
                if !self.rules.misere
                    && !self.pos.line_completing_moves(self.pos.side_to_move()).is_empty()
                {
                    s -= 500_000;
                }
            }
            self.pos.unapply_resolved(mv);
            scored.push((mv, s));
        }
        scored.sort_by(|a, b| b.1.cmp(&a.1)); // stable
        let cap = if self.rules.placement == Placement::Free {
            if root { 36 } else { 28 }
        } else if w > 18 {
            16
        } else {
            usize::MAX
        };
        if scored.len() > cap {
            self.candidate_restriction_used = true;
        }
        scored
            .into_iter()
            .take(cap)
            .map(|(m, _)| m)
            .collect()
    }

    fn neighbourhood_candidates(&self, legal: Vec<Move>) -> Vec<Move> {
        let w = self.rules.width;
        let h = self.rules.height;
        if self.pos.moves_played() == 0 {
            let centre: Vec<Move> = legal
                .iter()
                .copied()
                .filter(|m| {
                    let centre_col = m.column == (w - 1) / 2 || m.column == w / 2;
                    let centre_row = m.row == (h - 1) / 2 || m.row == h / 2;
                    centre_col && centre_row
                })
                .collect();
            return if centre.is_empty() { legal } else { centre };
        }
        let mut near = vec![vec![false; h as usize]; w as usize];
        for c in 0..w {
            for r in 0..h {
                if self.pos.is_empty(c, r) {
                    continue;
                }
                for dc in -2..=2 {
                    for dr in -2..=2 {
                        let (cc, rr) = (c + dc, r + dr);
                        if cc >= 0 && cc < w && rr >= 0 && rr < h {
                            near[cc as usize][rr as usize] = true;
                        }
                    }
                }
            }
        }
        let out: Vec<Move> = legal
            .iter()
            .copied()
            .filter(|m| near[m.column as usize][m.row as usize])
            .collect();
        if out.is_empty() {
            legal
        } else {
            out
        }
    }
}

fn reconstruct_variation(
    original: &Position,
    best_moves: &FastMap<PositionKey, Move>,
    cells: i32,
) -> Vec<Move> {
    let mut pv = Vec::new();
    let mut cursor = original.clone();
    while !cursor.is_terminal() && pv.len() as i32 <= cells {
        let Some(&m) = best_moves.get(&cursor.raw_key()) else {
            break;
        };
        match cursor.play(m) {
            Ok(cell) => pv.push(cell),
            Err(_) => break,
        }
    }
    pv
}

fn exact_terminal_score(p: &Position) -> i32 {
    match p.winner() {
        None => 0,
        Some(w) => {
            let magnitude = p.rules().cells() - p.moves_played() as i32 + 1;
            if w == p.side_to_move() {
                magnitude
            } else {
                -magnitude
            }
        }
    }
}
