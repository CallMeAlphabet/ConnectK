//! Published mathematical results, encoded as data. Covers the
//! classical gravity game (k=4, all directions, normal rules):
//!
//! * the game-theoretic value of the empty board for a range of sizes
//!   (Tromp, solved 2006 for width+height <= 15 and 8×8 in 2015;
//!   extended by Christophe Steininger to width+height = 16 and beyond),
//! * the exact first-move values of the standard 7×6 game
//!   (Allen / Allis, 1995/1988; exact scores from modern strong solvers).
//!
//! Anything not in this database is *not* claimed as proven here — the
//! solver will either prove it by search or label its answer heuristic.
//! See docs/THEORY.md for references.

use crate::model::{dirs, Move, Outcome, Placement, Player, Rules};
use crate::position::Position;

/// A published, mathematically proven result about a position.
#[derive(Clone, Debug)]
pub struct TheoryResult {
    /// Proven outcome for the side to move under perfect play.
    pub outcome: Outcome,
    /// Plies until the game ends under perfect play, if known.
    pub plies_to_end: Option<i32>,
    /// A move known to achieve the outcome, if published.
    pub proven_best_move: Option<Move>,
    /// Where the result comes from.
    pub source: &'static str,
}

const SOURCE_TABLE: &str =
    "Published board-size table (Tromp 2006/2015; Christophe Steininger) - see docs/THEORY.md";
const SOURCE_STANDARD: &str = "Standard 7x6 solution (Allen / Allis 1988; exact \
                               scores verified by modern strong solvers)";

// (width, height, outcome for player one, last-move ply or -1 unknown),
// sorted by (width, height) for binary search.
static EMPTY_BOARD: &[(i32, i32, Outcome, i32)] = &[
    (4, 4, Outcome::Draw, 16), (4, 5, Outcome::Draw, 20), (4, 6, Outcome::Draw, 24),
    (4, 7, Outcome::Draw, 28), (4, 8, Outcome::Draw, 32), (4, 9, Outcome::Draw, 36),
    (4, 10, Outcome::Draw, 40), (4, 11, Outcome::Draw, 44), (4, 12, Outcome::Draw, 48),
    (4, 13, Outcome::Draw, 52),
    (5, 4, Outcome::Draw, 20), (5, 5, Outcome::Draw, 25), (5, 6, Outcome::Draw, 30),
    (5, 7, Outcome::Draw, 35), (5, 8, Outcome::Draw, 40), (5, 9, Outcome::Draw, 45),
    (5, 10, Outcome::Draw, 50), (5, 11, Outcome::Draw, 55), (5, 12, Outcome::Draw, 60),
    (5, 13, Outcome::Draw, 65),
    (6, 4, Outcome::Loss, 24), (6, 5, Outcome::Draw, 30), (6, 6, Outcome::Loss, 36),
    (6, 7, Outcome::Win, 41), (6, 8, Outcome::Loss, 48), (6, 9, Outcome::Win, 53),
    (6, 10, Outcome::Loss, 60), (6, 11, Outcome::Win, 63), (6, 12, Outcome::Loss, 72),
    (7, 4, Outcome::Draw, 28), (7, 5, Outcome::Draw, 35), (7, 6, Outcome::Win, 41),
    (7, 7, Outcome::Draw, 49), (7, 8, Outcome::Win, 55), (7, 9, Outcome::Draw, 63),
    (7, 10, Outcome::Win, 69),
    (8, 4, Outcome::Loss, 32), (8, 5, Outcome::Win, 39), (8, 6, Outcome::Loss, 48),
    (8, 7, Outcome::Win, 53), (8, 8, Outcome::Loss, 62), (8, 9, Outcome::Win, -1),
    (9, 4, Outcome::Loss, 36), (9, 5, Outcome::Win, 41), (9, 6, Outcome::Loss, 52),
    (9, 7, Outcome::Win, 55),
    (10, 4, Outcome::Loss, 40), (10, 5, Outcome::Win, 47), (10, 6, Outcome::Win, 59),
    (11, 4, Outcome::Loss, 44), (11, 5, Outcome::Win, 51),
    (12, 4, Outcome::Loss, 48),
];

fn applies(r: &Rules) -> bool {
    r.placement == Placement::Gravity && r.connect == 4 && !r.misere && r.directions == dirs::ALL
}

/// Published result for this position, if any.
pub fn lookup(p: &Position) -> Option<TheoryResult> {
    let r = p.rules();
    if !applies(r) || p.is_terminal() {
        return None;
    }
    if p.moves_played() == 0 {
        let idx = EMPTY_BOARD
            .binary_search_by(|e| (e.0, e.1).cmp(&(r.width, r.height)))
            .ok()?;
        let (_, _, outcome, last_ply) = EMPTY_BOARD[idx];
        let best = if r.width == 7 && r.height == 6 {
            Some(Move::cell(3, 0)) // the unique winning opening
        } else {
            None
        };
        return Some(TheoryResult {
            outcome,
            plies_to_end: if last_ply > 0 { Some(last_ply) } else { None },
            proven_best_move: best,
            source: if r.width == 7 && r.height == 6 {
                SOURCE_STANDARD
            } else {
                SOURCE_TABLE
            },
        });
    }
    // Standard 7x6 after exactly one move: exact values are published.
    if p.moves_played() == 1 && r.width == 7 && r.height == 6 {
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
        // Outcome for the side to move (player two) and the final ply.
        let (outcome, end_ply) = match first_column {
            3 => (Outcome::Loss, 41),     // centre: first player wins
            2 | 4 => (Outcome::Draw, 42),
            1 | 5 => (Outcome::Win, 42),  // second player wins
            _ => (Outcome::Win, 40),      // columns 1 and 7 (0-based 0,6)
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
