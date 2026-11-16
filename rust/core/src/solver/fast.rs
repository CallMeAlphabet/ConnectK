use super::{stone_count_bounds, Abort, Certainty, SearchControl, SearchResult};
use crate::book::OpeningBook;
use crate::model::{Move, Outcome, Placement, Rules};
use crate::position::Position;
use std::sync::{Arc, OnceLock};
use std::time::Instant;

const NO_LOWER: i32 = -128;
const NO_UPPER: i32 = 127;
const DEFAULT_TT_CAPACITY: usize = 1 << 21;
const MAX_PROBES: usize = 4;
const HOT_TT_CAPACITY: usize = 1 << 15;
const HOT_TT_SHIFT: u32 = 64 - HOT_TT_CAPACITY.trailing_zeros();

pub(crate) fn claim_even_enabled() -> bool {
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
struct TtKey {
    code: u64,
    flipped: bool,
}

#[derive(Clone, Copy)]
struct BookKey {
    cur: u64,
    mask: u64,
    flipped: bool,
}

pub struct BoundResult {
    pub value: i32,
    pub aborted: bool,
    pub nodes: u64,
}

pub struct FastExactSolver {
    max_tt_capacity: usize,
    tt_code: Vec<u64>,
    tt_data: Vec<i32>,
    tt_index_mask: usize,
    tt_shift: u32,
    tt_touched: Vec<usize>,
    tt_cap: usize,
    hot_code: Vec<u64>,
    hot_data: Vec<i32>,
    hot_touched: Vec<usize>,

    width: i32,
    height: i32,
    connect: i32,
    cells: i32,
    stride: i32,
    book_stride: i32,
    infinity: i32,
    shifts: Vec<u32>,
    board_mask: u64,
    bottom_mask: u64,
    pair_lower_mask: u64,
    pair_upper_mask: u64,
    claim_even_applicable: bool,
    mirror_symmetric: bool,
    geom_key: (i32, i32, i32, i32, i32),

    cur: u64,
    mask: u64,
    heights: Vec<i32>,
    moves: i32,
    odd_height_columns: i32,

    windows_flat: Vec<u64>,
    window_offsets: Vec<usize>,
    spots_cur: u64,
    spots_opp: u64,
    playable: u64,
    spot_stack_cur: Vec<u64>,
    spot_stack_opp: Vec<u64>,
    won_by_last_move: bool,
    won_stack: Vec<bool>,

    move_cols: Vec<i32>,
    move_scores: Vec<i32>,
    move_keys: Vec<u64>,
    move_flipped: Vec<bool>,
    move_slots: Vec<i32>,
    move_hot_data: Vec<i32>,
    move_skip: Vec<bool>,
    move_created_spots: Vec<u64>,
    move_created_won: Vec<bool>,
    move_spots_ready: Vec<bool>,
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
            tt_code: Vec::new(),
            tt_data: Vec::new(),
            tt_index_mask: 0,
            tt_shift: 64,
            tt_touched: Vec::new(),
            tt_cap: 0,
            hot_code: Vec::new(),
            hot_data: Vec::new(),
            hot_touched: Vec::new(),
            width: 0,
            height: 0,
            connect: 0,
            cells: 0,
            stride: 0,
            book_stride: 0,
            infinity: 0,
            shifts: Vec::new(),
            board_mask: 0,
            bottom_mask: 0,
            pair_lower_mask: 0,
            pair_upper_mask: 0,
            claim_even_applicable: false,
            mirror_symmetric: false,
            geom_key: (0, 0, 0, 0, 0),
            cur: 0,
            mask: 0,
            heights: Vec::new(),
            moves: 0,
            odd_height_columns: 0,
            windows_flat: Vec::new(),
            window_offsets: Vec::new(),
            spots_cur: 0,
            spots_opp: 0,
            playable: 0,
            spot_stack_cur: Vec::new(),
            spot_stack_opp: Vec::new(),
            won_by_last_move: false,
            won_stack: Vec::new(),
            move_cols: Vec::new(),
            move_scores: Vec::new(),
            move_keys: Vec::new(),
            move_flipped: Vec::new(),
            move_slots: Vec::new(),
            move_hot_data: Vec::new(),
            move_skip: Vec::new(),
            move_created_spots: Vec::new(),
            move_created_won: Vec::new(),
            move_spots_ready: Vec::new(),
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
        rules.placement == Placement::Gravity
            && !rules.misere
            && rules.connect >= 2
            && rules.connect <= rules.max_line()
            && rules.word_count() == 1
    }

    pub fn encode_canonical(position: &Position) -> (u64, u64, bool) {
        let rules = position.rules();
        let cur = position.current_words()[0];
        let mask = position.mask_words()[0];
        crate::book::canonical_single_flipped(cur, mask, rules)
    }

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
        match self.negamax(alpha, beta, None) {
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
        self.book_stride = rules.stride();

        self.stride = if self.height == 6 && self.width <= 8 {
            8
        } else {
            self.book_stride
        };
        self.infinity = self.cells + 2;
        let geom_key = (
            self.width,
            self.height,
            self.connect,
            self.stride,
            rules.directions as i32,
        );
        let rebuild_geom = self.geom_key != geom_key;
        self.geom_key = geom_key;
        self.board_mask = if rebuild_geom { 0 } else { self.board_mask };
        self.bottom_mask = 0;
        self.pair_lower_mask = 0;
        self.pair_upper_mask = 0;
        for c in 0..self.width {
            for r in 0..self.height {
                let bit = 1u64 << (c * self.stride + r);
                self.board_mask |= bit;
                if r == 0 {
                    self.bottom_mask |= bit;
                }
                if r & 1 == 0 {
                    self.pair_lower_mask |= bit;
                } else {
                    self.pair_upper_mask |= bit;
                }
            }
        }
        self.claim_even_applicable =
            claim_even_enabled() && self.height % 2 == 0 && self.height >= 2;
        self.mirror_symmetric = rules.mirror_symmetric();
        if rebuild_geom {
            self.shifts = rules
                .dir_steps()
                .iter()
                .map(|&(dc, dr)| (dc * self.stride + dr) as u32)
                .collect();
        }

        if rebuild_geom {
            let layout_cells = (self.width * self.stride).max(1) as usize;
            let mut windows_by_cell = vec![Vec::new(); layout_cells];
            for c in 0..self.width {
                for r in 0..self.height {
                    let idx = (c * self.stride + r) as usize;
                    for &(dc, dr) in rules.dir_steps() {
                        for offset in 0..self.connect {
                            let sc = c - dc * offset;
                            let sr = r - dr * offset;
                            let ec = sc + dc * (self.connect - 1);
                            let er = sr + dr * (self.connect - 1);
                            if sc < 0
                                || sr < 0
                                || ec < 0
                                || er < 0
                                || sc >= self.width
                                || sr >= self.height
                                || ec >= self.width
                                || er >= self.height
                            {
                                continue;
                            }
                            let mut window = 0u64;
                            for i in 0..self.connect {
                                window |= 1u64 << ((sc + dc * i) * self.stride + (sr + dr * i));
                            }
                            windows_by_cell[idx].push(window);
                        }
                    }
                }
            }

            self.window_offsets = Vec::with_capacity(layout_cells + 1);
            self.windows_flat = Vec::new();
            for windows in &windows_by_cell {
                self.window_offsets.push(self.windows_flat.len());
                self.windows_flat.extend_from_slice(windows);
            }
            self.window_offsets.push(self.windows_flat.len());
        }

        self.cur = repack_word(
            position.current_words()[0],
            self.book_stride,
            self.stride,
            self.width,
            self.height,
        );
        self.mask = repack_word(
            position.mask_words()[0],
            self.book_stride,
            self.stride,
            self.width,
            self.height,
        );
        if self.heights.len() != self.width as usize {
            self.heights = vec![0; self.width as usize];
        }
        for c in 0..self.width {
            self.heights[c as usize] = position.column_count(c);
        }
        self.moves = position.moves_played() as i32;
        self.odd_height_columns = self.heights.iter().filter(|h| **h & 1 != 0).count() as i32;
        self.playable = self.playable_mask();
        self.spots_cur = self.winning_spots(self.cur);
        self.spots_opp = self.winning_spots(self.mask ^ self.cur);
        let stack_len = (self.cells + 1).max(1) as usize;
        if self.spot_stack_cur.len() != stack_len {
            self.spot_stack_cur = vec![0; stack_len];
            self.spot_stack_opp = vec![0; stack_len];
        }
        self.won_by_last_move = self.moves > 0 && self.has_line(self.mask ^ self.cur);
        if self.won_stack.len() != stack_len {
            self.won_stack = vec![false; stack_len];
        }
        self.init_table(self.cells - self.moves);
        let w = self.width.max(1) as usize;
        let scratch_len = w * (self.cells + 1) as usize;
        if self.move_cols.len() != scratch_len {
            self.move_cols = vec![0; scratch_len];
            self.move_scores = vec![0; scratch_len];
            self.move_keys = vec![0; scratch_len];
            self.move_flipped = vec![false; scratch_len];
            self.move_slots = vec![-1; scratch_len];
            self.move_hot_data = vec![-1; scratch_len];
            self.move_skip = vec![false; scratch_len];
            self.move_created_spots = vec![0; scratch_len];
            self.move_created_won = vec![false; scratch_len];
            self.move_spots_ready = vec![false; scratch_len];
        }
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
        let slot = self.slot_for(key.code);
        self.prep_tt_col = -1;
        self.prepare();
        if self.prep_forced {
            self.root_best_col = self.prep_forced_col;
            let (s, c) = (self.prep_forced_score, self.prep_forced_col);
            self.store_exact(slot, key, s, c);
            return Ok(s);
        }
        if let Some(book) = &self.book {
            if book.may_contain_ply(self.moves) {
                let book_key = self.book_canonical_key();
                let book_score = book.lookup_key(book_key.cur, book_key.mask);
                if book_score != crate::book::MISS {
                    let bc = book.best_col(book_key.cur, book_key.mask);
                    self.root_best_col = if bc >= 0 {
                        if book_key.flipped {
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
        let empties = self.cells - self.moves;
        let mut lower = if self.prep_min_possible_valid && empties >= 4 {
            -(empties - 3)
        } else if empties >= 2 {
            -(empties - 1)
        } else {
            0
        };
        let mut upper = if empties >= 3 { empties - 2 } else { 0 };
        let (stone_lower, stone_upper) = stone_count_bounds(self.cells, self.moves, self.connect);
        lower = lower.max(stone_lower);
        upper = upper.min(stone_upper);

        if self.tt_code[slot] == key.code {
            let data = self.tt_data[slot];
            lower = lower.max(tt_lower(data));
            upper = upper.min(tt_upper(data));
        }
        let mut lower_weight = 1i64;
        let mut upper_weight = 1i64;
        while lower < upper {
            let pivot64 = (lower as i64 * lower_weight + upper as i64 * upper_weight)
                .div_euclid(lower_weight + upper_weight);
            let pivot = (pivot64 as i32).clamp(lower, upper - 1);
            let score = self.negamax(pivot, pivot + 1, None)?;
            if score <= pivot {
                upper = self.snap_upper_bound(score);
                lower_weight += 1;
                upper_weight = 1;
            } else {
                lower = self.snap_lower_bound(score);
                lower_weight = 1;
                upper_weight += 1;
            }
            if lower < upper
                && self.tt_cap < self.max_tt_capacity
                && self.nodes > 2_000_000
                && self.tt_touched.len() > self.tt_cap / 2
            {
                self.grow_tt(self.max_tt_capacity);
            }
        }
        let best = lower;
        self.root_best_col = self.find_achieving_move(best, key);
        let rc = self.root_best_col;
        let slot = self.slot_for(key.code);
        self.store_exact(slot, key, best, rc);
        Ok(best)
    }

    fn find_achieving_move(&mut self, score: i32, key: TtKey) -> i32 {
        self.prep_tt_col = -1;
        self.prepare();
        if self.prep_forced {
            return self.prep_forced_col;
        }
        let level = self.moves as usize * self.width.max(1) as usize;
        let n = self.prep_count;
        let mut tt_col = -1;
        let slot = self.slot_for(key.code);
        if self.tt_code[slot] == key.code {
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

            let mut best_col = self.move_cols[level];
            let mut best_val = -self.infinity;
            for i in 0..n {
                let col = self.move_cols[level + i];
                self.play(col);
                let r = self.negamax(-self.infinity, self.infinity, None);
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
        let r = self.negamax(-score, -score + 1, None);
        self.undo(col);
        Ok(r? <= -score)
    }

    fn negamax(
        &mut self,
        mut alpha: i32,
        mut beta: i32,
        precomputed: Option<(TtKey, Option<i32>)>,
    ) -> Result<i32, Abort> {
        self.nodes += 1;
        self.control.check_exact(self.nodes)?;
        if self.won_by_last_move {
            return Ok(-(self.cells - self.moves + 1));
        }
        if self.moves == self.cells {
            return Ok(0);
        }
        let mut alpha_orig = alpha;
        let mut beta_orig = beta;
        let key = match precomputed {
            Some(pair) => pair.0,
            None => self.canonical_key(),
        };
        if let Some(book) = &self.book {
            if book.may_contain_ply(self.moves) {
                let book_key = self.book_canonical_key();
                let book_score = book.lookup_key(book_key.cur, book_key.mask);
                if book_score != crate::book::MISS {
                    return Ok(book_score);
                }
            }
        }
        self.prep_tt_col = -1;
        let hot_data = match precomputed {
            Some((_, hot)) => hot,
            None => self.hot_get(key.code),
        };
        if let Some(data) = hot_data {
            let lower = tt_lower(data);
            let upper = tt_upper(data);
            if lower >= beta {
                return Ok(lower);
            }
            if upper <= alpha {
                return Ok(upper);
            }
        }
        let slot = self.slot_for(key.code);
        let tt_data = hot_data.or_else(|| {
            if self.tt_code[slot] == key.code {
                Some(self.tt_data[slot])
            } else {
                None
            }
        });
        if let Some(data) = tt_data {
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

        let empties = self.cells - self.moves;
        let (stone_lower, stone_upper) = stone_count_bounds(self.cells, self.moves, self.connect);
        let max_possible = (if empties >= 3 {
            self.cells - self.moves - 2
        } else {
            0
        })
        .min(stone_upper);
        if max_possible < beta {
            beta = max_possible;
            if alpha >= beta {
                self.store_bounds(slot, key, NO_LOWER, max_possible, -1);
                return Ok(max_possible);
            }
            beta_orig = beta;
        }
        let min_valid = self.prep_min_possible_valid;
        let mut min_possible = stone_lower;
        if min_valid {
            min_possible = min_possible.max(if empties >= 4 {
                -(self.cells - self.moves - 3)
            } else {
                0
            });
        }
        if min_possible > alpha {
            alpha = min_possible;
            if alpha >= beta {
                self.store_bounds(slot, key, min_possible, NO_UPPER, -1);
                return Ok(min_possible);
            }
            alpha_orig = alpha;
        }
        let opponent = self.mask ^ self.cur;
        let cannot_win = if self.claim_even_applicable {
            !self.has_nonvertical_line(self.board_mask ^ opponent)
        } else {
            !self.has_line(self.board_mask ^ opponent)
        };
        if beta > 0 && cannot_win {
            beta = 0;
            if alpha >= beta {
                self.store_bounds(slot, key, NO_LOWER, 0, -1);
                return Ok(0);
            }
            beta_orig = beta;
        }
        if empties <= 20 && alpha < 0 && !self.has_line(self.board_mask ^ self.cur) {
            alpha = 0;
            if alpha >= beta {
                self.store_bounds(slot, key, 0, NO_UPPER, -1);
                return Ok(0);
            }
            alpha_orig = alpha;
        }
        if self.claim_even_applicable && beta > -self.height {
            let ce = self.snap_upper_bound(self.claim_even_bound());
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
        let etc = n > 1;
        let mut first_searched = true;
        if etc {
            for i in 0..n {
                self.move_skip[level + i] = false;
                if self.move_hot_data[level + i] < 0 {
                    let code = self.move_keys[level + i];
                    if code != 0 {
                        let child_slot = self.slot_for(code);
                        if self.tt_code[child_slot] == code {
                            self.move_hot_data[level + i] = self.tt_data[child_slot];
                        }
                    }
                }
            }
            let mut all_skipped = true;
            let mut exact_any = false;
            let mut exact_best = -self.infinity;
            let mut exact_col = -1;
            let mut bound_best = -self.infinity;
            let mut bound_col = -1;
            for i in 0..n {
                let data = self.move_hot_data[level + i];
                if data < 0 {
                    all_skipped = false;
                    continue;
                }
                let child_lower = tt_lower(data);
                let child_upper = tt_upper(data);
                let col = self.move_cols[level + i];
                if child_upper <= -beta {
                    let score = -child_upper;
                    self.store_bounds(slot, key, score, NO_UPPER, col);
                    return Ok(score);
                }
                if child_lower == child_upper {
                    self.move_skip[level + i] = true;
                    exact_any = true;
                    let score = -child_lower;
                    if score > exact_best {
                        exact_best = score;
                        exact_col = col;
                    }
                    if exact_best > alpha {
                        alpha = exact_best;
                    }
                } else if child_lower >= -alpha {
                    self.move_skip[level + i] = true;
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
                    self.store_exact(slot, key, exact_best, exact_col);
                    return Ok(exact_best);
                }
                let upper = self.snap_upper_bound(exact_best.max(bound_best));
                let col = if bound_best > exact_best {
                    bound_col
                } else {
                    exact_col
                };
                self.store_bounds(slot, key, NO_LOWER, upper, col);
                return Ok(upper);
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
        for _picked in 0..n {
            let mut i = usize::MAX;
            let mut picked_score = i32::MIN;
            for cand in 0..n {
                if etc && self.move_skip[level + cand] {
                    continue;
                }
                let score = self.move_scores[level + cand];
                if score > picked_score {
                    picked_score = score;
                    i = cand;
                }
            }
            if i == usize::MAX {
                break;
            }
            if etc {
                self.move_skip[level + i] = true;
            }
            let col = self.move_cols[level + i];
            let precomputed = if self.move_spots_ready[level + i] {
                let hot = self.move_hot_data[level + i];
                Some((
                    TtKey {
                        code: self.move_keys[level + i],
                        flipped: self.move_flipped[level + i],
                    },
                    if hot >= 0 { Some(hot) } else { None },
                ))
            } else {
                None
            };
            let first = first_searched;
            first_searched = false;
            self.play_prepared(col, level, i);
            let r = (|| -> Result<i32, Abort> {
                if first {
                    Ok(-self.negamax(-beta, -alpha, precomputed)?)
                } else {
                    let s = -self.negamax(-alpha - 1, -alpha, precomputed)?;
                    if s > alpha && s < beta {
                        let refresh = precomputed.map(|(key, _)| (key, None));
                        Ok(-self.negamax(-beta, -alpha, refresh)?)
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
        if best < min_possible {
            best = min_possible;
        }
        if best <= alpha_orig {
            best = self.snap_upper_bound(best);
            self.store_bounds(slot, key, NO_LOWER, best, best_col);
        } else if best >= beta_orig {
            best = self.snap_lower_bound(best);
            self.store_bounds(slot, key, best, NO_UPPER, best_col);
        } else {
            self.store_exact(slot, key, best, best_col);
        }
        Ok(best)
    }

    fn prepare(&mut self) {
        self.prep_forced = false;
        self.prep_min_possible_valid = false;
        let playable = self.playable;
        let my_winning = self.spots_cur & playable;
        if my_winning != 0 {
            self.prep_forced = true;
            self.prep_forced_score = self.cells - self.moves;
            self.prep_forced_col = self.column_of(my_winning);
            return;
        }
        let opp_spots = self.spots_opp;
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
                self.prep_forced = true;
                self.prep_forced_score = -(self.cells - self.moves - 1);
                self.prep_forced_col = block_col;
                return;
            }
            self.move_cols[level] = block_col;
            self.move_slots[level] = -1;
            self.move_hot_data[level] = -1;
            self.move_spots_ready[level] = false;
            self.prep_count = 1;
            self.prep_min_possible_valid = true;
            return;
        }

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
                continue;
            }
            let mut s = (c.min(self.width - 1 - c) + 1) * 40;
            s += self.heights[c as usize] * 10;
            if self.heights[c as usize] == self.height - 1 {
                s += 75;
            }
            s += (self.window_offsets[idx as usize + 1] - self.window_offsets[idx as usize]) as i32
                * 10;
            if (opp_spots >> idx) & 1 != 0 {
                s += 5_000;
            }
            let bit = 1u64 << idx;
            let occupancy_after = self.mask | bit;
            let terminal_after = self.moves + 1 == self.cells;
            let mut child_key = 0u64;
            let mut child_flipped = false;
            let child_slot = -1i32;
            let mut child_hot_data = -1i32;
            let mut created_spots = 0u64;
            let mut created_won = false;
            let mut spots_ready = false;
            if !terminal_after {
                let (created, won) =
                    self.spots_created_by(idx as usize, self.cur | bit, occupancy_after);
                created_spots = created;
                created_won = won;
                spots_ready = true;
                let mover_spots_after = (self.spots_cur & !bit) | created;
                let mut playable_after = playable & !bit;
                if self.heights[c as usize] + 1 < self.height {
                    playable_after |= bit << 1;
                }
                let playable_spots = mover_spots_after & playable_after;
                if self.cells - self.moves >= 3
                    && (playable_spots.count_ones() >= 2
                        || playable_spots & (mover_spots_after >> 1) != 0)
                {
                    self.prep_forced = true;
                    self.prep_forced_score = self.cells - self.moves - 2;
                    self.prep_forced_col = c;
                    return;
                }
                let filtered_spots =
                    mover_spots_after & !((opp_spots & !bit) << 1) & !occupancy_after;
                s += filtered_spots.count_ones() as i32 * 300;
                s += (filtered_spots & playable_after).count_ones() as i32
                    * if self.cells - self.moves >= 25 {
                        600
                    } else {
                        450
                    };
                s += (filtered_spots & (playable_after << 1)).count_ones() as i32 * 600;
                s += (filtered_spots & (filtered_spots << 1)).count_ones() as i32 * 600;
                if self.cells - self.moves - 1 > 0 {
                    let child = self.canonical_key_from(self.mask ^ self.cur, occupancy_after);
                    child_key = child.code;
                    child_flipped = child.flipped;
                    if let Some(data) = self.hot_get(child.code) {
                        child_hot_data = data;
                        if c != self.prep_tt_col {
                            let upper = tt_upper(data);
                            if upper != NO_UPPER {
                                s += (self.cells - upper) * 30;
                            }
                        }
                    }
                }
            }
            self.move_cols[level + n] = c;
            self.move_scores[level + n] = s;
            self.move_keys[level + n] = child_key;
            self.move_flipped[level + n] = child_flipped;
            self.move_slots[level + n] = child_slot;
            self.move_hot_data[level + n] = child_hot_data;
            self.move_created_spots[level + n] = created_spots;
            self.move_created_won[level + n] = created_won;
            self.move_spots_ready[level + n] = spots_ready;
            n += 1;
        }
        if n == 0 {
            self.prep_forced = true;
            self.prep_forced_score = -(self.cells - self.moves - 1);
            self.prep_forced_col = first_legal_col;
            return;
        }

        self.prep_count = n;
    }

    fn reconstruct_variation(&mut self) -> Vec<Move> {
        let (save_cur, save_mask, save_heights, save_moves, save_odd) = (
            self.cur,
            self.mask,
            self.heights.clone(),
            self.moves,
            self.odd_height_columns,
        );
        let (save_spots_cur, save_spots_opp) = (self.spots_cur, self.spots_opp);
        let save_playable = self.playable;
        let save_won = self.won_by_last_move;
        let mut pv = Vec::new();
        while self.moves < self.cells && pv.len() as i32 <= self.cells {
            if self.won_by_last_move {
                break;
            }
            let key = self.canonical_key();
            let slot = self.slot_for(key.code);
            let mut col = -1;
            if self.tt_code[slot] == key.code {
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
                let win = self.spots_cur & self.playable;
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
        self.spots_cur = save_spots_cur;
        self.spots_opp = save_spots_opp;
        self.playable = save_playable;
        self.won_by_last_move = save_won;
        pv
    }

    #[inline]
    fn play(&mut self, col: i32) {
        self.play_inner(col, None);
    }

    #[inline]
    fn play_prepared(&mut self, col: i32, level: usize, i: usize) {
        let pre = if self.move_spots_ready[level + i] {
            Some((
                self.move_created_spots[level + i],
                self.move_created_won[level + i],
            ))
        } else {
            None
        };
        self.play_inner(col, pre);
    }

    #[inline]
    fn play_inner(&mut self, col: i32, precomputed_spots: Option<(u64, bool)>) {
        let c = col as usize;
        let idx = col * self.stride + self.heights[c];
        let bit = 1u64 << idx;
        let stack_idx = self.moves as usize;
        self.spot_stack_cur[stack_idx] = self.spots_cur;
        self.spot_stack_opp[stack_idx] = self.spots_opp;
        self.won_stack[stack_idx] = self.won_by_last_move;
        let mover_spots_before = self.spots_cur;
        let opponent_spots_before = self.spots_opp;

        self.cur ^= self.mask;
        self.mask |= bit;
        if self.heights[c] & 1 == 0 {
            self.odd_height_columns += 1;
        } else {
            self.odd_height_columns -= 1;
        }
        self.heights[c] += 1;
        self.playable &= !bit;
        if self.heights[c] < self.height {
            self.playable |= bit << 1;
        }
        self.moves += 1;

        let mover_bits = self.mask ^ self.cur;
        let mut mover_spots = mover_spots_before & !bit;
        let (created, won) = match precomputed_spots {
            Some(pair) => pair,
            None => self.spots_created_by(idx as usize, mover_bits, self.mask),
        };
        mover_spots |= created;
        self.won_by_last_move = won;

        self.spots_cur = opponent_spots_before & !bit;
        self.spots_opp = mover_spots;
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
        if self.heights[c] + 1 < self.height {
            self.playable &= !(bit << 1);
        }
        self.playable |= bit;
        let stack_idx = self.moves as usize;
        self.spots_cur = self.spot_stack_cur[stack_idx];
        self.spots_opp = self.spot_stack_opp[stack_idx];
        self.won_by_last_move = self.won_stack[stack_idx];
    }

    #[inline]
    fn spots_created_by(&self, idx: usize, mover_bits: u64, occupancy: u64) -> (u64, bool) {
        let mut created = 0u64;
        let mut won = false;
        let start = self.window_offsets[idx];
        let end = self.window_offsets[idx + 1];
        for &window in &self.windows_flat[start..end] {
            let count = (mover_bits & window).count_ones() as i32;
            if count == self.connect {
                won = true;
            } else if count == self.connect - 1 {
                created |= window & !occupancy;
            }
        }
        (created & self.board_mask, won)
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
                    return self.infinity;
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
    fn has_nonvertical_line(&self, bits: u64) -> bool {
        for &sh in &self.shifts {
            if sh == 1 {
                continue;
            }
            let mut x = bits;
            let mut i = 1;
            while i < self.connect && x != 0 {
                x &= x >> sh;
                i += 1;
            }
            if x != 0 {
                return true;
            }
        }
        false
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
            1 << 18
        };
        cap = cap.min(self.max_tt_capacity);
        for slot in self.tt_touched.drain(..) {
            self.tt_code[slot] = 0;
        }
        if self.tt_code.len() < cap {
            self.tt_code.resize(cap, 0);
            self.tt_data.resize(cap, 0);
        }
        self.tt_cap = cap;
        self.tt_index_mask = cap - 1;
        self.tt_shift = 64 - cap.trailing_zeros();
        if self.hot_code.len() != HOT_TT_CAPACITY {
            self.hot_code = vec![0; HOT_TT_CAPACITY];
            self.hot_data = vec![0; HOT_TT_CAPACITY];
            self.hot_touched.clear();
        } else {
            for slot in self.hot_touched.drain(..) {
                self.hot_code[slot] = 0;
            }
        }
    }

    fn grow_tt(&mut self, new_cap: usize) {
        if new_cap <= self.tt_cap {
            return;
        }
        let old_touched = std::mem::take(&mut self.tt_touched);
        let mut entries = Vec::with_capacity(old_touched.len());
        for slot in old_touched {
            let code = self.tt_code[slot];
            if code != 0 {
                entries.push((code, self.tt_data[slot]));
            }
            self.tt_code[slot] = 0;
        }
        if self.tt_code.len() < new_cap {
            self.tt_code.resize(new_cap, 0);
            self.tt_data.resize(new_cap, 0);
        }
        self.tt_cap = new_cap;
        self.tt_index_mask = new_cap - 1;
        self.tt_shift = 64 - new_cap.trailing_zeros();
        for (code, data) in entries {
            let slot = self.slot_for(code);
            if self.tt_code[slot] == 0 {
                self.tt_touched.push(slot);
            }
            self.tt_code[slot] = code;
            self.tt_data[slot] = data;
        }
    }

    #[inline]
    fn hot_get(&self, code: u64) -> Option<i32> {
        if self.hot_code.is_empty() {
            return None;
        }
        let idx = (code.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> HOT_TT_SHIFT) as usize;
        if self.hot_code[idx] == code {
            Some(self.hot_data[idx])
        } else {
            None
        }
    }

    #[inline]
    fn hot_put(&mut self, code: u64, data: i32) {
        if self.hot_code.is_empty() {
            return;
        }
        let idx = (code.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> HOT_TT_SHIFT) as usize;
        if self.hot_code[idx] == 0 {
            self.hot_touched.push(idx);
        }
        self.hot_code[idx] = code;
        self.hot_data[idx] = data;
    }

    #[inline]
    fn canonical_key(&self) -> TtKey {
        self.canonical_key_from(self.cur, self.mask)
    }

    #[inline]
    fn canonical_key_from(&self, cur: u64, mask: u64) -> TtKey {
        let code = cur.wrapping_add(mask).wrapping_add(self.bottom_mask);
        if !self.mirror_symmetric {
            return TtKey {
                code,
                flipped: false,
            };
        }
        let mirrored = mirror_bits_with_stride(code, self.stride, self.width);
        if mirrored < code {
            TtKey {
                code: mirrored,
                flipped: true,
            }
        } else {
            TtKey {
                code,
                flipped: false,
            }
        }
    }

    #[inline]
    fn book_canonical_key(&self) -> BookKey {
        let cur = repack_word(
            self.cur,
            self.stride,
            self.book_stride,
            self.width,
            self.height,
        );
        let mask = repack_word(
            self.mask,
            self.stride,
            self.book_stride,
            self.width,
            self.height,
        );
        self.canonical_pair_from(cur, mask, self.book_stride)
    }

    #[inline]
    fn canonical_pair_from(&self, cur: u64, mask: u64, stride: i32) -> BookKey {
        if !self.mirror_symmetric {
            return BookKey {
                cur,
                mask,
                flipped: false,
            };
        }
        let m_mask = mirror_bits_with_stride(mask, stride, self.width);
        if m_mask < mask {
            return BookKey {
                cur: mirror_bits_with_stride(cur, stride, self.width),
                mask: m_mask,
                flipped: true,
            };
        }
        if m_mask == mask {
            let m_cur = mirror_bits_with_stride(cur, stride, self.width);
            if m_cur < cur {
                return BookKey {
                    cur: m_cur,
                    mask: m_mask,
                    flipped: true,
                };
            }
        }
        BookKey {
            cur,
            mask,
            flipped: false,
        }
    }

    #[inline]
    fn slot_for(&self, code: u64) -> usize {
        let home = (code.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> self.tt_shift) as usize;
        for probe in 0..MAX_PROBES {
            let i = (home + probe) & self.tt_index_mask;
            if self.tt_code[i] == 0 || self.tt_code[i] == code {
                return i;
            }
        }
        home
    }

    #[inline]
    fn snap_lower_bound(&self, bound: i32) -> i32 {
        if bound == NO_LOWER || bound == 0 {
            return bound;
        }
        let empties_parity = (self.cells - self.moves) & 1;
        let feasible = if bound > 0 {
            bound & 1 == empties_parity
        } else {
            (-bound) & 1 != empties_parity
        };
        if feasible {
            bound
        } else {
            bound + 1
        }
    }

    #[inline]
    fn snap_upper_bound(&self, bound: i32) -> i32 {
        if bound == NO_UPPER || bound == 0 {
            return bound;
        }
        let empties_parity = (self.cells - self.moves) & 1;
        let feasible = if bound > 0 {
            bound & 1 == empties_parity
        } else {
            (-bound) & 1 != empties_parity
        };
        if feasible {
            bound
        } else {
            bound - 1
        }
    }

    fn store_exact(&mut self, slot: usize, key: TtKey, score: i32, best_col: i32) {
        self.store_bounds(slot, key, score, score, best_col);
    }

    fn store_bounds(
        &mut self,
        slot: usize,
        key: TtKey,
        mut lower: i32,
        mut upper: i32,
        best_col: i32,
    ) {
        lower = self.snap_lower_bound(lower);
        upper = self.snap_upper_bound(upper);
        let mut canonical_col = if best_col < 0 {
            -1
        } else if key.flipped {
            self.width - 1 - best_col
        } else {
            best_col
        };
        let was_empty = self.tt_code[slot] == 0;
        if self.tt_code[slot] == key.code {
            let data = self.tt_data[slot];
            lower = lower.max(tt_lower(data));
            upper = upper.min(tt_upper(data));
            if canonical_col < 0 {
                canonical_col = tt_best_col(data);
            }
        }
        if was_empty {
            self.tt_touched.push(slot);
        }
        self.tt_code[slot] = key.code;
        self.tt_data[slot] = (lower & 0xFF) | ((upper & 0xFF) << 8) | ((canonical_col + 1) << 16);
        let packed = self.tt_data[slot];
        self.hot_put(key.code, packed);
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
fn repack_word(word: u64, from_stride: i32, to_stride: i32, width: i32, height: i32) -> u64 {
    if from_stride == to_stride {
        return word;
    }
    let col_mask = if height >= 64 {
        u64::MAX
    } else {
        (1u64 << height) - 1
    };
    let mut out = 0u64;
    for c in 0..width {
        let chunk = (word >> (c * from_stride)) & col_mask;
        out |= chunk << (c * to_stride);
    }
    out
}

#[inline]
fn mirror_bits_with_stride(bits: u64, stride: i32, width: i32) -> u64 {
    if stride == 8 && width <= 8 {
        return bits.swap_bytes() >> ((8 - width) * 8);
    }
    let stride = stride as u32;
    let col_mask: u64 = if stride >= 64 {
        u64::MAX
    } else {
        (1u64 << stride) - 1
    };
    let mut out = 0u64;
    for c in 0..width as u32 {
        let chunk = (bits >> (c * stride)) & col_mask;
        out |= chunk << ((width as u32 - 1 - c) * stride);
    }
    out
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
