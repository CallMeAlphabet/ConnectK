use super::fast::BoundResult;
use super::{AbortReason, Certainty, SearchControl, SearchResult};
use crate::book::OpeningBook;
use crate::model::{dirs, Move, Outcome, Placement, Rules};
use crate::position::Position;
use std::sync::Arc;
use std::time::Instant;

const CELLS: i32 = 42;
const INF: i32 = 44;
const NO_LOWER: i32 = -128;
const NO_UPPER: i32 = 127;
const NO_TT: u32 = u32::MAX;
const PHI: u64 = 0x9E37_79B9_7F4A_7C15;
const HOT_TT_CAPACITY: usize = 1 << 15;
const HOT_TT_SHIFT: u32 = 64 - HOT_TT_CAPACITY.trailing_zeros();

const BOARD_MASK: u64 = 0x003f_3f3f_3f3f_3f3f;
const BOTTOM_MASK: u64 = 0x0001_0101_0101_0101;
const PAIR_LOWER_MASK: u64 = 0x0015_1515_1515_1515;
const PAIR_UPPER_MASK: u64 = 0x002a_2a2a_2a2a_2a2a;
const COLUMN_MASKS: [u64; 7] = [
    0x3f,
    0x3f << 8,
    0x3f << 16,
    0x3f << 24,
    0x3f << 32,
    0x3f << 40,
    0x3f << 48,
];
const ORDER: [usize; 7] = [3, 4, 2, 5, 1, 6, 0];

const fn build_stone_bounds() -> [(i32, i32); 43] {
    let mut out = [(0, 0); 43];
    let mut moves = 0i32;
    while moves <= CELLS {
        let side_stones = moves / 2;
        let opp_stones = moves - side_stones;
        let side_needed = if 4 - side_stones > 1 {
            4 - side_stones
        } else {
            1
        };
        let side_final_ply = moves + 2 * side_needed - 1;
        let upper = if side_final_ply > CELLS {
            0
        } else {
            CELLS - side_final_ply + 1
        };
        let opp_needed = if 4 - opp_stones > 1 {
            4 - opp_stones
        } else {
            1
        };
        let opp_final_ply = moves + 2 * opp_needed;
        let lower = if opp_final_ply > CELLS {
            0
        } else {
            -(CELLS - opp_final_ply + 1)
        };
        out[moves as usize] = (lower, upper);
        moves += 1;
    }
    out
}

const STONE_BOUNDS: [(i32, i32); 43] = build_stone_bounds();

const fn build_windows() -> ([u64; 69], [u64; 69]) {
    let mut windows = [0u64; 69];
    let mut below = [0u64; 69];
    let dirs = [(1i32, 0i32), (0, 1), (1, 1), (1, -1)];
    let mut n = 0usize;
    let mut c = 0i32;
    while c < 7 {
        let mut r = 0i32;
        while r < 6 {
            let mut d = 0usize;
            while d < dirs.len() {
                let (dc, dr) = dirs[d];
                let ec = c + dc * 3;
                let er = r + dr * 3;
                if ec >= 0 && ec < 7 && er >= 0 && er < 6 {
                    let mut w = 0u64;
                    let mut b = 0u64;
                    let mut i = 0i32;
                    while i < 4 {
                        let cc = c + dc * i;
                        let rr = r + dr * i;
                        w |= 1u64 << (cc * 8 + rr);
                        let mut below_row = 0i32;
                        while below_row < rr {
                            b |= 1u64 << (cc * 8 + below_row);
                            below_row += 1;
                        }
                        i += 1;
                    }
                    windows[n] = w;
                    below[n] = b & !w & BOARD_MASK;
                    n += 1;
                }
                d += 1;
            }
            r += 1;
        }
        c += 1;
    }
    (windows, below)
}

const WINDOWS_AND_BELOW: ([u64; 69], [u64; 69]) = build_windows();
const WINDOWS: [u64; 69] = WINDOWS_AND_BELOW.0;
const BELOW_WINDOWS: [u64; 69] = WINDOWS_AND_BELOW.1;

const fn build_cell_line_counts() -> [i32; 56] {
    let mut counts = [0i32; 56];
    let mut i = 0usize;
    while i < WINDOWS.len() {
        let mut window = WINDOWS[i];
        while window != 0 {
            let bit = window.trailing_zeros() as usize;
            counts[bit] += 1;
            window &= window - 1;
        }
        i += 1;
    }
    counts
}

const CELL_LINE_COUNTS: [i32; 56] = build_cell_line_counts();

#[derive(Clone, Copy, Default)]
struct Entry {
    code: u64,
    data: u32,
    generation: u32,
}

#[derive(Clone, Copy, Default)]
struct Candidate {
    col: u8,
    bit: u64,
    score: i32,
    spots_after: u64,
    code: u64,
    flipped: bool,
    tt_data: u32,
    book_score: i32,
    double_threat: bool,
    skip: bool,
}

#[allow(clippy::large_enum_variant)]
enum Prep {
    Forced(i32, i32),
    Moves([Candidate; 7], usize),
}

#[derive(Clone, Copy)]
struct Undo {
    bit: u64,
    spots_cur: u64,
    spots_opp: u64,
    won: bool,
}

pub struct StandardSolver {
    tt: Vec<Entry>,
    hot_tt: Vec<Entry>,
    tt_mask: usize,
    tt_shift: u32,
    tt_entries: usize,
    generation: u32,
    max_tt_capacity: usize,
    persistent_tt: bool,

    cur: u64,
    mask: u64,
    moves: i32,
    spots_cur: u64,
    spots_opp: u64,
    won_by_last_move: bool,

    root_best_col: i32,
    root_move_proven: bool,
    book: Option<Arc<OpeningBook>>,
    book_min_ply: i32,
    book_max_ply: i32,
    claim_even: bool,
    nodes: u64,
    control: SearchControl,
    start: Instant,
    aborted: bool,
    abort_reason: AbortReason,
}

