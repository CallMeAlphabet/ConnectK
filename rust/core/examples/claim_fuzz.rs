//! Validation harness for the claim-even bound: plays many random and
//! claim-style games; whenever the bound fires on a position with few
//! enough empties, checks it against the generic exact solver.
//!
//! Usage: cargo run --release --example claim_fuzz -- [games] [maxEmpties]

use connectk_core::model::Rules;
use connectk_core::position::Position;
use connectk_core::solver::fast::FastExactSolver;
use connectk_core::solver::generic::ExactSolver;
use connectk_core::solver::{Certainty, SearchControl};

struct JavaRandom {
    seed: u64,
}
impl JavaRandom {
    fn new(seed: i64) -> JavaRandom {
        JavaRandom { seed: ((seed as u64) ^ 0x5DEE_CE66D) & ((1u64 << 48) - 1) }
    }
    fn next(&mut self, bits: u32) -> i32 {
        self.seed = (self.seed.wrapping_mul(0x5DEE_CE66D).wrapping_add(0xB)) & ((1u64 << 48) - 1);
        (self.seed >> (48 - bits)) as u64 as i32
    }
    fn next_int(&mut self, bound: i32) -> i32 {
        if bound & -bound == bound {
            return ((bound as i64 * self.next(31) as i64) >> 31) as i32;
        }
        loop {
            let bits = self.next(31);
            let val = bits % bound;
            if bits.wrapping_sub(val).wrapping_add(bound - 1) >= 0 {
                return val;
            }
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let games: i32 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(3000);
    let max_empties: i32 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(18);
    let rules = Rules::standard_connect4();
    let mut rng = JavaRandom::new(1234567);
    let (mut positions, mut fired, mut checked, mut negative) = (0u64, 0u64, 0u64, 0u64);
    for g in 0..games {
        let mut pos = Position::initial(&rules);
        let mut last_column = -1;
        let mode = g % 3;
        while !pos.is_terminal() {
            let empties = rules.cells() - pos.moves_played() as i32;
            if pos.moves_played() >= 16 {
                positions += 1;
                let bound = FastExactSolver::claim_even_upper_bound(&pos);
                if bound != i32::MAX {
                    fired += 1;
                    if bound < 0 {
                        negative += 1;
                    }
                    if empties <= max_empties {
                        let ex =
                            ExactSolver::new().solve(&pos, SearchControl::new(0, 60_000));
                        checked += 1;
                        if ex.certainty != Certainty::ProvenExact {
                            println!("oracle UNKNOWN at empties={empties}");
                        } else if ex.score > bound {
                            println!(
                                "VIOLATION: bound={bound} exact={} movesPlayed={} sideToMove={}",
                                ex.score,
                                pos.moves_played(),
                                pos.side_to_move()
                            );
                            println!("{pos}");
                            for c in 0..rules.width {
                                println!(
                                    "col {} height {} ({})",
                                    c + 1,
                                    pos.column_count(c),
                                    if pos.column_count(c) % 2 == 1 { "odd" } else { "even" }
                                );
                            }
                            return;
                        }
                    }
                }
            }
            let legal = pos.legal_moves();
            if legal.is_empty() {
                break;
            }
            let claimer_turn = (mode == 1 && pos.moves_played() % 2 == 1)
                || (mode == 2 && pos.moves_played() % 2 == 0);
            let mut chosen = None;
            if claimer_turn && last_column >= 0 {
                for m in &legal {
                    if m.column == last_column {
                        chosen = Some(*m);
                        break;
                    }
                }
            }
            let chosen = chosen.unwrap_or(legal[rng.next_int(legal.len() as i32) as usize]);
            pos.play(chosen).unwrap();
            last_column = chosen.column;
        }
    }
    println!(
        "done: games={games} positions={positions} fired={fired} negative={negative} exact-checked={checked} violations=0"
    );
}
