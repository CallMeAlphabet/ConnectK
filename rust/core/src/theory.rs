use crate::model::{dirs, Move, Outcome, Placement, Player};
use crate::position::Position;

#[derive(Clone, Debug)]
pub struct TheoryResult {
    pub outcome: Outcome,

    pub plies_to_end: Option<i32>,

    pub proven_best_move: Option<Move>,

    pub source: &'static str,
}

#[derive(Clone, Debug)]
pub struct SymbolicTheoryResult {
    pub outcome: Outcome,
    pub plies_to_end: Option<i64>,
    pub proven_best_move: Option<Move>,
    pub source: &'static str,
}

const SOURCE_TABLE: &str = "Published board-size table (Tromp 2006/2015; Christophe Steininger)";
const SOURCE_STANDARD: &str = "Standard 7x6 solution (Allen / Allis 1988; exact \
                               scores verified by modern strong solvers)";
const SOURCE_ELEMENTARY: &str =
    "Elementary exact ConnectK theory (degenerate K / no feasible line / forced single column)";
const SOURCE_PAIRING: &str = "Pairing strategy plus strategy stealing for strong positional games";
const SOURCE_MNK: &str = "Published m,n,k-game classification";

static EMPTY_BOARD: &[(i32, i32, Outcome, i32)] = &[
    (4, 4, Outcome::Draw, 16),
    (4, 5, Outcome::Draw, 20),
    (4, 6, Outcome::Draw, 24),
    (4, 7, Outcome::Draw, 28),
    (4, 8, Outcome::Draw, 32),
    (4, 9, Outcome::Draw, 36),
    (4, 10, Outcome::Draw, 40),
    (4, 11, Outcome::Draw, 44),
    (4, 12, Outcome::Draw, 48),
    (4, 13, Outcome::Draw, 52),
    (5, 4, Outcome::Draw, 20),
    (5, 5, Outcome::Draw, 25),
    (5, 6, Outcome::Draw, 30),
    (5, 7, Outcome::Draw, 35),
    (5, 8, Outcome::Draw, 40),
    (5, 9, Outcome::Draw, 45),
    (5, 10, Outcome::Draw, 50),
    (5, 11, Outcome::Draw, 55),
    (5, 12, Outcome::Draw, 60),
    (5, 13, Outcome::Draw, 65),
    (6, 4, Outcome::Loss, 24),
    (6, 5, Outcome::Draw, 30),
    (6, 6, Outcome::Loss, 36),
    (6, 7, Outcome::Win, 41),
    (6, 8, Outcome::Loss, 48),
    (6, 9, Outcome::Win, 53),
    (6, 10, Outcome::Loss, 60),
    (6, 11, Outcome::Win, 63),
    (6, 12, Outcome::Loss, 72),
    (7, 4, Outcome::Draw, 28),
    (7, 5, Outcome::Draw, 35),
    (7, 6, Outcome::Win, 41),
    (7, 7, Outcome::Draw, 49),
    (7, 8, Outcome::Win, 55),
    (7, 9, Outcome::Draw, 63),
    (7, 10, Outcome::Win, 69),
    (8, 4, Outcome::Loss, 32),
    (8, 5, Outcome::Win, 39),
    (8, 6, Outcome::Loss, 48),
    (8, 7, Outcome::Win, 53),
    (8, 8, Outcome::Loss, 62),
    (8, 9, Outcome::Win, -1),
    (9, 4, Outcome::Loss, 36),
    (9, 5, Outcome::Win, 41),
    (9, 6, Outcome::Loss, 52),
    (9, 7, Outcome::Win, 55),
    (10, 4, Outcome::Loss, 40),
    (10, 5, Outcome::Win, 47),
    (10, 6, Outcome::Win, 59),
    (11, 4, Outcome::Loss, 44),
    (11, 5, Outcome::Win, 51),
    (12, 4, Outcome::Loss, 48),
];

fn raw_max_line(width: i32, height: i32, directions: u8) -> i32 {
    let mut max_line = 0;
    if directions & dirs::HORIZONTAL != 0 {
        max_line = max_line.max(width);
    }
    if directions & dirs::VERTICAL != 0 {
        max_line = max_line.max(height);
    }
    if directions & (dirs::DIAGONAL_UP | dirs::DIAGONAL_DOWN) != 0 {
        max_line = max_line.max(width.min(height));
    }
    max_line
}

fn raw_feasible_directions(width: i32, height: i32, connect: i32, directions: u8) -> u8 {
    let mut feasible = 0;
    if directions & dirs::HORIZONTAL != 0 && width >= connect {
        feasible |= dirs::HORIZONTAL;
    }
    if directions & dirs::VERTICAL != 0 && height >= connect {
        feasible |= dirs::VERTICAL;
    }
    if directions & dirs::DIAGONAL_UP != 0 && width >= connect && height >= connect {
        feasible |= dirs::DIAGONAL_UP;
    }
    if directions & dirs::DIAGONAL_DOWN != 0 && width >= connect && height >= connect {
        feasible |= dirs::DIAGONAL_DOWN;
    }
    feasible
}