impl StandardSolver {
    pub fn new(max_tt_capacity: usize) -> StandardSolver {
        let max_tt_capacity = if max_tt_capacity == 0 {
            1 << 21
        } else {
            max_tt_capacity.next_power_of_two()
        };
        StandardSolver {
            tt: Vec::new(),
            hot_tt: Vec::new(),
            tt_mask: 0,
            tt_shift: 64,
            tt_entries: 0,
            generation: 0,
            max_tt_capacity,
            persistent_tt: false,
            cur: 0,
            mask: 0,
            moves: 0,
            spots_cur: 0,
            spots_opp: 0,
            won_by_last_move: false,
            root_best_col: -1,
            root_move_proven: true,
            book: None,
            book_min_ply: 1,
            book_max_ply: -1,
            claim_even: true,
            nodes: 0,
            control: SearchControl::none(),
            start: Instant::now(),
            aborted: false,
            abort_reason: AbortReason::Cancelled,
        }
    }

    pub fn set_opening_book(&mut self, book: Option<Arc<OpeningBook>>) {
        match &book {
            Some(book) => {
                self.book_min_ply = book.min_ply();
                self.book_max_ply = book.max_ply();
            }
            None => {
                self.book_min_ply = 1;
                self.book_max_ply = -1;
            }
        }
        self.book = book;
    }

    #[inline(always)]
    fn book_may_contain(&self, ply: i32) -> bool {
        ply >= self.book_min_ply && ply <= self.book_max_ply
    }

    pub fn set_persistent_tt(&mut self, persistent: bool) {
        self.persistent_tt = persistent;
    }

    pub fn supports(rules: &Rules) -> bool {
        rules.width == 7
            && rules.height == 6
            && rules.connect == 4
            && rules.placement == Placement::Gravity
            && rules.directions == dirs::ALL
            && !rules.misere
    }

    pub fn nodes_explored(&self) -> u64 {
        self.nodes
    }

