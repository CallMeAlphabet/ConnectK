use super::{stone_count_bounds, Abort, Certainty, SearchControl, SearchResult};
use crate::book::OpeningBook;
use crate::hash::{fast_map_with_capacity, FastMap};
use crate::model::{Move, Outcome, Rules};
use crate::position::{Position, PositionKey};
use std::sync::Arc;
use std::time::Instant;

#[derive(Clone, Copy, PartialEq)]
enum Bound {
    Exact,
    Lower,
    Upper,
}

#[derive(Clone, Copy)]
struct Entry {
    lower: i32,
    upper: i32,
}

enum Prepared {
    Forced(i32, Option<Move>),
    Moves(Vec<Move>),
}

const MAX_TABLE_ENTRIES: usize = 1_000_000;

pub struct ExactSolver {
    table: FastMap<PositionKey, Entry>,
    principal_moves: FastMap<PositionKey, Move>,
    rules: Rules,
    pos: Position,
    cells: i32,
    infinity: i32,
    nodes: u64,
    control: SearchControl,
    start: Instant,
    book: Option<Arc<OpeningBook>>,
}

impl Default for ExactSolver {
    fn default() -> Self {
        Self::new()
    }
}

impl ExactSolver {
    pub fn new() -> ExactSolver {
        ExactSolver {
            table: fast_map_with_capacity(1 << 15),
            principal_moves: fast_map_with_capacity(1024),
            rules: Rules::standard_connect4(),
            pos: Position::initial(&Rules::standard_connect4()),
            cells: 0,
            infinity: 0,
            nodes: 0,
            control: SearchControl::none(),
            start: Instant::now(),
            book: None,
        }
    }

    pub fn set_opening_book(&mut self, book: Option<Arc<OpeningBook>>) {
        self.book = book;
    }