fn first_cell(width: i32, height: i32) -> Option<Move> {
    if width > 0 && height > 0 {
        Some(Move::cell(0, 0))
    } else {
        None
    }
}

fn free_k2_first_move(width: i32, height: i32, directions: u8) -> Option<Move> {
    let steps = [
        (dirs::VERTICAL, 0, 1),
        (dirs::HORIZONTAL, 1, 0),
        (dirs::DIAGONAL_UP, 1, 1),
        (dirs::DIAGONAL_DOWN, 1, -1),
    ];
    let xs = [width / 2, 0, 1, width - 2, width - 1];
    let ys = [height / 2, 0, 1, height - 2, height - 1];
    let mut best: Option<(i32, Move)> = None;
    for &x in &xs {
        if x < 0 || x >= width {
            continue;
        }
        for &y in &ys {
            if y < 0 || y >= height {
                continue;
            }
            let mut degree = 0;
            for &(bit, dc, dr) in &steps {
                if directions & bit == 0 {
                    continue;
                }
                for sign in [-1, 1] {
                    let nx = x + dc * sign;
                    let ny = y + dr * sign;
                    if nx >= 0 && nx < width && ny >= 0 && ny < height {
                        degree += 1;
                    }
                }
            }
            if best.map(|(d, _)| degree > d).unwrap_or(true) {
                best = Some((degree, Move::cell(x, y)));
            }
        }
    }
    match best {
        Some((degree, mv)) if degree >= 2 => Some(mv),
        _ => None,
    }
}

fn gravity_k2_first_move(width: i32, height: i32, directions: u8) -> Option<Move> {
    let horizontal = directions & dirs::HORIZONTAL != 0;
    if !horizontal {
        return None;
    }
    if height == 1 {
        if width >= 3 {
            return Some(Move::cell(width / 2, 0));
        }
        return None;
    }
    if width >= 3 {
        return Some(Move::cell(1, 0));
    }
    if width == 2 && height >= 2 {
        if directions & dirs::VERTICAL != 0 || directions & dirs::DIAGONAL_UP != 0 {
            return Some(Move::cell(0, 0));
        }
        if directions & dirs::DIAGONAL_DOWN != 0 {
            return Some(Move::cell(1, 0));
        }
    }
    None
}

fn mnk_free_classification(width: i32, height: i32, connect: i32) -> Option<Outcome> {
    if connect >= 9 {
        return Some(Outcome::Draw);
    }
    if width == height && width == connect && connect >= 3 {
        return Some(Outcome::Draw);
    }
    let lo = width.min(height);
    let hi = width.max(height);
    match connect {
        3 => {
            if width == 3 && height == 3 {
                Some(Outcome::Draw)
            } else if lo >= 3 && hi >= 4 {
                Some(Outcome::Win)
            } else {
                None
            }
        }
        4 => {
            if width <= 5 && height <= 5 {
                Some(Outcome::Draw)
            } else if (width >= 6 && height >= 5) || (width >= 5 && height >= 6) {
                Some(Outcome::Win)
            } else if lo == 4 {
                Some(if hi >= 9 { Outcome::Win } else { Outcome::Draw })
            } else {
                None
            }
        }
        5 => {
            if width <= 8 && height <= 8 {
                Some(Outcome::Draw)
            } else if width >= 15 && height >= 15 {
                Some(Outcome::Win)
            } else {
                None
            }
        }
        6 => {
            if (width <= 9 && height <= 6)
                || (width <= 6 && height <= 9)
                || (width <= 7 && height <= 7)
            {
                Some(Outcome::Draw)
            } else {
                None
            }
        }
        _ => None,
    }
}