    pub fn solve(&mut self, position: &Position, control: SearchControl) -> SearchResult {
        debug_assert!(StandardSolver::supports(position.rules()));
        self.init_state(position, control);
        if position.is_terminal() {
            let score = terminal_score(position);
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

        let score = self.exact_current();
        if self.aborted {
            return SearchResult::unknown(
                self.nodes,
                self.elapsed_millis(),
                format!(
                    "Exact proof aborted ({}) after {} nodes; no conclusion claimed.",
                    self.abort_reason.name(),
                    self.nodes
                ),
            );
        }
        if self.root_best_col < 0 {
            self.root_best_col = self.find_achieving_move(score);
        }
        let pv = self.reconstruct_variation();
        let elapsed = self.elapsed_millis();
        let best_move = if self.root_best_col >= 0 {
            Some(Move::cell(
                self.root_best_col,
                position.column_count(self.root_best_col),
            ))
        } else {
            None
        };
        let plies = if score == 0 {
            CELLS - position.moves_played() as i32
        } else {
            CELLS - score.abs() + 1 - position.moves_played() as i32
        };
        let mut explanation = "Proved by exhaustive principal-variation alpha-beta search \
             (specialized standard 7x6 solver: byte-column bitboard, direct dual-bound \
             transposition table, threat ordering and claim-even bounds)."
            .to_string();
        if !self.root_move_proven {
            explanation.push_str(
                " The score is proven; identifying an optimal move ran out of budget, \
                 so the suggested move is not certified.",
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

    pub fn evaluate_score(&mut self, position: &Position, control: SearchControl) -> SearchResult {
        debug_assert!(StandardSolver::supports(position.rules()));
        self.init_state(position, control);
        if position.is_terminal() {
            let score = terminal_score(position);
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
        let score = self.exact_current();
        if self.aborted {
            return SearchResult::unknown(
                self.nodes,
                self.elapsed_millis(),
                format!(
                    "Exact proof aborted ({}) after {} nodes; no conclusion claimed.",
                    self.abort_reason.name(),
                    self.nodes
                ),
            );
        }
        let best_move = if self.root_best_col >= 0 {
            Some(Move::cell(
                self.root_best_col,
                position.column_count(self.root_best_col),
            ))
        } else {
            None
        };
        let plies = if score == 0 {
            CELLS - position.moves_played() as i32
        } else {
            CELLS - score.abs() + 1 - position.moves_played() as i32
        };
        SearchResult::new(
            Certainty::ProvenExact,
            Some(Outcome::from_score_sign(score)),
            best_move,
            best_move.is_some() && self.root_move_proven,
            score,
            Some(plies),
            plies,
            self.nodes,
            self.elapsed_millis(),
            Vec::new(),
            "Exact score proved by the specialized standard 7x6 solver; best-move identification skipped.".into(),
        )
    }

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
                value: terminal_score(position),
                aborted: false,
                nodes: 0,
            };
        }
        let (code, flipped) = canonical_code(self.cur, self.mask);
        let value = self.negamax(alpha, beta, code, flipped, i32::MIN);
        BoundResult {
            value,
            aborted: self.aborted,
            nodes: self.nodes,
        }
    }

    fn init_state(&mut self, position: &Position, control: SearchControl) {
        let from_stride = position.rules().stride();
        self.cur = repack_to_stride8(position.current_words()[0], from_stride);
        self.mask = repack_to_stride8(position.mask_words()[0], from_stride);
        self.moves = position.moves_played() as i32;
        self.spots_cur = winning_spots(self.cur, self.mask);
        self.spots_opp = winning_spots(self.mask ^ self.cur, self.mask);
        self.won_by_last_move = self.moves > 0 && has_line(self.mask ^ self.cur);
        self.root_best_col = -1;
        self.root_move_proven = true;
        self.nodes = 0;
        self.control = control;
        self.claim_even = super::fast::claim_even_enabled();
        self.start = Instant::now();
        self.aborted = false;
        self.init_table(CELLS - self.moves);
    }

    fn init_table(&mut self, empties: i32) {
        let mut cap = if empties <= 22 {
            1 << 15
        } else if empties <= 26 {
            1 << 17
        } else if empties <= 30 {
            1 << 19
        } else {
            1 << 20
        };
        cap = cap.min(self.max_tt_capacity).max(1);
        if self.persistent_tt && self.generation != 0 && !self.tt.is_empty() {
            if self.hot_tt.len() != HOT_TT_CAPACITY {
                self.hot_tt = vec![Entry::default(); HOT_TT_CAPACITY];
            }
            let existing_cap = self.tt_mask + 1;
            if cap > existing_cap {
                self.grow_tt(cap);
            }
            return;
        }
        if self.tt.len() < cap {
            self.tt.resize(cap, Entry::default());
        }
        if self.hot_tt.len() != HOT_TT_CAPACITY {
            self.hot_tt = vec![Entry::default(); HOT_TT_CAPACITY];
        }
        self.tt_mask = cap - 1;
        self.tt_shift = tt_shift(cap);
        self.tt_entries = 0;
        self.generation = self.generation.wrapping_add(1);
        if self.generation == 0 {
            for e in &mut self.tt {
                e.code = 0;
                e.generation = 0;
            }
            for e in &mut self.hot_tt {
                e.code = 0;
                e.generation = 0;
            }
            self.generation = 1;
        }
    }

    #[inline]
    fn check_node(&mut self) {
        if let Err(e) = self.control.check_exact(self.nodes) {
            self.aborted = true;
            self.abort_reason = e.0;
            return;
        }
        let cap = self.tt_mask + 1;
        if self.nodes & 65_535 == 0 && cap < self.max_tt_capacity && self.tt_entries > cap / 2 {
            let new_cap = (cap * 4).min(self.max_tt_capacity);
            self.grow_tt(new_cap);
        }
    }

    fn grow_tt(&mut self, new_cap: usize) {
        let old_cap = self.tt_mask + 1;
        if new_cap <= old_cap || self.tt.is_empty() {
            return;
        }
        let mut entries = Vec::with_capacity(self.tt_entries);
        for i in 0..old_cap {
            let entry = self.tt[i];
            if entry.generation == self.generation && entry.code != 0 {
                entries.push(entry);
                self.tt[i].generation = 0;
                self.tt[i].code = 0;
            }
        }
        if self.tt.len() < new_cap {
            self.tt.resize(new_cap, Entry::default());
        }
        self.tt_mask = new_cap - 1;
        self.tt_shift = tt_shift(new_cap);
        self.tt_entries = 0;
        for entry in entries {
            let base = self.tt_index(entry.code);
            let idx = if self.tt_mask >= 1
                && self.tt[base].generation == self.generation
                && self.tt[base].code != entry.code
            {
                base + 1
            } else {
                base
            };
            if self.tt[idx].generation != self.generation {
                self.tt_entries += 1;
            }
            self.tt[idx] = entry;
        }
    }

    fn exact_current(&mut self) -> i32 {
        if self.won_by_last_move {
            return -(CELLS - self.moves + 1);
        }
        if self.moves == CELLS {
            return 0;
        }
        let (code, flipped) = canonical_code(self.cur, self.mask);
        if let Some((score, best_col)) = self.book_hit() {
            self.root_best_col = best_col;
            self.store_exact(code, flipped, score, best_col);
            return score;
        }
        let empties = CELLS - self.moves;
        let mut lower = if empties >= 2 { -(empties - 1) } else { 0 };
        let mut upper;
        match self.prepare(-1) {
            Prep::Forced(score, col) => {
                self.root_best_col = col;
                self.store_exact(code, flipped, score, col);
                return score;
            }
            Prep::Moves(_, _) => {
                if empties >= 4 {
                    lower = -(empties - 3);
                }
                if empties >= 3 {
                    upper = empties - 2;
                } else {
                    upper = 0;
                }
                let (stone_lower, stone_upper) = STONE_BOUNDS[self.moves as usize];
                lower = lower.max(stone_lower);
                upper = upper.min(stone_upper);
            }
        }
        if let Some(data) = self.tt_get(code) {
            lower = lower.max(tt_lower(data));
            upper = upper.min(tt_upper(data));
        }
        if self.book_may_contain(self.moves - 1) {
            lower = lower.max(self.book_parent_lower_bound());
        }

        let mut lower_weight = 1i64;
        let mut upper_weight = 1i64;
        let mut first_pass_nodes = 0u64;
        let mut bound_jump = false;
        let mut guess = if lower < upper {
            0.clamp(lower, upper - 1)
        } else {
            lower
        };
        while lower < upper && !self.aborted {
            let pivot = guess.clamp(lower, upper - 1);
            let nodes_before = self.nodes;
            let score = self.negamax(pivot, pivot + 1, code, flipped, i32::MIN);
            if first_pass_nodes == 0 {
                first_pass_nodes = self.nodes - nodes_before;
                bound_jump = first_pass_nodes >= 75_000;
            }
            if self.aborted {
                break;
            }
            let failed_low = score <= pivot;
            if failed_low {
                upper = self.snap_upper_bound(score);
                lower_weight += 1;
                upper_weight = 1;
            } else {
                lower = self.snap_lower_bound(score);
                lower_weight = 1;
                upper_weight += 1;
            }
            if lower < upper {
                guess = if bound_jump {
                    if failed_low {
                        upper - 1
                    } else {
                        lower
                    }
                } else if empties >= 28 {
                    (lower + upper) / 2
                } else {
                    let pivot64 = (lower as i64 * lower_weight + upper as i64 * upper_weight)
                        .div_euclid(lower_weight + upper_weight);
                    pivot64 as i32
                };
            }
        }
        lower
    }

    fn negamax(
        &mut self,
        mut alpha: i32,
        mut beta: i32,
        code: u64,
        flipped: bool,
        pre_tt: i32,
    ) -> i32 {
        if self.aborted {
            return 0;
        }
        self.nodes += 1;
        self.check_node();
        if self.aborted {
            return 0;
        }
        if self.won_by_last_move {
            return -(CELLS - self.moves + 1);
        }
        if self.moves == CELLS {
            return 0;
        }
        if self.book_may_contain(self.moves) {
            if let Some((score, _)) = self.book_hit() {
                return score;
            }
        }

        let mut alpha_orig = alpha;
        let mut beta_orig = beta;
        let mut tt_best_col = -1;
        let tt_data = if pre_tt == i32::MIN {
            self.tt_get(code)
        } else if pre_tt < 0 {
            None
        } else {
            Some(pre_tt as u32)
        };
        if let Some(data) = tt_data {
            let lower = tt_lower(data);
            let upper = tt_upper(data);
            let best = data_best_col(data);
            if best >= 0 {
                tt_best_col = if flipped { 6 - best } else { best };
            }
            if lower >= beta {
                return lower;
            }
            if upper <= alpha {
                return upper;
            }
            alpha = alpha.max(lower);
            beta = beta.min(upper);
            if alpha >= beta {
                return alpha;
            }
            alpha_orig = alpha;
            beta_orig = beta;
        }

        let parent_lower = if self.book_may_contain(self.moves - 1) {
            self.book_parent_lower_bound()
        } else {
            NO_LOWER
        };
        if parent_lower > alpha {
            alpha = parent_lower;
            if alpha >= beta {
                self.store_bounds(code, flipped, parent_lower, NO_UPPER, -1);
                return parent_lower;
            }
            alpha_orig = alpha;
        }

        let (mut candidates, n) = match self.prepare(tt_best_col) {
            Prep::Forced(score, col) => {
                self.store_exact(code, flipped, score, col);
                return score;
            }
            Prep::Moves(candidates, n) => (candidates, n),
        };

        let empties = CELLS - self.moves;
        let (stone_lower, stone_upper) = STONE_BOUNDS[self.moves as usize];
        let max_possible = (if empties >= 3 { empties - 2 } else { 0 }).min(stone_upper);
        if max_possible < beta {
            beta = max_possible;
            if alpha >= beta {
                self.store_bounds(code, flipped, NO_LOWER, max_possible, -1);
                return max_possible;
            }
            beta_orig = beta;
        }
        let min_possible = (if empties >= 4 { -(empties - 3) } else { 0 }).max(stone_lower);
        if min_possible > alpha {
            alpha = min_possible;
            if alpha >= beta {
                self.store_bounds(code, flipped, min_possible, NO_UPPER, -1);
                return min_possible;
            }
            alpha_orig = alpha;
        }

        let claim_enabled = self.claim_even;
        if beta > 0
            && if claim_enabled {
                !has_nonvertical_line(BOARD_MASK ^ (self.mask ^ self.cur))
            } else {
                !has_line(BOARD_MASK ^ (self.mask ^ self.cur))
            }
        {
            beta = 0;
            if alpha >= beta {
                self.store_bounds(code, flipped, NO_LOWER, 0, -1);
                return 0;
            }
            beta_orig = beta;
        }
        if empties <= 20 && alpha < 0 && !has_line(BOARD_MASK ^ self.cur) {
            alpha = 0;
            if alpha >= beta {
                self.store_bounds(code, flipped, 0, NO_UPPER, -1);
                return 0;
            }
            alpha_orig = alpha;
        }
        if claim_enabled && beta > -6 {
            let ce = self.snap_upper_bound(self.claim_even_bound());
            if ce < beta {
                beta = ce;
                if alpha >= beta {
                    self.store_bounds(code, flipped, NO_LOWER, ce, -1);
                    return ce;
                }
                beta_orig = beta;
            }
        }

        let mut best = -INF;
        let mut best_col = -1;
        let mut first_searched = true;
        let mut bound_best = -INF;
        let mut bound_col = -1;
        if n > 1 {
            let mut all_skipped = true;
            let mut exact_any = false;
            let mut exact_best = -INF;
            let mut exact_col = -1;
            for cand in &mut candidates[..n] {
                if cand.book_score != crate::book::MISS {
                    let score = -cand.book_score;
                    let col = cand.col as i32;
                    if score >= beta {
                        self.store_bounds(code, flipped, score, NO_UPPER, col);
                        return score;
                    }
                    cand.skip = true;
                    exact_any = true;
                    if score > exact_best {
                        exact_best = score;
                        exact_col = col;
                    }
                    if exact_best > alpha {
                        alpha = exact_best;
                    }
                    continue;
                }
                if cand.tt_data == NO_TT {
                    match self.tt_get_main(cand.code) {
                        Some(data) => cand.tt_data = data,
                        None => {
                            all_skipped = false;
                            continue;
                        }
                    }
                }
                let child_lower = tt_lower(cand.tt_data);
                let child_upper = tt_upper(cand.tt_data);
                let col = cand.col as i32;
                if child_upper <= -beta {
                    let score = -child_upper;
                    self.store_bounds(code, flipped, score, NO_UPPER, col);
                    return score;
                }
                if child_lower == child_upper && child_lower != NO_LOWER {
                    cand.skip = true;
                    exact_any = true;
                    let score = -child_lower;
                    if score > exact_best {
                        exact_best = score;
                        exact_col = col;
                    }
                    if exact_best > alpha {
                        alpha = exact_best;
                    }
                } else if child_lower != NO_LOWER && child_lower >= -alpha {
                    cand.skip = true;
                    if -child_lower > bound_best {
                        bound_best = -child_lower;
                        bound_col = col;
                    }
                } else {
                    all_skipped = false;
                }
            }
            if all_skipped {
                if exact_any && bound_best <= exact_best {
                    self.store_exact(code, flipped, exact_best, exact_col);
                    return exact_best;
                }
                let upper = self.snap_upper_bound(exact_best.max(bound_best));
                let col = if bound_best > exact_best {
                    bound_col
                } else {
                    exact_col
                };
                self.store_bounds(code, flipped, NO_LOWER, upper, col);
                return upper;
            }
            if exact_any {
                best = exact_best;
                best_col = exact_col;
                first_searched = false;
            }
            if bound_best > best {
                best = bound_best;
                best_col = bound_col;
            }
        }

        for cand in candidates.iter().copied().take(n) {
            if cand.skip {
                continue;
            }
            let child_pre_tt = if cand.tt_data == NO_TT {
                if n > 1 {
                    -1
                } else {
                    i32::MIN
                }
            } else {
                cand.tt_data as i32
            };
            let first = first_searched;
            first_searched = false;
            let undo = self.play(cand);
            let score = if first {
                -self.negamax(-beta, -alpha, cand.code, cand.flipped, child_pre_tt)
            } else {
                let s = -self.negamax(-alpha - 1, -alpha, cand.code, cand.flipped, child_pre_tt);
                if !self.aborted && s > alpha && s < beta {
                    -self.negamax(-beta, -alpha, cand.code, cand.flipped, i32::MIN)
                } else {
                    s
                }
            };
            self.undo(undo);
            if self.aborted {
                return 0;
            }
            if score > best {
                best = score;
                best_col = cand.col as i32;
            }
            if best > alpha {
                alpha = best;
            }
            if alpha >= beta {
                break;
            }
        }

        if bound_best > best {
            best = bound_best;
            best_col = bound_col;
        }
        if best > max_possible {
            best = max_possible;
        }
        if best < min_possible {
            best = min_possible;
        }
        if best <= alpha_orig {
            best = self.snap_upper_bound(best);
            self.store_bounds(code, flipped, NO_LOWER, best, best_col);
        } else if best >= beta_orig {
            best = self.snap_lower_bound(best);
            self.store_bounds(code, flipped, best, NO_UPPER, best_col);
        } else {
            self.store_exact(code, flipped, best, best_col);
        }
        best
    }

    fn prepare(&mut self, tt_best_col: i32) -> Prep {
        let playable = (self.mask + BOTTOM_MASK) & BOARD_MASK;
        let empties = CELLS - self.moves;
        let my_winning = self.spots_cur & playable;
        if my_winning != 0 {
            return Prep::Forced(empties, column_of(my_winning));
        }
        let opp_playable = self.spots_opp & playable;
        let opp_count = opp_playable.count_ones();
        if opp_count >= 2 {
            return Prep::Forced(-(empties - 1), column_of(playable));
        }
        if opp_count == 1 {
            let bit = opp_playable;
            if bit << 1 & self.spots_opp != 0 {
                return Prep::Forced(-(empties - 1), column_of(bit));
            }
            let mut out = [Candidate::default(); 7];
            out[0] = self.make_candidate(bit, tt_best_col, false);
            return Prep::Moves(out, 1);
        }

        let mut candidates = [Candidate::default(); 7];
        let mut n = 0usize;
        let mut first_legal = -1;
        for &col in &ORDER {
            let bit = playable & COLUMN_MASKS[col];
            if bit == 0 {
                continue;
            }
            if first_legal < 0 {
                first_legal = col as i32;
            }
            if bit << 1 & self.spots_opp != 0 {
                continue;
            }
            let cand = self.make_candidate(bit, tt_best_col, true);
            if cand.double_threat {
                return Prep::Forced(empties - 2, col as i32);
            }
            candidates[n] = cand;
            n += 1;
        }
        if n == 0 {
            return Prep::Forced(-(empties - 1), first_legal);
        }
        sort_candidates(&mut candidates[..n]);
        Prep::Moves(candidates, n)
    }

    #[inline(always)]
    fn make_candidate(&self, bit: u64, tt_best_col: i32, scored: bool) -> Candidate {
        let idx = bit.trailing_zeros() as i32;
        let col = idx / 8;
        let row = idx % 8;
        let occupancy_after = self.mask | bit;
        let spots_after = winning_spots(self.cur | bit, occupancy_after);
        let child_cur = self.mask ^ self.cur;
        let (code, flipped) = canonical_code(child_cur, occupancy_after);
        let tt_data = self.tt_get_hot(code).unwrap_or(NO_TT);
        if scored && tt_data == NO_TT {
            self.tt_prefetch_main(code);
        }
        let book_score = if scored && self.book_may_contain(self.moves + 1) {
            self.book_hit_state(child_cur, occupancy_after, self.moves + 1)
                .map(|(score, _)| score)
                .unwrap_or(crate::book::MISS)
        } else {
            crate::book::MISS
        };
        let mut score = 0;
        let mut double_threat = false;
        if scored {
            let playable_after = (occupancy_after + BOTTOM_MASK) & BOARD_MASK;
            let filtered = spots_after & !((self.spots_opp & !bit) << 1);
            let playable_spots = spots_after & playable_after;
            double_threat = CELLS - self.moves >= 3
                && (playable_spots.count_ones() >= 2 || playable_spots & (spots_after >> 1) != 0);
            score = (col.min(6 - col) + 1) * 40 + row * 10;
            if row == 5 {
                score += 75;
            }
            score += CELL_LINE_COUNTS[idx as usize] * 10;

            let favourable_rows = if self.moves & 1 == 0 {
                PAIR_LOWER_MASK
            } else {
                PAIR_UPPER_MASK
            };
            let favourable = filtered & favourable_rows;
            let other = filtered & !favourable_rows;
            score += favourable.count_ones() as i32 * 430 + other.count_ones() as i32 * 170;
            let immediate_weight = if CELLS - self.moves >= 25 { 700 } else { 520 };
            score += (favourable & playable_after).count_ones() as i32 * immediate_weight;
            score += (other & playable_after).count_ones() as i32 * (immediate_weight * 2 / 3);
            score += (favourable & (playable_after << 1)).count_ones() as i32 * 760;
            score += (other & (playable_after << 1)).count_ones() as i32 * 440;
            score += (favourable & (filtered << 1)).count_ones() as i32 * 760;
            score += (other & (filtered << 1)).count_ones() as i32 * 440;
            if tt_data != NO_TT && col != tt_best_col {
                let upper = tt_upper(tt_data);
                if upper != NO_UPPER {
                    score += (CELLS - upper) * 30;
                }
            }
            if book_score != crate::book::MISS {
                score = 1_000_000 - book_score * 1000;
            }
        }
        Candidate {
            col: col as u8,
            bit,
            score,
            spots_after,
            code,
            flipped,
            tt_data,
            book_score,
            double_threat,
            skip: false,
        }
    }

    #[inline]
    fn play(&mut self, cand: Candidate) -> Undo {
        let undo = Undo {
            bit: cand.bit,
            spots_cur: self.spots_cur,
            spots_opp: self.spots_opp,
            won: self.won_by_last_move,
        };
        let won = self.spots_cur & cand.bit != 0;
        let old_opp_spots = self.spots_opp;
        self.cur ^= self.mask;
        self.mask |= cand.bit;
        self.moves += 1;
        self.spots_cur = old_opp_spots & !cand.bit;
        self.spots_opp = cand.spots_after;
        self.won_by_last_move = won;
        undo
    }

    #[inline]
    fn undo(&mut self, undo: Undo) {
        self.moves -= 1;
        self.mask ^= undo.bit;
        self.cur ^= self.mask;
        self.spots_cur = undo.spots_cur;
        self.spots_opp = undo.spots_opp;
        self.won_by_last_move = undo.won;
    }

    fn play_col(&mut self, col: i32) -> Option<Undo> {
        let playable = (self.mask + BOTTOM_MASK) & BOARD_MASK;
        let bit = playable & COLUMN_MASKS[col as usize];
        if bit == 0 {
            return None;
        }
        let cand = self.make_candidate(bit, -1, false);
        Some(self.play(cand))
    }

    fn find_achieving_move(&mut self, score: i32) -> i32 {
        if self.aborted {
            self.root_move_proven = false;
            return -1;
        }
        let (code, flipped) = canonical_code(self.cur, self.mask);
        let mut tt_col = -1;
        if let Some(data) = self.tt_get(code) {
            let best = data_best_col(data);
            if best >= 0 {
                tt_col = if flipped { 6 - best } else { best };
            }
        }
        let (candidates, n) = match self.prepare(tt_col) {
            Prep::Forced(_, col) => return col,
            Prep::Moves(candidates, n) => (candidates, n),
        };
        for cand in ordered_with_first(&candidates[..n], tt_col) {
            if self.achieves(cand, score) {
                return cand.col as i32;
            }
            if self.aborted {
                self.root_move_proven = false;
                return if tt_col >= 0 {
                    tt_col
                } else {
                    candidates[0].col as i32
                };
            }
        }
        let mut best_col = candidates[0].col as i32;
        let mut best_value = -INF;
        for cand in &candidates[..n] {
            let undo = self.play(*cand);
            let child = self.exact_current();
            self.undo(undo);
            if self.aborted {
                self.root_move_proven = false;
                return best_col;
            }
            let value = -child;
            if value > best_value {
                best_value = value;
                best_col = cand.col as i32;
            }
        }
        self.root_move_proven = best_value == score;
        best_col
    }

    fn achieves(&mut self, cand: Candidate, score: i32) -> bool {
        let undo = self.play(cand);
        let pre_tt = if cand.tt_data == NO_TT {
            i32::MIN
        } else {
            cand.tt_data as i32
        };
        let value = self.negamax(-score, -score + 1, cand.code, cand.flipped, pre_tt);
        self.undo(undo);
        !self.aborted && value <= -score
    }

    fn reconstruct_variation(&mut self) -> Vec<Move> {
        let saved = (
            self.cur,
            self.mask,
            self.moves,
            self.spots_cur,
            self.spots_opp,
            self.won_by_last_move,
        );
        let mut pv = Vec::new();
        while !self.won_by_last_move && self.moves < CELLS {
            let (code, flipped) = canonical_code(self.cur, self.mask);
            let mut col = -1;
            if let Some(data) = self.tt_get(code) {
                let best = data_best_col(data);
                if best >= 0 {
                    col = if flipped { 6 - best } else { best };
                }
            }
            if col < 0 {
                let playable = (self.mask + BOTTOM_MASK) & BOARD_MASK;
                let win = self.spots_cur & playable;
                if win == 0 {
                    break;
                }
                col = column_of(win);
            }
            let playable = (self.mask + BOTTOM_MASK) & BOARD_MASK;
            let bit = playable & COLUMN_MASKS[col as usize];
            if bit == 0 {
                break;
            }
            let row = (bit.trailing_zeros() as i32) % 8;
            pv.push(Move::cell(col, row));
            if self.play_col(col).is_none() {
                break;
            }
        }
        self.cur = saved.0;
        self.mask = saved.1;
        self.moves = saved.2;
        self.spots_cur = saved.3;
        self.spots_opp = saved.4;
        self.won_by_last_move = saved.5;
        pv
    }

    fn claim_even_bound(&self) -> i32 {
        let odd_bits = ((self.mask + BOTTOM_MASK) & BOARD_MASK) & PAIR_UPPER_MASK;
        let odd_columns = odd_bits.count_ones();
        if odd_columns > 1 {
            return INF;
        }
        let odd_column = if odd_columns == 1 {
            column_of(odd_bits)
        } else {
            -1
        };
        let opponent = self.mask ^ self.cur;
        let empty = BOARD_MASK & !self.mask;
        let claimed = if odd_columns == 0 {
            empty & PAIR_UPPER_MASK
        } else {
            let odd_mask = COLUMN_MASKS[odd_column as usize];
            let claimed =
                (empty & PAIR_UPPER_MASK & !odd_mask) | (empty & PAIR_LOWER_MASK & odd_mask);
            if !has_line(opponent | (claimed & odd_mask)) {
                return INF;
            }
            claimed
        };
        let my_future = self.cur | (empty & !claimed);
        for i in 0..WINDOWS.len() {
            let window = WINDOWS[i];
            if window & !my_future == 0 {
                let support = BELOW_WINDOWS[i] & claimed;
                if !has_line(opponent | support) {
                    return INF;
                }
            }
        }
        let mut bound = 0;
        let opp_future = opponent | claimed;
        for window in WINDOWS {
            if window & !opp_future != 0 {
                continue;
            }
            let mut e = window & claimed;
            if e == 0 {
                continue;
            }
            let mut max_row = 0;
            while e != 0 {
                let row = (e.trailing_zeros() as i32) % 8;
                if row > max_row {
                    max_row = row;
                }
                e &= e - 1;
            }
            bound = bound.min(-(6 - max_row));
        }
        bound
    }

    fn book_hit(&self) -> Option<(i32, i32)> {
        self.book_hit_state(self.cur, self.mask, self.moves)
    }

    fn book_parent_lower_bound(&self) -> i32 {
        let Some(book) = self.book.as_ref() else {
            return NO_LOWER;
        };
        if self.moves == 0 || !book.may_contain_ply(self.moves - 1) {
            return NO_LOWER;
        }
        let opponent = self.mask ^ self.cur;
        let mut lower = NO_LOWER;
        for col in 0..7 {
            let column = (self.mask >> (col * 8)) & 0x3f;
            if column == 0 {
                continue;
            }
            let height = column.count_ones() as i32;
            let bit = 1u64 << (col * 8 + height - 1);
            if opponent & bit == 0 {
                continue;
            }
            if let Some((parent_score, _)) =
                self.book_hit_state(opponent ^ bit, self.mask ^ bit, self.moves - 1)
            {
                lower = lower.max(-parent_score);
            }
        }
        lower
    }

    #[inline]
    fn book_hit_state(&self, cur: u64, mask: u64, moves: i32) -> Option<(i32, i32)> {
        let book = self.book.as_ref()?;
        if !book.may_contain_ply(moves) {
            return None;
        }
        let cur7 = pack_stride7(cur);
        let mask7 = pack_stride7(mask);
        let (cc, mm, flipped) = canonical_pair7(cur7, mask7);
        let (score, best) = book.probe_entry(cc, mm)?;
        let best = if best < 0 {
            -1
        } else if flipped {
            6 - best
        } else {
            best
        };
        Some((score, best))
    }

    #[inline]
    fn tt_index(&self, code: u64) -> usize {
        (code.wrapping_mul(PHI) >> self.tt_shift) as usize & (self.tt_mask & !1usize)
    }

    #[inline(always)]
    fn tt_prefetch_main(&self, code: u64) {
        #[cfg(not(target_arch = "x86_64"))]
        let _ = code;
        #[cfg(target_arch = "x86_64")]
        if !self.tt.is_empty() {
            let idx = self.tt_index(code);

            unsafe {
                core::arch::x86_64::_mm_prefetch(
                    self.tt.as_ptr().add(idx).cast::<i8>(),
                    core::arch::x86_64::_MM_HINT_T1,
                );
            }
        }
    }

    #[inline(always)]
    fn tt_get_hot(&self, code: u64) -> Option<u32> {
        if self.hot_tt.is_empty() {
            return None;
        }

        let e = unsafe { self.hot_tt.get_unchecked(hot_tt_index(code)) };
        if e.code == code && e.generation == self.generation {
            Some(e.data)
        } else {
            None
        }
    }

    #[inline(always)]
    fn tt_get_main(&self, code: u64) -> Option<u32> {
        if self.tt.is_empty() {
            return None;
        }
        let base = self.tt_index(code);

        let e = unsafe { self.tt.get_unchecked(base) };
        if e.code == code && e.generation == self.generation {
            return Some(e.data);
        }
        if self.tt_mask >= 1 {
            let e = unsafe { self.tt.get_unchecked(base + 1) };
            if e.code == code && e.generation == self.generation {
                return Some(e.data);
            }
        }
        None
    }

    #[inline(always)]
    fn tt_get(&self, code: u64) -> Option<u32> {
        if let Some(data) = self.tt_get_hot(code) {
            return Some(data);
        }
        self.tt_get_main(code)
    }

    fn store_exact(&mut self, code: u64, flipped: bool, score: i32, best_col: i32) {
        self.store_bounds(code, flipped, score, score, best_col);
    }

    fn store_bounds(
        &mut self,
        code: u64,
        flipped: bool,
        mut lower: i32,
        mut upper: i32,
        best_col: i32,
    ) {
        if self.tt.is_empty() {
            return;
        }
        lower = self.snap_lower_bound(lower);
        upper = self.snap_upper_bound(upper);
        let mut canonical_best = if best_col < 0 {
            -1
        } else if flipped {
            6 - best_col
        } else {
            best_col
        };
        let base = self.tt_index(code);

        let old0 = unsafe { *self.tt.get_unchecked(base) };
        let (idx, old) = if self.tt_mask >= 1 {
            let old1 = unsafe { *self.tt.get_unchecked(base + 1) };
            if old0.code == code && old0.generation == self.generation {
                (base, old0)
            } else if old1.code == code && old1.generation == self.generation {
                (base + 1, old1)
            } else if old0.generation != self.generation {
                (base, old0)
            } else if old1.generation != self.generation {
                (base + 1, old1)
            } else if (code >> 17) & 1 == 0 {
                (base, old0)
            } else {
                (base + 1, old1)
            }
        } else {
            (base, old0)
        };
        if old.generation != self.generation {
            self.tt_entries += 1;
        }
        let old_data = if old.code == code && old.generation == self.generation {
            Some(old.data)
        } else if self.tt_mask + 1 != HOT_TT_CAPACITY && !self.hot_tt.is_empty() {
            let hot = unsafe { *self.hot_tt.get_unchecked(hot_tt_index(code)) };
            if hot.code == code && hot.generation == self.generation {
                Some(hot.data)
            } else {
                None
            }
        } else {
            None
        };
        if let Some(data) = old_data {
            lower = lower.max(tt_lower(data));
            upper = upper.min(tt_upper(data));
            if canonical_best < 0 {
                canonical_best = data_best_col(data);
            }
        }
        let entry = Entry {
            code,
            data: pack_data(lower, upper, canonical_best),
            generation: self.generation,
        };

        unsafe {
            *self.tt.get_unchecked_mut(idx) = entry;
        }
        if !self.hot_tt.is_empty() {
            let hot_idx = hot_tt_index(code);
            unsafe {
                *self.hot_tt.get_unchecked_mut(hot_idx) = entry;
            }
        }
    }

    #[inline]
    fn snap_lower_bound(&self, bound: i32) -> i32 {
        if bound == NO_LOWER || bound == 0 || score_feasible(bound, CELLS - self.moves) {
            bound
        } else {
            bound + 1
        }
    }

    #[inline]
    fn snap_upper_bound(&self, bound: i32) -> i32 {
        if bound == NO_UPPER || bound == 0 || score_feasible(bound, CELLS - self.moves) {
            bound
        } else {
            bound - 1
        }
    }

    fn elapsed_millis(&self) -> u64 {
        self.start.elapsed().as_millis() as u64
    }
}

fn ordered_with_first(candidates: &[Candidate], first_col: i32) -> Vec<Candidate> {
    let mut out = Vec::with_capacity(candidates.len());
    if first_col >= 0 {
        for cand in candidates {
            if cand.col as i32 == first_col {
                out.push(*cand);
                break;
            }
        }
    }
    for cand in candidates {
        if cand.col as i32 != first_col {
            out.push(*cand);
        }
    }
    out
}

fn sort_candidates(candidates: &mut [Candidate]) {
    for i in 1..candidates.len() {
        let cand = candidates[i];
        let mut j = i;
        while j > 0 && candidates[j - 1].score < cand.score {
            candidates[j] = candidates[j - 1];
            j -= 1;
        }
        candidates[j] = cand;
    }
}

#[inline]
fn score_feasible(bound: i32, empties: i32) -> bool {
    if bound > 0 {
        bound & 1 == empties & 1
    } else if bound < 0 {
        (-bound) & 1 != empties & 1
    } else {
        true
    }
}

#[inline]
fn tt_shift(cap: usize) -> u32 {
    if cap <= 1 {
        63
    } else {
        64 - cap.trailing_zeros()
    }
}

#[inline]
fn hot_tt_index(code: u64) -> usize {
    (code.wrapping_mul(PHI) >> HOT_TT_SHIFT) as usize
}

#[inline]
fn pack_data(lower: i32, upper: i32, best_col: i32) -> u32 {
    ((lower + 128) as u32 & 0xff)
        | (((upper + 128) as u32 & 0xff) << 8)
        | (((best_col + 1) as u32 & 0xff) << 16)
}

#[inline]
fn tt_lower(data: u32) -> i32 {
    (data & 0xff) as i32 - 128
}

#[inline]
fn tt_upper(data: u32) -> i32 {
    ((data >> 8) & 0xff) as i32 - 128
}

#[inline]
fn data_best_col(data: u32) -> i32 {
    ((data >> 16) & 0xff) as i32 - 1
}

#[inline]
fn canonical_code(cur: u64, mask: u64) -> (u64, bool) {
    let code = cur.wrapping_add(mask).wrapping_add(BOTTOM_MASK);
    let mirrored = code.swap_bytes() >> 8;
    if mirrored < code {
        (mirrored, true)
    } else {
        (code, false)
    }
}

#[inline]
fn column_of(bits: u64) -> i32 {
    (bits.trailing_zeros() as i32) / 8
}

#[inline]
fn has_line(p: u64) -> bool {
    let m = p & (p >> 1);
    if m & (m >> 2) != 0 {
        return true;
    }
    has_nonvertical_line(p)
}

#[inline]
fn has_nonvertical_line(p: u64) -> bool {
    let m = p & (p >> 8);
    if m & (m >> 16) != 0 {
        return true;
    }
    let m = p & (p >> 7);
    if m & (m >> 14) != 0 {
        return true;
    }
    let m = p & (p >> 9);
    m & (m >> 18) != 0
}

#[inline]
fn winning_spots(p: u64, mask: u64) -> u64 {
    let mut r = (p << 1) & (p << 2) & (p << 3);
    macro_rules! add_dir {
        ($s:literal) => {{
            let mut q = (p << $s) & (p << (2 * $s));
            r |= q & (p << (3 * $s));
            r |= q & (p >> $s);
            q >>= 3 * $s;
            r |= q & (p << $s);
            r |= q & (p >> (3 * $s));
        }};
    }
    add_dir!(8);
    add_dir!(7);
    add_dir!(9);
    r & BOARD_MASK & !mask
}

#[inline]
fn repack_to_stride8(word: u64, from_stride: i32) -> u64 {
    if from_stride == 8 {
        return word & BOARD_MASK;
    }
    let mut out = 0u64;
    for c in 0..7 {
        let chunk = (word >> (c * from_stride)) & 0x3f;
        out |= chunk << (c * 8);
    }
    out
}

#[inline]
fn pack_stride7(word: u64) -> u64 {
    let mut out = 0u64;
    for c in 0..7 {
        let chunk = (word >> (c * 8)) & 0x3f;
        out |= chunk << (c * 7);
    }
    out
}

#[inline]
fn mirror_stride7(word: u64) -> u64 {
    let mut out = 0u64;
    for c in 0..7 {
        let chunk = (word >> (c * 7)) & 0x3f;
        out |= chunk << ((6 - c) * 7);
    }
    out
}

#[inline]
fn canonical_pair7(cur: u64, mask: u64) -> (u64, u64, bool) {
    let mm = mirror_stride7(mask);
    if mm > mask {
        return (cur, mask, false);
    }
    let mc = mirror_stride7(cur);
    if mm == mask && mc >= cur {
        return (cur, mask, false);
    }
    (mc, mm, true)
}

fn terminal_score(position: &Position) -> i32 {
    match position.winner() {
        None => 0,
        Some(w) => {
            let magnitude = CELLS - position.moves_played() as i32 + 1;
            if w == position.side_to_move() {
                magnitude
            } else {
                -magnitude
            }
        }
    }
}
