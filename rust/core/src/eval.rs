use crate::bits;
use crate::model::{Placement, Rules};
use crate::position::Position;

pub const WIN_SCORE: i32 = 1_000_000;
const HEURISTIC_CLAMP: i32 = WIN_SCORE / 2;

pub fn is_winning_score(score: i32) -> bool {
    score > HEURISTIC_CLAMP
}
pub fn is_losing_score(score: i32) -> bool {
    score < -HEURISTIC_CLAMP
}

pub fn terminal_score(p: &Position) -> i32 {
    match p.winner() {
        Some(w) => {
            if w == p.side_to_move() {
                WIN_SCORE - p.moves_played() as i32 * 10
            } else {
                -WIN_SCORE + p.moves_played() as i32 * 10
            }
        }
        None => 0,
    }
}

struct Window {
    mask1: u64,
    mask: Box<[u64]>,
    cells: Box<[i32]>,
}

pub struct Evaluator {
    rules: Rules,
    windows: Vec<Window>,
    weights: Vec<i32>,
    column_masks: Vec<Box<[u64]>>,
    column_weights: Vec<i32>,
}

impl Evaluator {
    pub fn new(rules: &Rules) -> Evaluator {
        let k = rules.connect;
        let mut weights = vec![0i32; (k + 1) as usize];
        let mut w = 1i32;
        for slot in weights.iter_mut().skip(1) {
            *slot = w;
            w *= 8;
        }
        let mut windows = Vec::new();
        for &(dc, dr) in rules.dir_steps() {
            for c in 0..rules.width {
                for r in 0..rules.height {
                    let (ec, er) = (c + dc * (k - 1), r + dr * (k - 1));
                    if ec < 0 || ec >= rules.width || er < 0 || er >= rules.height {
                        continue;
                    }
                    let mut mask = vec![0u64; rules.word_count()];
                    let mut cells = Vec::with_capacity(k as usize);
                    for i in 0..k {
                        let idx = rules.index(c + dc * i, r + dr * i);
                        bits::set(&mut mask, idx);
                        cells.push(idx);
                    }
                    windows.push(Window {
                        mask1: mask[0],
                        mask: mask.into_boxed_slice(),
                        cells: cells.into_boxed_slice(),
                    });
                }
            }
        }
        let mut column_masks = Vec::with_capacity(rules.width as usize);
        let mut column_weights = Vec::with_capacity(rules.width as usize);
        for c in 0..rules.width {
            let mut mask = vec![0u64; rules.word_count()];
            for r in 0..rules.height {
                bits::set(&mut mask, rules.index(c, r));
            }
            column_masks.push(mask.into_boxed_slice());
            column_weights.push(c.min(rules.width - 1 - c) + 1);
        }
        Evaluator {
            rules: rules.clone(),
            windows,
            weights,
            column_masks,
            column_weights,
        }
    }

    pub fn rules(&self) -> &Rules {
        &self.rules
    }
    pub fn window_count(&self) -> usize {
        self.windows.len()
    }

    pub fn evaluate(&self, p: &Position) -> i32 {
        if p.is_terminal() {
            return terminal_score(p);
        }
        let side = p.side_to_move();
        let gravity = self.rules.placement == Placement::Gravity;
        let k = self.rules.connect;
        if self.rules.word_count() == 1 {
            let own = p.current_words()[0];
            let opp = p.mask_words()[0] ^ own;
            self.evaluate_words(p, own, opp, gravity, k)
        } else {
            let words = self.rules.word_count();
            if words <= 4 {
                let mut own = [0u64; 4];
                let mut opp = [0u64; 4];
                p.write_stone_bits(side, &mut own[..words]);
                p.write_stone_bits(side.other(), &mut opp[..words]);
                self.evaluate_generic(p, &own[..words], &opp[..words], gravity, k)
            } else {
                let own = p.stone_bits(side);
                let opp = p.stone_bits(side.other());
                self.evaluate_generic(p, &own, &opp, gravity, k)
            }
        }
    }