    pub fn solve(&mut self, position: &Position, control: SearchControl) -> SearchResult {
        self.rules = position.rules().clone();
        self.pos = position.clone();
        self.cells = self.rules.cells();
        self.infinity = self.cells + 2;
        self.nodes = 0;
        self.control = control;
        self.start = Instant::now();
        self.table.clear();
        self.principal_moves.clear();

        if position.is_terminal() {
            let score = terminal_score(position, self.cells);
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
        if let Some(entry) = self
            .book
            .as_ref()
            .and_then(|book| book.lookup_entry(position))
        {
            if let Some(best_move) = entry.best_move {
                self.nodes = 1;
                let plies = if entry.score == 0 {
                    self.cells - position.moves_played() as i32
                } else {
                    self.cells - entry.score.abs() + 1 - position.moves_played() as i32
                };
                return SearchResult::new(
                    Certainty::ProvenExact,
                    Some(Outcome::from_score_sign(entry.score)),
                    Some(best_move),
                    true,
                    entry.score,
                    Some(plies),
                    plies,
                    self.nodes,
                    self.elapsed_millis(),
                    vec![best_move],
                    "Exact value and move from a generated opening book.".into(),
                );
            }
        }
        match self.search_root() {
            Ok((score, best_move)) => {
                let elapsed = self.elapsed_millis();
                let pv = self.reconstruct_variation(position);
                let plies = if score == 0 {
                    self.cells - position.moves_played() as i32
                } else {
                    let terminal_ply = self.cells - score.abs() + 1;
                    terminal_ply - position.moves_played() as i32
                };
                SearchResult::new(
                    Certainty::ProvenExact,
                    Some(Outcome::from_score_sign(score)),
                    best_move,
                    true,
                    score,
                    Some(plies),
                    plies,
                    self.nodes,
                    elapsed,
                    pv,
                    if self.book.is_some() {
                        "Proved by exhaustive alpha-beta search with a generated exact opening book."
                    } else {
                        "Proved by exhaustive alpha-beta search (negamax, transposition table, threat ordering)."
                    }
                    .into(),
                )
            }
            Err(Abort(reason)) => SearchResult::unknown(
                self.nodes,
                self.elapsed_millis(),
                format!(
                    "Exact proof aborted ({}) after {} nodes; no conclusion claimed.",
                    reason.name(),
                    self.nodes
                ),
            ),
        }
    }

    pub fn nodes_explored(&self) -> u64 {
        self.nodes
    }

    fn elapsed_millis(&self) -> u64 {
        self.start.elapsed().as_millis() as u64
    }

    fn search_root(&mut self) -> Result<(i32, Option<Move>), Abort> {
        self.nodes += 1;
        self.control.check_exact(self.nodes)?;
        match self.prepare_moves() {
            Prepared::Forced(score, forced_move) => {
                if let Some(m) = forced_move {
                    self.principal_moves.insert(self.pos.raw_key(), m);
                }
                Ok((score, forced_move))
            }
            Prepared::Moves(moves) => {
                let mut alpha = -self.infinity;
                let beta = self.infinity;
                let mut best = -self.infinity;
                let mut best_move: Option<Move> = None;
                for (i, mv) in moves.into_iter().enumerate() {
                    self.pos.apply_resolved(mv);
                    let r = (|| -> Result<i32, Abort> {
                        if i == 0 {
                            Ok(-self.negamax(-beta, -alpha)?)
                        } else {
                            let s = -self.negamax(-alpha - 1, -alpha)?;
                            if s > alpha {
                                Ok(-self.negamax(-beta, -alpha)?)
                            } else {
                                Ok(s)
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
                if let Some(bm) = best_move {
                    self.principal_moves.insert(self.pos.raw_key(), bm);
                }
                Ok((best, best_move))
            }
        }
    }

    fn negamax(&mut self, mut alpha: i32, mut beta: i32) -> Result<i32, Abort> {
        self.nodes += 1;
        self.control.check_exact(self.nodes)?;
        if self.pos.is_terminal() {
            return Ok(terminal_score(&self.pos, self.cells));
        }
        let mut alpha_orig = alpha;
        let mut beta_orig = beta;
        let key = self.pos.canonical_key();
        if let Some(score) = self
            .book
            .as_ref()
            .and_then(|book| book.lookup_canonical(&key))
            .map(|entry| entry.score)
        {
            self.store(key, score, Bound::Exact);
            return Ok(score);
        }
        if let Some(hit) = self.table.get(&key).copied() {
            if hit.lower == hit.upper {
                return Ok(hit.lower);
            }
            if hit.lower > alpha {
                alpha = hit.lower;
                if alpha >= beta {
                    return Ok(hit.lower);
                }
            }
            if hit.upper < beta {
                beta = hit.upper;
                if alpha >= beta {
                    return Ok(hit.upper);
                }
            }
            alpha_orig = alpha;
            beta_orig = beta;
        }
        if !self.rules.misere && self.rules.connect >= 1 {
            let (stone_lower, stone_upper) = stone_count_bounds(
                self.cells,
                self.pos.moves_played() as i32,
                self.rules.connect,
            );
            if stone_lower > alpha {
                alpha = stone_lower;
                if alpha >= beta {
                    return Ok(alpha);
                }
            }
            if stone_upper < beta {
                beta = stone_upper;
                if alpha >= beta {
                    return Ok(beta);
                }
            }
            alpha_orig = alpha;
            beta_orig = beta;
        }
        match self.prepare_moves() {
            Prepared::Forced(score, forced_move) => {
                if let Some(m) = forced_move {
                    if self.principal_moves.len() < MAX_TABLE_ENTRIES {
                        self.principal_moves.insert(self.pos.raw_key(), m);
                    }
                }
                self.store(key, score, Bound::Exact);
                Ok(score)
            }
            Prepared::Moves(moves) => {
                let mut best = -self.infinity;
                let mut best_move: Option<Move> = None;
                for (i, mv) in moves.into_iter().enumerate() {
                    self.pos.apply_resolved(mv);
                    let r = (|| -> Result<i32, Abort> {
                        if i == 0 {
                            Ok(-self.negamax(-beta, -alpha)?)
                        } else {
                            let s = -self.negamax(-alpha - 1, -alpha)?;
                            if s > alpha && s < beta {
                                Ok(-self.negamax(-beta, -alpha)?)
                            } else {
                                Ok(s)
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
                    if alpha >= beta {
                        break;
                    }
                }
                let bound = if best <= alpha_orig {
                    Bound::Upper
                } else if best >= beta_orig {
                    Bound::Lower
                } else {
                    Bound::Exact
                };
                let raw = self.pos.raw_key();
                self.store(key, best, bound);
                if let Some(bm) = best_move {
                    if self.principal_moves.len() < MAX_TABLE_ENTRIES {
                        self.principal_moves.insert(raw, bm);
                    }
                }
                Ok(best)
            }
        }
    }

    fn store(&mut self, key: PositionKey, score: i32, bound: Bound) {
        match bound {
            Bound::Exact => {
                if self.table.contains_key(&key) || self.table.len() < MAX_TABLE_ENTRIES {
                    self.table.insert(
                        key,
                        Entry {
                            lower: score,
                            upper: score,
                        },
                    );
                }
            }
            Bound::Lower => {
                if let Some(e) = self.table.get_mut(&key) {
                    e.lower = e.lower.max(score);
                } else if self.table.len() < MAX_TABLE_ENTRIES {
                    self.table.insert(
                        key,
                        Entry {
                            lower: score,
                            upper: i32::MAX / 2,
                        },
                    );
                }
            }
            Bound::Upper => {
                if let Some(e) = self.table.get_mut(&key) {
                    e.upper = e.upper.min(score);
                } else if self.table.len() < MAX_TABLE_ENTRIES {
                    self.table.insert(
                        key,
                        Entry {
                            lower: i32::MIN / 2,
                            upper: score,
                        },
                    );
                }
            }
        }
    }

    fn prepare_moves(&mut self) -> Prepared {
        let mut legal = self.pos.legal_moves();
        let side = self.pos.side_to_move();
        let moves = self.pos.moves_played() as i32;
        if !self.rules.misere {
            let winners = self.pos.line_completing_moves(side);
            if !winners.is_empty() {
                return Prepared::Forced(self.cells - moves, Some(winners[0]));
            }
            let threats = self.pos.line_completing_moves(side.other());
            if threats.len() >= 2 {
                return Prepared::Forced(-(self.cells - moves - 1), legal.first().copied());
            }
            if threats.len() == 1 {
                legal = threats;
            }
        } else {
            let suicides = self.pos.line_completing_moves(side);
            if !suicides.is_empty() {
                let safe: Vec<Move> = legal
                    .iter()
                    .copied()
                    .filter(|m| !suicides.contains(m))
                    .collect();
                if safe.is_empty() {
                    return Prepared::Forced(-(self.cells - moves), Some(legal[0]));
                }
                legal = safe;
            }
        }
        Prepared::Moves(self.order_moves(legal))
    }

    fn order_moves(&mut self, moves: Vec<Move>) -> Vec<Move> {
        if moves.len() <= 1 {
            return moves;
        }
        let side = self.pos.side_to_move();
        let opponent_spots = if self.rules.misere {
            None
        } else {
            Some(self.pos.winning_spots(side.other()))
        };
        let w = self.rules.width;
        let mut scored: Vec<(Move, i32)> = Vec::with_capacity(moves.len());
        for mv in moves {
            let mut s = (mv.column.min(w - 1 - mv.column) + 1) * 10;
            s += line_count_through(&self.rules, mv) * 10;
            if let Some(spots) = &opponent_spots {
                if crate::bits::test(spots, self.rules.index(mv.column, mv.row)) {
                    s += 5_000;
                }
            }
            self.pos.apply_resolved(mv);
            if !self.pos.is_terminal() {
                s += self.pos.count_winning_spots(side) as i32 * 100;
                if !self.rules.misere
                    && !self
                        .pos
                        .line_completing_moves(self.pos.side_to_move())
                        .is_empty()
                {
                    s -= 20_000;
                }
            }
            self.pos.unapply_resolved(mv);
            scored.push((mv, s));
        }
        scored.sort_by_key(|a| std::cmp::Reverse(a.1));
        scored.into_iter().map(|(m, _)| m).collect()
    }

    fn reconstruct_variation(&self, original: &Position) -> Vec<Move> {
        let mut pv = Vec::new();
        let mut cursor = original.clone();
        while !cursor.is_terminal() && pv.len() as i32 <= self.cells {
            let m = match self.principal_moves.get(&cursor.raw_key()) {
                Some(&m) => m,
                None => {
                    if !cursor.rules().misere {
                        let wins = cursor.line_completing_moves(cursor.side_to_move());
                        if !wins.is_empty() {
                            wins[0]
                        } else {
                            break;
                        }
                    } else {
                        break;
                    }
                }
            };
            match cursor.play(m) {
                Ok(cell) => pv.push(cell),
                Err(_) => break,
            }
        }
        pv
    }
}

fn line_count_through(rules: &Rules, mv: Move) -> i32 {
    if rules.connect <= 0 || rules.connect > rules.max_line() {
        return 0;
    }
    let k = rules.connect;
    let mut total = 0;
    for &(dc, dr) in rules.dir_steps() {
        let mut lo = 0;
        let mut hi = k - 1;
        for (x, n, d) in [(mv.column, rules.width, dc), (mv.row, rules.height, dr)] {
            if d > 0 {
                lo = lo.max(x + k - n);
                hi = hi.min(x);
            } else if d < 0 {
                lo = lo.max(k - 1 - x);
                hi = hi.min(n - 1 - x);
            }
        }
        if hi >= lo {
            total += hi - lo + 1;
        }
    }
    total
}

fn terminal_score(p: &Position, cells: i32) -> i32 {
    match p.winner() {
        None => 0,
        Some(w) => {
            let magnitude = cells - p.moves_played() as i32 + 1;
            if w == p.side_to_move() {
                magnitude
            } else {
                -magnitude
            }
        }
    }
}
