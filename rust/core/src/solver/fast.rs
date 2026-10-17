//! High-performance exact (proof) solver for gravity positions whose
//! bitboard fits in a single u64 — the fast path behind the facade.
//! PVS negamax with a dual-bound open-addressing transposition table
//! (full (current, mask) keys: collision-safe), mirror-canonical keys,
//! TT-move + threat-based ordering, tactical shortcuts, direct
//! score-bound tightening, the claim-even bound (Allis 1988), and an
//! optional generated opening book. Primitive, allocation-free search
//! loop: state is two u64s plus column heights; move lists live in
//! preallocated per-ply scratch arrays.
//!
//! Supported rules: gravity, non-misère, one-word bitboard, any k and
//! any direction subset. Everything else falls back to the generic
//! solver. Instances are single-use per `init`; create one per solve
//! (or per worker thread).

use super::{Abort, Certainty, SearchControl, SearchResult};
use crate::book::OpeningBook;
use crate::model::{Move, Outcome, Placement, Rules};
use crate::position::Position;
use std::sync::{Arc, OnceLock};
use std::time::Instant;

const NO_LOWER: i32 = -128;
const NO_UPPER: i32 = 127;
const DEFAULT_TT_CAPACITY: usize = 1 << 22; // ~4M entries (measured faster on hard openings; small solves still get small tables via the sizing ladder)
const MAX_PROBES: usize = 64;

fn claim_even_enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| {
        !matches!(
            std::env::var("CONNECTK_CLAIM_EVEN"),
            Ok(v) if v.eq_ignore_ascii_case("false")
        )
    })
}

fn default_capacity() -> usize {
    if let Ok(v) = std::env::var("CONNECTK_TT_CAPACITY") {
        if let Ok(n) = v.parse::<usize>() {
            if n > 0 && n.is_power_of_two() {
                return n;
            }
        }
    }
    DEFAULT_TT_CAPACITY
}

#[derive(Clone, Copy)]
struct Key {
    cur: u64,
    mask: u64,
    flipped: bool,
}

/// Outcome of a bounded (window) search.
pub struct BoundResult {
    pub value: i32,
    pub aborted: bool,
    pub nodes: u64,
}

pub struct FastExactSolver {
    max_tt_capacity: usize,
    tt_cur: Vec<u64>,
    tt_mask: Vec<u64>,
    tt_data: Vec<i32>, // packed: lower(8) | upper(8) | bestCol+1(7)
    tt_used: Vec<bool>,
    tt_index_mask: usize,

    width: i32,
    height: i32,
    connect: i32,
    cells: i32,
    stride: i32,
    infinity: i32,
    shifts: Vec<u32>,
    board_mask: u64,
    pair_lower_mask: u64,
    pair_upper_mask: u64,
    claim_even_applicable: bool,
    mirror_symmetric: bool,

    cur: u64,
    mask: u64,
    heights: Vec<i32>,
    moves: i32,
    odd_height_columns: i32,

    // Per-ply move scratch, flattened: level * width + i.
    move_cols: Vec<i32>,
    move_scores: Vec<i32>,
    prep_count: usize,
    prep_forced: bool,
    prep_forced_score: i32,
    prep_forced_col: i32,
    prep_min_possible_valid: bool,
    prep_tt_col: i32,

    root_best_col: i32,
    root_move_proven: bool,

    book: Option<Arc<OpeningBook>>,
    nodes: u64,
    control: SearchControl,
    start: Instant,
}

impl Default for FastExactSolver {
    fn default() -> Self {
        Self::new()
    }
}

impl FastExactSolver {
    pub fn new() -> FastExactSolver {
        FastExactSolver::with_capacity(default_capacity())
    }