    fn evaluate_words(&self, p: &Position, own: u64, opp: u64, gravity: bool, k: i32) -> i32 {
        let mut score = 0.0f64;
        for win in &self.windows {
            let own_in = own & win.mask1;
            let opp_in = opp & win.mask1;
            if own_in == 0 && opp_in == 0 {
                continue;
            }
            if own_in != 0 && opp_in != 0 {
                continue;
            }
            let own_count = own_in.count_ones() as i32;
            let opp_count = opp_in.count_ones() as i32;
            let mine = own_count > 0;
            let count = if mine { own_count } else { opp_count };
            let mut factor = 1.0f64;
            let mut min_support = i32::MAX;
            let mut total_support = 0i32;
            if gravity {
                for &idx in win.cells.iter() {
                    let bit = 1u64 << idx;
                    if own & bit != 0 || opp & bit != 0 {
                        continue;
                    }
                    let col = self.rules.column_of(idx);
                    let row = self.rules.row_of(idx);
                    let support = (row - p.column_count(col)).max(0);
                    total_support += support;
                    min_support = min_support.min(support);
                }
                factor = 1.0 / (1.0 + 0.4 * total_support as f64);
            } else {
                min_support = 0;
            }
            if count == k - 1 {
                if min_support == 0 {
                    factor *= 25.0;
                } else if min_support == 1 {
                    factor *= 5.0;
                }
            }
            let value = self.weights[count as usize] as f64 * factor;
            score += if mine { value } else { -1.05 * value };
        }
        score += 0.05 * (self.centrality1(own) - self.centrality1(opp)) as f64;
        clamp_round(score)
    }

    fn evaluate_generic(
        &self,
        p: &Position,
        own: &[u64],
        opp: &[u64],
        gravity: bool,
        k: i32,
    ) -> i32 {
        let mut score = 0.0f64;
        for win in &self.windows {
            let mut own_any = false;
            let mut opp_any = false;
            for i in 0..win.mask.len() {
                own_any |= own[i] & win.mask[i] != 0;
                opp_any |= opp[i] & win.mask[i] != 0;
                if own_any && opp_any {
                    break;
                }
            }
            if own_any == opp_any {
                continue;
            }
            let own_count = if own_any {
                bits::and_count(own, &win.mask) as i32
            } else {
                0
            };
            let opp_count = if opp_any {
                bits::and_count(opp, &win.mask) as i32
            } else {
                0
            };
            let mine = own_count > 0;
            let count = if mine { own_count } else { opp_count };
            let mut factor = 1.0f64;
            let mut min_support = i32::MAX;
            let mut total_support = 0i32;
            if gravity {
                for &idx in win.cells.iter() {
                    if bits::test(own, idx) || bits::test(opp, idx) {
                        continue;
                    }
                    let col = self.rules.column_of(idx);
                    let row = self.rules.row_of(idx);
                    let support = (row - p.column_count(col)).max(0);
                    total_support += support;
                    min_support = min_support.min(support);
                }
                factor = 1.0 / (1.0 + 0.4 * total_support as f64);
            } else {
                min_support = 0;
            }
            if count == k - 1 {
                if min_support == 0 {
                    factor *= 25.0;
                } else if min_support == 1 {
                    factor *= 5.0;
                }
            }
            let value = self.weights[count as usize] as f64 * factor;
            score += if mine { value } else { -1.05 * value };
        }
        score += 0.05 * (self.centrality(own) - self.centrality(opp)) as f64;
        clamp_round(score)
    }

    fn centrality1(&self, stones: u64) -> i32 {
        let mut total = 0i32;
        for (mask, weight) in self.column_masks.iter().zip(&self.column_weights) {
            total += (stones & mask[0]).count_ones() as i32 * weight;
        }
        total
    }

    fn centrality(&self, stones: &[u64]) -> i32 {
        let mut total = 0i32;
        for (mask, weight) in self.column_masks.iter().zip(&self.column_weights) {
            total += bits::and_count(stones, mask) as i32 * weight;
        }
        total
    }
}

#[inline]
fn clamp_round(score: f64) -> i32 {
    let s = (score + 0.5).floor() as i32;
    s.clamp(-HEURISTIC_CLAMP, HEURISTIC_CLAMP)
}