pub fn lookup_empty_symbolic(
    width: i32,
    height: i32,
    connect: i32,
    placement: Placement,
    directions: u8,
    misere: bool,
) -> Option<SymbolicTheoryResult> {
    if width < 0 || height < 0 || connect < 0 || directions == 0 || directions & !dirs::ALL != 0 {
        return None;
    }
    let cells = width as i64 * height as i64;
    if connect == 0 {
        return Some(SymbolicTheoryResult {
            outcome: if misere { Outcome::Loss } else { Outcome::Win },
            plies_to_end: Some(0),
            proven_best_move: None,
            source: SOURCE_ELEMENTARY,
        });
    }
    if cells == 0 {
        return Some(SymbolicTheoryResult {
            outcome: Outcome::Draw,
            plies_to_end: Some(0),
            proven_best_move: None,
            source: SOURCE_ELEMENTARY,
        });
    }
    if connect > raw_max_line(width, height, directions) {
        return Some(SymbolicTheoryResult {
            outcome: Outcome::Draw,
            plies_to_end: Some(cells),
            proven_best_move: first_cell(width, height),
            source: SOURCE_ELEMENTARY,
        });
    }
    if connect == 1 {
        return Some(SymbolicTheoryResult {
            outcome: if misere { Outcome::Loss } else { Outcome::Win },
            plies_to_end: Some(1),
            proven_best_move: first_cell(width, height),
            source: SOURCE_ELEMENTARY,
        });
    }
    if placement == Placement::Gravity && width == 1 {
        return Some(SymbolicTheoryResult {
            outcome: Outcome::Draw,
            plies_to_end: Some(cells),
            proven_best_move: first_cell(width, height),
            source: SOURCE_ELEMENTARY,
        });
    }
    if connect == 2 && !misere {
        let first = match placement {
            Placement::Free => free_k2_first_move(width, height, directions),
            Placement::Gravity => gravity_k2_first_move(width, height, directions),
        };
        if let Some(mv) = first {
            return Some(SymbolicTheoryResult {
                outcome: Outcome::Win,
                plies_to_end: Some(3),
                proven_best_move: Some(mv),
                source: SOURCE_ELEMENTARY,
            });
        }
    }
    if !misere
        && connect >= 3
        && (placement == Placement::Free || (placement == Placement::Gravity && height == 1))
        && raw_feasible_directions(width, height, connect, directions).count_ones() == 1
    {
        return Some(SymbolicTheoryResult {
            outcome: Outcome::Draw,
            plies_to_end: Some(cells),
            proven_best_move: None,
            source: SOURCE_PAIRING,
        });
    }
    if !misere && placement == Placement::Free && directions == dirs::ALL {
        if let Some(outcome) = mnk_free_classification(width, height, connect) {
            return Some(SymbolicTheoryResult {
                outcome,
                plies_to_end: if outcome == Outcome::Draw {
                    Some(cells)
                } else {
                    None
                },
                proven_best_move: None,
                source: SOURCE_MNK,
            });
        }
    }
    if placement == Placement::Gravity && connect == 4 && !misere && directions == dirs::ALL {
        for &(w, h, outcome, last_ply) in EMPTY_BOARD {
            if w == width && h == height {
                return Some(SymbolicTheoryResult {
                    outcome,
                    plies_to_end: if last_ply > 0 {
                        Some(last_ply as i64)
                    } else {
                        None
                    },
                    proven_best_move: if width == 7 && height == 6 {
                        Some(Move::cell(3, 0))
                    } else {
                        None
                    },
                    source: if width == 7 && height == 6 {
                        SOURCE_STANDARD
                    } else {
                        SOURCE_TABLE
                    },
                });
            }
        }
    }
    None
}

fn symbolic_to_theory(result: SymbolicTheoryResult) -> TheoryResult {
    TheoryResult {
        outcome: result.outcome,
        plies_to_end: result
            .plies_to_end
            .and_then(|plies| i32::try_from(plies).ok()),
        proven_best_move: result.proven_best_move,
        source: result.source,
    }
}

fn single_column_alternating_draw(p: &Position) -> bool {
    let r = p.rules();
    if r.placement != Placement::Gravity || r.width != 1 || r.connect < 2 {
        return false;
    }
    let filled = p.column_count(0);
    (0..filled).all(|row| {
        p.owner_at(0, row)
            == Some(if row % 2 == 0 {
                Player::One
            } else {
                Player::Two
            })
    })
}

pub fn lookup(p: &Position) -> Option<TheoryResult> {
    let r = p.rules();
    if p.is_terminal() {
        return None;
    }
    let empties = r.cells() - p.moves_played() as i32;
    if r.connect > 0 && r.connect > r.max_line() {
        return Some(TheoryResult {
            outcome: Outcome::Draw,
            plies_to_end: Some(empties),
            proven_best_move: p.legal_moves().first().copied(),
            source: SOURCE_ELEMENTARY,
        });
    }
    if r.connect == 1 {
        return Some(TheoryResult {
            outcome: if r.misere {
                Outcome::Loss
            } else {
                Outcome::Win
            },
            plies_to_end: Some(1),
            proven_best_move: p.legal_moves().first().copied(),
            source: SOURCE_ELEMENTARY,
        });
    }
    if single_column_alternating_draw(p) {
        return Some(TheoryResult {
            outcome: Outcome::Draw,
            plies_to_end: Some(empties),
            proven_best_move: p.legal_moves().first().copied(),
            source: SOURCE_ELEMENTARY,
        });
    }
    if p.moves_played() == 0 {
        if let Some(result) = lookup_empty_symbolic(
            r.width,
            r.height,
            r.connect,
            r.placement,
            r.directions,
            r.misere,
        ) {
            return Some(symbolic_to_theory(result));
        }
    }
    if p.moves_played() == 1
        && r.placement == Placement::Gravity
        && r.connect == 4
        && !r.misere
        && r.directions == dirs::ALL
        && r.width == 7
        && r.height == 6
    {
        let mut first_column = -1;
        for c in 0..7 {
            if p.owner_at(c, 0) == Some(Player::One) {
                first_column = c;
                break;
            }
        }
        if first_column < 0 {
            return None;
        }
        let (outcome, end_ply) = match first_column {
            3 => (Outcome::Loss, 41),
            2 | 4 => (Outcome::Draw, 42),
            1 | 5 => (Outcome::Win, 42),
            _ => (Outcome::Win, 40),
        };
        return Some(TheoryResult {
            outcome,
            plies_to_end: Some(end_ply - 1),
            proven_best_move: None,
            source: SOURCE_STANDARD,
        });
    }
    None
}