    pub fn with_capacity(max_tt_capacity: usize) -> FastExactSolver {
        assert!(
            max_tt_capacity > 0 && max_tt_capacity.is_power_of_two(),
            "max_tt_capacity must be a positive power of two"
        );
        FastExactSolver {
            max_tt_capacity,
            tt_cur: Vec::new(),
            tt_mask: Vec::new(),
            tt_data: Vec::new(),
            tt_used: Vec::new(),
            tt_index_mask: 0,
            width: 0,
            height: 0,
            connect: 0,
            cells: 0,
            stride: 0,
            infinity: 0,
            shifts: Vec::new(),
            board_mask: 0,
            pair_lower_mask: 0,
            pair_upper_mask: 0,
            claim_even_applicable: false,
            mirror_symmetric: false,
            cur: 0,
            mask: 0,
            heights: Vec::new(),
            moves: 0,
            odd_height_columns: 0,
            move_cols: Vec::new(),
            move_scores: Vec::new(),
            prep_count: 0,
            prep_forced: false,
            prep_forced_score: 0,
            prep_forced_col: -1,
            prep_min_possible_valid: false,
            prep_tt_col: -1,
            root_best_col: -1,
            root_move_proven: true,
            book: None,
            nodes: 0,
            control: SearchControl::none(),
            start: Instant::now(),
        }
    }

    pub fn set_opening_book(&mut self, book: Option<Arc<OpeningBook>>) {
        self.book = book;
    }

    pub fn supports(rules: &Rules) -> bool {
        rules.placement == Placement::Gravity && !rules.misere && rules.word_count() == 1
    }

    /// The mirror-canonical (cur, mask, flipped) key for a position
    /// (mask compared first, as in `Position::canonical_key`).
    pub fn encode_canonical(position: &Position) -> (u64, u64, bool) {
        let rules = position.rules();
        let cur = position.current_words()[0];
        let mask = position.mask_words()[0];
        crate::book::canonical_single_flipped(cur, mask, rules)
    }

    /// Claim-even upper bound for validation tests; i32::MAX when the
    /// argument does not apply.
    pub fn claim_even_upper_bound(position: &Position) -> i32 {
        if !FastExactSolver::supports(position.rules()) || position.is_terminal() {
            return i32::MAX;
        }
        let mut solver = FastExactSolver::new();
        solver.init_state(position, SearchControl::none());
        if !solver.claim_even_applicable {
            return i32::MAX;
        }
        let bound = solver.claim_even_bound();
        if bound >= solver.infinity {
            i32::MAX
        } else {
            bound
        }
    }

    pub fn nodes_explored(&self) -> u64 {
        self.nodes
    }

