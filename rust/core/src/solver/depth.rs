use super::{Abort, Certainty, SearchControl, SearchResult};
use crate::bits;
use crate::eval::{self, Evaluator, WIN_SCORE};
use crate::hash::{fast_map_with_capacity, FastMap};
use crate::model::{Move, Outcome, Placement, Player, Rules};
use crate::position::{Position, PositionKey};
use std::time::Instant;

#[derive(Clone, Copy, PartialEq)]
enum Bound {
    Exact,
    Lower,
    Upper,
}

#[derive(Clone, PartialEq, Eq, Hash)]
enum DepthKey {
    Small([u64; 8]),
    Multi(PositionKey),
}

fn depth_key(pos: &Position) -> DepthKey {
    let words = pos.rules().word_count();
    if words <= 4 {
        let mut key = [0u64; 8];
        key[..words].copy_from_slice(pos.current_words());
        key[4..4 + words].copy_from_slice(pos.mask_words());
        DepthKey::Small(key)
    } else {
        DepthKey::Multi(pos.raw_key())
    }
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
    table: FastMap<DepthKey, Entry>,
    rules: Rules,
    pos: Position,
    evaluator: Evaluator,
    history: Vec<i32>,
    killers: Vec<[Option<Move>; 2]>,
    nodes: u64,
    candidate_restriction_used: bool,
    near_marks: Vec<u32>,
    near_generation: u32,
    own_spots_scratch: Vec<u64>,
    opp_spots_scratch: Vec<u64>,
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
            pos: Position::initial(&rules),
            evaluator: Evaluator::new(&rules),
            rules,
            history: Vec::new(),
            killers: Vec::new(),
            nodes: 0,
            candidate_restriction_used: false,
            near_marks: Vec::new(),
            near_generation: 0,
            own_spots_scratch: Vec::new(),
            opp_spots_scratch: Vec::new(),
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
        self.killers = vec![[None, None]; self.rules.cells() as usize + 1];
        self.nodes = 0;
        self.candidate_restriction_used = false;
        self.near_marks = vec![0; self.rules.cells().max(0) as usize];
        self.near_generation = 0;
        self.own_spots_scratch = vec![0; self.rules.word_count()];
        self.opp_spots_scratch = vec![0; self.rules.word_count()];
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
            let root = match self.search_root(depth) {
                Ok(r) => r,
                Err(_) => break,
            };
            completed_best = root.1;
            completed_score = root.0;
            completed_depth = depth;
            completed_pv =
                reconstruct_variation(position, &self.table, completed_best, self.rules.cells());
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
            "Iterative-deepening principal-variation search to depth {} plies with heuristic evaluation \
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
            .get(&depth_key(&self.pos))
            .and_then(|e| e.best_move);
        let moves = self.ordered_candidates(tt_move, true, depth);
        let mut alpha = -INF;
        let mut best = -INF;
        let mut best_move: Option<Move> = None;
        for (i, mv) in moves.into_iter().enumerate() {
            self.pos.apply_resolved(mv);
            let r = (|| -> Result<i32, Abort> {
                if i == 0 {
                    Ok(-self.negamax(depth - 1, -INF, -alpha)?)
                } else {
                    let probe = -self.negamax(depth - 1, -alpha - 1, -alpha)?;
                    if probe > alpha {
                        Ok(-self.negamax(depth - 1, -INF, -alpha)?)
                    } else {
                        Ok(probe)
                    }
                }
            })();
            self.pos.unapply_resolved(mv);
            let score = r?;
            if score > best {
                best = score;
                best_move = Some(mv);
            }
            if best > alpha {
                alpha = best;
            }
        }
        if best_move.is_some() && self.table.len() < MAX_TABLE_ENTRIES {
            self.table.insert(
                depth_key(&self.pos),
                Entry {
                    depth,
                    score: best,
                    bound: Bound::Exact,
                    best_move,
                },
            );
        }
        Ok((best, best_move))
    }

    fn negamax(&mut self, depth: i32, mut alpha: i32, mut beta: i32) -> Result<i32, Abort> {
        self.nodes += 1;
        self.control.check_depth(self.nodes)?;
        if self.pos.is_terminal() {
            return Ok(eval::terminal_score(&self.pos));
        }
        if depth == 0 {
            return Ok(self.evaluator.evaluate(&self.pos));
        }
        let alpha_orig = alpha;
        let key = depth_key(&self.pos);
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
        let moves = self.ordered_candidates(tt_move, false, depth);
        let mut best = -INF;
        let mut best_move: Option<Move> = None;
        for (i, mv) in moves.into_iter().enumerate() {
            self.pos.apply_resolved(mv);

            let r = (|| -> Result<i32, Abort> {
                if i == 0 {
                    self.negamax(depth - 1, -beta, -alpha)
                } else {
                    let probe = -self.negamax(depth - 1, -alpha - 1, -alpha)?;
                    if probe > alpha && probe < beta {
                        self.negamax(depth - 1, -beta, -alpha)
                    } else {
                        Ok(-probe)
                    }
                }
            })();
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
                let ply = self.pos.moves_played() as usize;
                if self.killers[ply][0] != Some(mv) {
                    self.killers[ply][1] = self.killers[ply][0];
                    self.killers[ply][0] = Some(mv);
                }
                break;
            }
        }

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
        Ok(best)
    }

    fn ordered_candidates(&mut self, tt_move: Option<Move>, root: bool, depth: i32) -> Vec<Move> {
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
        self.pos
            .write_winning_spots(side, &mut self.own_spots_scratch);
        self.pos
            .write_winning_spots(side.other(), &mut self.opp_spots_scratch);
        let own_spot_count = bits::bit_count(&self.own_spots_scratch);
        let opp_immediate_count = candidates
            .iter()
            .filter(|m| bits::test(&self.opp_spots_scratch, self.rules.index(m.column, m.row)))
            .count() as u32;
        let w = self.rules.width;
        let h = self.rules.height;
        let cells = self.rules.cells();
        let ply = self.pos.moves_played() as usize;
        let killer_pair = self.killers.get(ply).copied().unwrap_or([None, None]);
        let mut scored: Vec<(Move, i64)> = Vec::with_capacity(candidates.len());
        for mv in candidates {
            let mut s: i64 = 0;
            let idx = self.rules.index(mv.column, mv.row);
            if Some(mv) == tt_move {
                s += 10_000_000;
            }
            let completes = bits::test(&self.own_spots_scratch, idx);
            if completes {
                s += if self.rules.misere {
                    -9_000_000
                } else {
                    9_000_000
                };
            }
            if !self.rules.misere && bits::test(&self.opp_spots_scratch, idx) {
                s += 8_000_000;
            }
            if Some(mv) == killer_pair[0] {
                s += 6_000_000;
            } else if Some(mv) == killer_pair[1] {
                s += 5_500_000;
            }
            s += self.history[idx as usize] as i64;
            s += (mv.column.min(w - 1 - mv.column) + 1) as i64 * 10;
            s += (mv.row.min(h - 1 - mv.row) + 1) as i64 * 4;
            if depth > 1 && !completes && self.pos.moves_played() as i32 + 1 < cells {
                s += self.own_spots_after(mv, side, &self.own_spots_scratch, own_spot_count) as i64
                    * 1_000;
                if !self.rules.misere {
                    let blocks_immediate = bits::test(&self.opp_spots_scratch, idx) as u32;
                    let mut opponent_wins_next = opp_immediate_count > blocks_immediate;
                    if self.rules.placement == Placement::Gravity && mv.row + 1 < h {
                        opponent_wins_next |= bits::test(
                            &self.opp_spots_scratch,
                            self.rules.index(mv.column, mv.row + 1),
                        );
                    }
                    if opponent_wins_next {
                        s -= 500_000;
                    }
                }
            }
            scored.push((mv, s));
        }
        scored.sort_by_key(|a| std::cmp::Reverse(a.1));
        let cap = if self.rules.placement == Placement::Free {
            if root {
                36
            } else {
                28
            }
        } else if w > 18 {
            16
        } else {
            usize::MAX
        };
        if scored.len() > cap {
            self.candidate_restriction_used = true;
        }
        scored.into_iter().take(cap).map(|(m, _)| m).collect()
    }

    fn neighbourhood_candidates(&mut self, legal: Vec<Move>) -> Vec<Move> {
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
        self.near_generation = self.near_generation.wrapping_add(1);
        if self.near_generation == 0 {
            self.near_marks.fill(0);
            self.near_generation = 1;
        }
        let generation = self.near_generation;
        for stone in self.pos.move_history() {
            for dc in -2..=2 {
                for dr in -2..=2 {
                    let (cc, rr) = (stone.column + dc, stone.row + dr);
                    if cc >= 0 && cc < w && rr >= 0 && rr < h {
                        self.near_marks[(cc * h + rr) as usize] = generation;
                    }
                }
            }
        }
        let out: Vec<Move> = legal
            .iter()
            .copied()
            .filter(|m| self.near_marks[(m.column * h + m.row) as usize] == generation)
            .collect();
        if out.is_empty() {
            legal
        } else {
            out
        }
    }

    fn own_spots_after(
        &self,
        mv: Move,
        side: Player,
        own_spots: &[u64],
        own_spot_count: u32,
    ) -> u32 {
        let idx = self.rules.index(mv.column, mv.row);
        let mut count = own_spot_count - bits::test(own_spots, idx) as u32;
        let k = self.rules.connect;
        let mut created: Vec<i32> = Vec::new();
        for &(dc, dr) in self.rules.dir_steps() {
            for offset in 0..k {
                let start_c = mv.column - dc * offset;
                let start_r = mv.row - dr * offset;
                let mut own = 0;
                let mut empty_count = 0;
                let mut empty_idx = -1;
                let mut blocked = false;
                for i in 0..k {
                    let c = start_c + dc * i;
                    let r = start_r + dr * i;
                    if c < 0 || c >= self.rules.width || r < 0 || r >= self.rules.height {
                        blocked = true;
                        break;
                    }
                    if c == mv.column && r == mv.row {
                        own += 1;
                        continue;
                    }
                    match self.pos.owner_at(c, r) {
                        Some(p) if p == side => own += 1,
                        Some(_) => {
                            blocked = true;
                            break;
                        }
                        None => {
                            empty_count += 1;
                            empty_idx = self.rules.index(c, r);
                        }
                    }
                }
                if !blocked
                    && own == k - 1
                    && empty_count == 1
                    && !bits::test(own_spots, empty_idx)
                    && !created.contains(&empty_idx)
                {
                    created.push(empty_idx);
                    count += 1;
                }
            }
        }
        count
    }
}

fn reconstruct_variation(
    original: &Position,
    table: &FastMap<DepthKey, Entry>,
    root_best: Option<Move>,
    cells: i32,
) -> Vec<Move> {
    let mut pv = Vec::new();
    let mut cursor = original.clone();
    let mut next = root_best;
    while !cursor.is_terminal() && pv.len() as i32 <= cells {
        let Some(m) = next else {
            break;
        };
        match cursor.play(m) {
            Ok(cell) => pv.push(cell),
            Err(_) => break,
        }
        next = table
            .get(&depth_key(&cursor))
            .and_then(|entry| entry.best_move);
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