    pub fn solve(&mut self, position: &Position, control: SearchControl) -> SearchResult {
        debug_assert!(FastExactSolver::supports(position.rules()));
        self.init_state(position, control);
        if position.is_terminal() {
            let score = self.terminal_score(position);
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
        match self.search_root() {
            Ok(score) => {
                let elapsed = self.elapsed_millis();
                let best_move = if self.root_best_col >= 0 {
                    Some(Move::cell(
                        self.root_best_col,
                        position.column_count(self.root_best_col),
                    ))
                } else {
                    None
                };
                let pv = self.reconstruct_variation();
                let plies = if score == 0 {
                    self.cells - position.moves_played() as i32
                } else {
                    let terminal_ply = self.cells - score.abs() + 1;
                    terminal_ply - position.moves_played() as i32
                };
                let mut explanation = format!(
                    "Proved by exhaustive principal-variation alpha-beta search (fast one-word \
                     gravity solver: primitive bitboard state, open-addressing transposition \
                     table, threat ordering{}).",
                    if self.book.is_some() {
                        ", generated opening book"
                    } else {
                        ""
                    }
                );
                if !self.root_move_proven {
                    explanation.push_str(
                        " The score is proven; identifying an optimal move ran out of \
                         budget, so the suggested move is not certified.",
                    );
                }
                SearchResult::new(
                    Certainty::ProvenExact,
                    Some(Outcome::from_score_sign(score)),
                    best_move,
                    self.root_move_proven,
                    score,
                    Some(plies),
                    plies,
                    self.nodes,
                    elapsed,
                    pv,
                    explanation,
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

    /// Fail-soft bounded search with window (alpha, beta) — the block
    /// parallel refutations are built from.
    pub fn search_bound(
        &mut self,
        position: &Position,
        alpha: i32,
        beta: i32,
        control: SearchControl,
    ) -> BoundResult {
        debug_assert!(alpha < beta);
        self.init_state(position, control);
        if position.is_terminal() {
            return BoundResult {
                value: self.terminal_score(position),
                aborted: false,
                nodes: 0,
            };
        }
        match self.negamax(alpha, beta) {
            Ok(v) => BoundResult {
                value: v,
                aborted: false,
                nodes: self.nodes,
            },
            Err(_) => BoundResult {
                value: 0,
                aborted: true,
                nodes: self.nodes,
            },
        }
    }

    fn init_state(&mut self, position: &Position, control: SearchControl) {
        let rules = position.rules();
        self.width = rules.width;
        self.height = rules.height;
        self.connect = rules.connect;
        self.cells = rules.cells();
        self.stride = rules.stride();
        self.infinity = self.cells + 2;
        self.board_mask = rules.board_mask()[0];
        self.pair_lower_mask = 0;
        self.pair_upper_mask = 0;
        for c in 0..self.width {
            for r in 0..self.height {
                if r & 1 == 0 {
                    self.pair_lower_mask |= 1u64 << (c * self.stride + r);
                } else {
                    self.pair_upper_mask |= 1u64 << (c * self.stride + r);
                }
            }
        }
        self.claim_even_applicable =
            claim_even_enabled() && self.height % 2 == 0 && self.height >= 2;
        self.mirror_symmetric = rules.mirror_symmetric();
        self.shifts = rules.shifts().to_vec();

        self.cur = position.current_words()[0];
        self.mask = position.mask_words()[0];
        self.heights = (0..self.width).map(|c| position.column_count(c)).collect();
        self.moves = position.moves_played() as i32;
        self.odd_height_columns = self.heights.iter().filter(|h| **h & 1 != 0).count() as i32;
        self.init_table(self.cells - self.moves);
        let w = self.width.max(1) as usize;
        self.move_cols = vec![0; w * (self.cells + 1) as usize];
        self.move_scores = vec![0; w * (self.cells + 1) as usize];
        self.root_best_col = -1;
        self.root_move_proven = true;
        self.nodes = 0;
        self.control = control;
        self.start = Instant::now();
    }

    fn search_root(&mut self) -> Result<i32, Abort> {
        self.nodes += 1;
        self.control.check_exact(self.nodes)?;
        let key = self.canonical_key();
        let slot = self.slot_for(key.cur, key.mask);
        self.prep_tt_col = -1;
        self.prepare();
        if self.prep_forced {
            self.root_best_col = self.prep_forced_col;
            let (s, c) = (self.prep_forced_score, self.prep_forced_col);
            self.store_exact(slot, key, s, c);
            return Ok(s);
        }
        if let Some(book) = &self.book {
            if book.covers_ply(self.moves) {
                let book_score = book.lookup_key(key.cur, key.mask);
                if book_score != crate::book::MISS {
                    let bc = book.best_col(key.cur, key.mask);
                    self.root_best_col = if bc >= 0 {
                        if key.flipped {
                            self.width - 1 - bc
                        } else {
                            bc
                        }
                    } else {
                        self.find_achieving_move(book_score, key)
                    };
                    let rc = self.root_best_col;
                    self.store_exact(slot, key, book_score, rc);
                    return Ok(book_score);
                }
            }
        }
        let level = self.moves as usize * self.width.max(1) as usize;
        let n = self.prep_count;
        let mut alpha = -self.infinity;
        let mut best = -self.infinity;
        self.root_best_col = -1;
        for i in 0..n {
            let col = self.move_cols[level + i];
            self.play(col);
            let r = (|| -> Result<i32, Abort> {
                if i == 0 {
                    Ok(-self.negamax(-self.infinity, -alpha)?)
                } else {
                    let s = -self.negamax(-alpha - 1, -alpha)?;
                    if s > alpha {
                        Ok(-self.negamax(-self.infinity, -alpha)?)
                    } else {
                        Ok(s)
                    }
                }
            })();
            self.undo(col);
            let score = r?;
            if score > best {
                best = score;
                self.root_best_col = col;
            }
            if best > alpha {
                alpha = best;
            }
        }
        let rc = self.root_best_col;
        self.store_exact(slot, key, best, rc);
        Ok(best)
    }

    fn find_achieving_move(&mut self, score: i32, key: Key) -> i32 {
        self.prep_tt_col = -1;
        self.prepare();
        if self.prep_forced {
            return self.prep_forced_col;
        }
        let level = self.moves as usize * self.width.max(1) as usize;
        let n = self.prep_count;
        let mut tt_col = -1;
        let slot = self.slot_for(key.cur, key.mask);
        if self.tt_used[slot] && self.tt_cur[slot] == key.cur && self.tt_mask[slot] == key.mask {
            let best = tt_best_col(self.tt_data[slot]);
            if best >= 0 {
                tt_col = if key.flipped {
                    self.width - 1 - best
                } else {
                    best
                };
            }
        }
        let attempt = (|| -> Result<i32, Abort> {
            if tt_col >= 0
                && self.heights[tt_col as usize] < self.height
                && self.achieves(tt_col, score)?
            {
                return Ok(tt_col);
            }
            for i in 0..n {
                let col = self.move_cols[level + i];
                if col == tt_col {
                    continue;
                }
                if self.achieves(col, score)? {
                    return Ok(col);
                }
            }
            // Safety net: full-window scan.
            let mut best_col = self.move_cols[level];
            let mut best_val = -self.infinity;
            for i in 0..n {
                let col = self.move_cols[level + i];
                self.play(col);
                let r = self.negamax(-self.infinity, self.infinity);
                self.undo(col);
                let v = -r?;
                if v > best_val {
                    best_val = v;
                    best_col = col;
                }
            }
            self.root_move_proven = best_val == score;
            Ok(best_col)
        })();
        match attempt {
            Ok(c) => c,
            Err(_) => {
                self.root_move_proven = false;
                if tt_col >= 0 && self.heights[tt_col as usize] < self.height {
                    tt_col
                } else {
                    self.move_cols[level]
                }
            }
        }
    }

    fn achieves(&mut self, col: i32, score: i32) -> Result<bool, Abort> {
        self.play(col);
        let r = self.negamax(-score, -score + 1);
        self.undo(col);
        Ok(r? <= -score)
    }

    fn negamax(&mut self, mut alpha: i32, mut beta: i32) -> Result<i32, Abort> {
        self.nodes += 1;
        self.control.check_exact(self.nodes)?;
        if self.moves > 0 && self.has_line(self.mask ^ self.cur) {
            return Ok(-(self.cells - self.moves + 1));
        }
        if self.moves == self.cells {
            return Ok(0);
        }
        let mut alpha_orig = alpha;
        let mut beta_orig = beta;
        let key = self.canonical_key();
        if let Some(book) = &self.book {
            if book.covers_ply(self.moves) {
                let book_score = book.lookup_key(key.cur, key.mask);
                if book_score != crate::book::MISS {
                    return Ok(book_score);
                }
            }
        }
        self.prep_tt_col = -1;
        let slot = self.slot_for(key.cur, key.mask);
        if self.tt_used[slot] && self.tt_cur[slot] == key.cur && self.tt_mask[slot] == key.mask {
            let data = self.tt_data[slot];
            let lower = tt_lower(data);
            let upper = tt_upper(data);
            let tt_best = tt_best_col(data);
            if tt_best >= 0 {
                self.prep_tt_col = if key.flipped {
                    self.width - 1 - tt_best
                } else {
                    tt_best
                };
            }
            if lower >= beta {
                return Ok(lower);
            }
            if upper <= alpha {
                return Ok(upper);
            }
            alpha = alpha.max(lower);
            beta = beta.min(upper);
            if alpha >= beta {
                return Ok(alpha);
            }
            alpha_orig = alpha;
            beta_orig = beta;
        }

        self.prepare();
        if self.prep_forced {
            let (s, c) = (self.prep_forced_score, self.prep_forced_col);
            self.store_exact(slot, key, s, c);
            return Ok(s);
        }
        // Score-bound tightening (see Java docs): no immediate win
        // exists, so the earliest win is in 3 plies; symmetric bound
        // when prepare() certified it.
        let empties = self.cells - self.moves;
        let max_possible = if empties >= 3 {
            self.cells - self.moves - 2
        } else {
            0
        };
        if max_possible < beta {
            beta = max_possible;
            if alpha >= beta {
                self.store_bounds(slot, key, NO_LOWER, max_possible, -1);
                return Ok(max_possible);
            }
            beta_orig = beta;
        }
        let min_valid = self.prep_min_possible_valid;
        let mut min_possible = 0;
        if min_valid {
            min_possible = if empties >= 4 {
                -(self.cells - self.moves - 3)
            } else {
                0
            };
            if min_possible > alpha {
                alpha = min_possible;
                if alpha >= beta {
                    self.store_bounds(slot, key, min_possible, NO_UPPER, -1);
                    return Ok(min_possible);
                }
                alpha_orig = alpha;
            }
        }
        if self.claim_even_applicable && beta > -self.height {
            let ce = self.claim_even_bound();
            if ce < beta {
                beta = ce;
                if alpha >= beta {
                    self.store_bounds(slot, key, NO_LOWER, ce, -1);
                    return Ok(ce);
                }
                beta_orig = beta;
            }
        }
        let level = self.moves as usize * self.width.max(1) as usize;
        let n = self.prep_count;
        let mut best = -self.infinity;
        let mut best_col = -1;
        for i in 0..n {
            let col = self.move_cols[level + i];
            self.play(col);
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
            self.undo(col);
            let score = r?;
            if score > best {
                best = score;
                best_col = col;
            }
            if best > alpha {
                alpha = best;
            }
            if alpha >= beta {
                break;
            }
        }
        if best > max_possible {
            best = max_possible;
        }
        if min_valid && best < min_possible {
            best = min_possible;
        }
        if best <= alpha_orig {
            self.store_bounds(slot, key, NO_LOWER, best, best_col);
        } else if best >= beta_orig {
            self.store_bounds(slot, key, best, NO_UPPER, best_col);
        } else {
            self.store_exact(slot, key, best, best_col);
        }
        Ok(best)
    }

    fn prepare(&mut self) {
        self.prep_forced = false;
        self.prep_min_possible_valid = false;
        let playable = self.playable_mask();
        let my_winning = self.winning_spots(self.cur) & playable;
        if my_winning != 0 {
            self.prep_forced = true;
            self.prep_forced_score = self.cells - self.moves;
            self.prep_forced_col = self.column_of(my_winning);
            return;
        }
        let opp_spots = self.winning_spots(self.mask ^ self.cur);
        let opp_playable = opp_spots & playable;
        let opp_count = opp_playable.count_ones();
        if opp_count >= 2 {
            self.prep_forced = true;
            self.prep_forced_score = -(self.cells - self.moves - 1);
            self.prep_forced_col = self.column_of(playable);
            return;
        }
        let level = self.moves as usize * self.width.max(1) as usize;
        if opp_count == 1 {
            let block_col = self.column_of(opp_playable);
            let block_idx = block_col * self.stride + self.heights[block_col as usize];
            if block_idx + 1 < 64 && (opp_spots >> (block_idx + 1)) & 1 != 0 {
                // Stacked second threat directly above the block.
                self.prep_forced = true;
                self.prep_forced_score = -(self.cells - self.moves - 1);
                self.prep_forced_col = block_col;
                return;
            }
            self.move_cols[level] = block_col;
            self.prep_count = 1;
            self.prep_min_possible_valid = true;
            return;
        }
        // Exclude moves directly below an opponent winning spot.
        self.prep_min_possible_valid = true;
        let mut n = 0usize;
        let mut first_legal_col = -1;
        for c in 0..self.width {
            if self.heights[c as usize] >= self.height {
                continue;
            }
            if first_legal_col < 0 {
                first_legal_col = c;
            }
            let idx = c * self.stride + self.heights[c as usize];
            if idx + 1 < 64 && (opp_spots >> (idx + 1)) & 1 != 0 {
                continue; // losing move
            }
            let mut s = (c.min(self.width - 1 - c) + 1) * 10;
            if c == self.prep_tt_col {
                s += 10_000_000;
            }
            if (opp_spots >> idx) & 1 != 0 {
                s += 5_000;
            }
            self.play(c);
            let terminal_after =
                self.moves == self.cells || self.has_line(self.mask ^ self.cur);
            if !terminal_after {
                s += self.winning_spots(self.mask ^ self.cur).count_ones() as i32 * 100;
                // Child TT probe for ordering (cells-moves after the move).
                if c != self.prep_tt_col && self.cells - self.moves > 12 {
                    let child = self.canonical_key();
                    let child_slot = self.slot_for(child.cur, child.mask);
                    if self.tt_used[child_slot]
                        && self.tt_cur[child_slot] == child.cur
                        && self.tt_mask[child_slot] == child.mask
                    {
                        let upper = tt_upper(self.tt_data[child_slot]);
                        if upper != NO_UPPER {
                            s += (self.cells - upper) * 30;
                        }
                    }
                }
            }
            self.undo(c);
            self.move_cols[level + n] = c;
            self.move_scores[level + n] = s;
            n += 1;
        }
        if n == 0 {
            self.prep_forced = true;
            self.prep_forced_score = -(self.cells - self.moves - 1);
            self.prep_forced_col = first_legal_col;
            return;
        }
        // Insertion sort, best first (stable, tiny n).
        for i in 1..n {
            let c0 = self.move_cols[level + i];
            let s0 = self.move_scores[level + i];
            let mut j = i as i32 - 1;
            while j >= 0 && self.move_scores[level + j as usize] < s0 {
                self.move_cols[level + (j + 1) as usize] = self.move_cols[level + j as usize];
                self.move_scores[level + (j + 1) as usize] = self.move_scores[level + j as usize];
                j -= 1;
            }
            self.move_cols[level + (j + 1) as usize] = c0;
            self.move_scores[level + (j + 1) as usize] = s0;
        }
        self.prep_count = n;
    }

    fn reconstruct_variation(&mut self) -> Vec<Move> {
        // PV-walking plays moves; snapshot and restore so the instance
        // stays consistent for bookgen/parallel reuse.
        let (save_cur, save_mask, save_heights, save_moves, save_odd) =
            (self.cur, self.mask, self.heights.clone(), self.moves, self.odd_height_columns);
        let mut pv = Vec::new();
        while self.moves < self.cells && pv.len() as i32 <= self.cells {
            if self.moves > 0 && self.has_line(self.mask ^ self.cur) {
                break;
            }
            let key = self.canonical_key();
            let slot = self.slot_for(key.cur, key.mask);
            let mut col = -1;
            if self.tt_used[slot] && self.tt_cur[slot] == key.cur && self.tt_mask[slot] == key.mask
            {
                let best = tt_best_col(self.tt_data[slot]);
                if best >= 0 {
                    col = if key.flipped {
                        self.width - 1 - best
                    } else {
                        best
                    };
                }
            }
            if col < 0 {
                let win = self.winning_spots(self.cur) & self.playable_mask();
                if win == 0 {
                    break;
                }
                col = self.column_of(win);
            }
            if col < 0 || col >= self.width || self.heights[col as usize] >= self.height {
                break;
            }
            pv.push(Move::cell(col, self.heights[col as usize]));
            self.play(col);
        }
        self.cur = save_cur;
        self.mask = save_mask;
        self.heights = save_heights;
        self.moves = save_moves;
        self.odd_height_columns = save_odd;
        pv
    }

    // ---- bitboard primitives ----

    #[inline]
    fn play(&mut self, col: i32) {
        let c = col as usize;
        let bit = 1u64 << (col * self.stride + self.heights[c]);
        self.cur ^= self.mask;
        self.mask |= bit;
        if self.heights[c] & 1 == 0 {
            self.odd_height_columns += 1;
        } else {
            self.odd_height_columns -= 1;
        }
        self.heights[c] += 1;
        self.moves += 1;
    }

    #[inline]
    fn undo(&mut self, col: i32) {
        let c = col as usize;
        self.heights[c] -= 1;
        if self.heights[c] & 1 == 0 {
            self.odd_height_columns -= 1;
        } else {
            self.odd_height_columns += 1;
        }
        self.moves -= 1;
        let bit = 1u64 << (col * self.stride + self.heights[c]);
        self.mask ^= bit;
        self.cur ^= self.mask;
    }

    fn claim_even_bound(&self) -> i32 {
        let odd_columns = self.odd_height_columns;
        if odd_columns > 1 {
            return self.infinity;
        }
        let mut odd_column = -1;
        if odd_columns == 1 {
            for c in 0..self.width {
                if self.heights[c as usize] & 1 != 0 {
                    odd_column = c;
                    break;
                }
            }
        }
        let opponent = self.mask ^ self.cur;
        let empty = self.board_mask & !self.mask;
        let claimed;
        if odd_columns == 0 {
            claimed = empty & self.pair_upper_mask;
        } else {
            let odd_col_mask =
                (((1u64 << self.height) - 1) << (odd_column * self.stride)) & self.board_mask;
            claimed = (empty & self.pair_upper_mask & !odd_col_mask)
                | (empty & self.pair_lower_mask & odd_col_mask);
            if !self.has_line(opponent | (claimed & odd_col_mask)) {
                return self.infinity;
            }
        }
        let my_future = self.cur | (empty & !claimed);
        for &shift in &self.shifts {
            let mut ends = self.line_starts(my_future, shift);
            while ends != 0 {
                let start = ends.trailing_zeros() as i32;
                ends &= ends - 1;
                let line = self.line_mask(start, shift);
                let support = self.below_mask(line) & claimed;
                if !self.has_line(opponent | support) {
                    return self.infinity; // realizable: no bound
                }
            }
        }
        let mut bound = 0;
        let opp_future = opponent | claimed;
        for &shift in &self.shifts {
            let mut ends = self.line_starts(opp_future, shift);
            while ends != 0 {
                let start = ends.trailing_zeros() as i32;
                ends &= ends - 1;
                let line = self.line_mask(start, shift);
                let empty_line = line & claimed;
                if empty_line == 0 {
                    continue;
                }
                let mut max_row = 0;
                let mut e = empty_line;
                while e != 0 {
                    let row = (e.trailing_zeros() as i32) % self.stride;
                    if row > max_row {
                        max_row = row;
                    }
                    e &= e - 1;
                }
                bound = bound.min(-(self.height - max_row));
            }
        }
        bound
    }

    #[inline]
    fn line_starts(&self, bits: u64, shift: u32) -> u64 {
        let mut x = bits;
        let mut i = 1;
        while i < self.connect && x != 0 {
            x &= x >> shift;
            i += 1;
        }
        x
    }

    #[inline]
    fn line_mask(&self, start: i32, shift: u32) -> u64 {
        let mut line = 0u64;
        for i in 0..self.connect {
            line |= 1u64 << (start + i * shift as i32);
        }
        line
    }

    fn below_mask(&self, line: u64) -> u64 {
        let mut below = 0u64;
        let mut m = line;
        while m != 0 {
            let idx = m.trailing_zeros() as i32;
            let row = idx % self.stride;
            if row > 0 {
                below |= ((1u64 << row) - 1) << (idx - row);
            }
            m &= m - 1;
        }
        (below & !line) & self.board_mask
    }

    #[inline]
    fn has_line(&self, bits: u64) -> bool {
        for &s in &self.shifts {
            let mut x = bits;
            let mut i = 1;
            while i < self.connect && x != 0 {
                x &= x >> s;
                i += 1;
            }
            if x != 0 {
                return true;
            }
        }
        false
    }

    fn winning_spots(&self, bits: u64) -> u64 {
        let mut res = 0u64;
        for &s in &self.shifts {
            for hole in 0..self.connect {
                let mut cand = self.board_mask;
                for i in 0..self.connect {
                    if i == hole {
                        continue;
                    }
                    cand &= signed_shift(bits, -(i - hole) * s as i32);
                    if cand == 0 {
                        break;
                    }
                }
                res |= cand;
            }
        }
        res & self.board_mask & !self.mask
    }

    #[inline]
    fn playable_mask(&self) -> u64 {
        let mut p = 0u64;
        for c in 0..self.width {
            let h = self.heights[c as usize];
            if h < self.height {
                p |= 1u64 << (c * self.stride + h);
            }
        }
        p
    }

    #[inline]
    fn column_of(&self, bits: u64) -> i32 {
        bits.trailing_zeros() as i32 / self.stride
    }

    #[inline]
    fn mirror_bits(&self, bits: u64) -> u64 {
        let stride = self.stride as u32;
        let col_mask: u64 = if stride >= 64 {
            u64::MAX
        } else {
            (1u64 << stride) - 1
        };
        let mut out = 0u64;
        for c in 0..self.width as u32 {
            let chunk = (bits >> (c * stride)) & col_mask;
            out |= chunk << ((self.width as u32 - 1 - c) * stride);
        }
        out
    }

    // ---- canonical key + TT ----

    fn init_table(&mut self, empties: i32) {
        let mut cap = if empties <= 14 {
            1 << 14
        } else if empties <= 18 {
            1 << 16
        } else if empties <= 22 {
            1 << 18
        } else if empties <= 26 {
            1 << 20
        } else {
            self.max_tt_capacity
        };
        cap = cap.min(self.max_tt_capacity);
        if self.tt_cur.len() != cap {
            self.tt_cur = vec![0; cap];
            self.tt_mask = vec![0; cap];
            self.tt_data = vec![0; cap];
            self.tt_used = vec![false; cap];
        } else {
            self.tt_used.iter_mut().for_each(|u| *u = false);
        }
        self.tt_index_mask = cap - 1;
    }

    #[inline]
    fn canonical_key(&self) -> Key {
        if !self.mirror_symmetric {
            return Key {
                cur: self.cur,
                mask: self.mask,
                flipped: false,
            };
        }
        let m_mask = self.mirror_bits(self.mask);
        if m_mask < self.mask {
            return Key {
                cur: self.mirror_bits(self.cur),
                mask: m_mask,
                flipped: true,
            };
        }
        if m_mask == self.mask {
            let m_cur = self.mirror_bits(self.cur);
            if m_cur < self.cur {
                return Key {
                    cur: m_cur,
                    mask: m_mask,
                    flipped: true,
                };
            }
        }
        Key {
            cur: self.cur,
            mask: self.mask,
            flipped: false,
        }
    }

    #[inline]
    fn slot_for(&self, k_cur: u64, k_mask: u64) -> usize {
        let mut h = k_cur
            .wrapping_mul(0x9E37_79B9_7F4A_7C15)
            .wrapping_add(k_mask.wrapping_mul(0xC2B2_AE3D_27D4_EB4F));
        h ^= h >> 29;
        h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
        h ^= h >> 32;
        let home = h as usize & self.tt_index_mask;
        for probe in 0..MAX_PROBES {
            let i = (home + probe) & self.tt_index_mask;
            if !self.tt_used[i] || (self.tt_cur[i] == k_cur && self.tt_mask[i] == k_mask) {
                return i;
            }
        }
        home
    }

    fn store_exact(&mut self, slot: usize, key: Key, score: i32, best_col: i32) {
        self.store_bounds(slot, key, score, score, best_col);
    }

    fn store_bounds(
        &mut self,
        slot: usize,
        key: Key,
        mut lower: i32,
        mut upper: i32,
        best_col: i32,
    ) {
        let mut canonical_col = if best_col < 0 {
            -1
        } else if key.flipped {
            self.width - 1 - best_col
        } else {
            best_col
        };
        if self.tt_used[slot] && self.tt_cur[slot] == key.cur && self.tt_mask[slot] == key.mask {
            let data = self.tt_data[slot];
            lower = lower.max(tt_lower(data));
            upper = upper.min(tt_upper(data));
            if canonical_col < 0 {
                canonical_col = tt_best_col(data);
            }
        }
        self.tt_cur[slot] = key.cur;
        self.tt_mask[slot] = key.mask;
        self.tt_data[slot] =
            (lower & 0xFF) | ((upper & 0xFF) << 8) | ((canonical_col + 1) << 16);
        self.tt_used[slot] = true;
    }

    fn elapsed_millis(&self) -> u64 {
        self.start.elapsed().as_millis() as u64
    }

    fn terminal_score(&self, p: &Position) -> i32 {
        match p.winner() {
            None => 0,
            Some(w) => {
                let magnitude = self.cells - p.moves_played() as i32 + 1;
                if w == p.side_to_move() {
                    magnitude
                } else {
                    -magnitude
                }
            }
        }
    }
}

#[inline]
fn tt_lower(data: i32) -> i32 {
    (data << 24) >> 24
}
#[inline]
fn tt_upper(data: i32) -> i32 {
    (data << 16) >> 24
}
#[inline]
fn tt_best_col(data: i32) -> i32 {
    (((data as u32) >> 16) & 0x7F) as i32 - 1
}

#[inline]
fn signed_shift(value: u64, amount: i32) -> u64 {
    if amount == 0 {
        value
    } else if !(0..64).contains(&amount) && !(0..64).contains(&-amount) {
        0
    } else if amount > 0 {
        value << amount
    } else {
        value >> (-amount)
    }
}
