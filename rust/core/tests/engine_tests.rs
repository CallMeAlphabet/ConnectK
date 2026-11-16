use connectk_core::book::OpeningBook;
use connectk_core::eval::Evaluator;
use connectk_core::model::{dirs, Move, Outcome, Placement, Player, Rules};
use connectk_core::position::Position;
use connectk_core::solver::depth::DepthLimitedSolver;
use connectk_core::solver::facade::ConnectKSolver;
use connectk_core::solver::fast::FastExactSolver;
use connectk_core::solver::generic::ExactSolver;
use connectk_core::solver::parallel::ParallelExactSolver;
use connectk_core::solver::standard::StandardSolver;
use connectk_core::solver::{Certainty, SearchControl, SearchResult, SolveRequest};
use connectk_core::theory;
use std::collections::BTreeMap;
use std::sync::Arc;

struct JavaRandom {
    seed: u64,
}

impl JavaRandom {
    fn new(seed: i64) -> JavaRandom {
        JavaRandom {
            seed: ((seed as u64) ^ 0x5DEE_CE66D) & ((1u64 << 48) - 1),
        }
    }
    fn next(&mut self, bits: u32) -> i32 {
        self.seed = (self.seed.wrapping_mul(0x5DEE_CE66D).wrapping_add(0xB)) & ((1u64 << 48) - 1);
        (self.seed >> (48 - bits)) as i32
    }
    fn next_int(&mut self, bound: i32) -> i32 {
        if bound > 0 && (bound as u32).is_power_of_two() {
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

fn play_cols(r: &Rules, cols: &[i32]) -> Position {
    let mut p = Position::initial(r);
    for &c in cols {
        p.play_column(c).unwrap();
    }
    p
}

fn random_position(r: &Rules, rng: &mut JavaRandom, plies: i32) -> Position {
    let mut p = Position::initial(r);
    let mut i = 0;
    while i < plies && !p.is_terminal() {
        let legal = p.legal_moves();
        if legal.is_empty() {
            break;
        }
        let mv = legal[rng.next_int(legal.len() as i32) as usize];
        p.play(mv).unwrap();
        i += 1;
    }
    p
}

fn exact(p: &Position, nodes: u64, millis: u64) -> SearchResult {
    ExactSolver::new().solve(p, SearchControl::new(nodes, millis))
}

fn fast_exact(p: &Position, nodes: u64, millis: u64) -> SearchResult {
    FastExactSolver::new().solve(p, SearchControl::new(nodes, millis))
}

fn standard_exact(p: &Position, nodes: u64, millis: u64) -> SearchResult {
    StandardSolver::new(0).solve(p, SearchControl::new(nodes, millis))
}

fn parallel_exact(p: &Position, threads: usize, nodes: u64, millis: u64) -> SearchResult {
    ParallelExactSolver::new(threads, None).solve(p, SearchControl::new(nodes, millis))
}

#[test]
fn gravity_play_undo() {
    let mut p = Position::initial(&Rules::standard_connect4());
    let m1 = p.play_column(3).unwrap();
    assert_eq!(0, m1.row, "first disc lands on row 0");
    assert_eq!(Some(Player::One), p.owner_at(3, 0), "owner of first disc");
    assert_eq!(Player::Two, p.side_to_move(), "turn passes");
    let m2 = p.play_column(3).unwrap();
    assert_eq!(1, m2.row, "second disc stacks");
    p.undo().unwrap();
    assert_eq!(1, p.moves_played(), "undo restores move count");
    assert!(p.is_empty(3, 1), "undo clears the cell");
    assert_eq!(Player::Two, p.side_to_move(), "undo restores side to move");
    let mut q = Position::initial(&Rules::standard_connect4());
    q.play_column(3).unwrap();
    assert_eq!(q.raw_key(), p.raw_key(), "play+undo equals direct play");
    assert_eq!(q.canonical_key(), p.canonical_key(), "keys equal");
}

#[test]
fn degenerate_and_large_rule_values() {
    assert!(Rules::gravity(5, 5, 10, true).is_ok());
    assert!(Rules::gravity(0, 6, 4, true).is_ok());
    assert!(Rules::gravity(6, 0, 4, true).is_ok());
    assert!(Rules::gravity(3, 3, 0, true).is_ok());
    assert!(Rules::gravity(-1, 6, 4, true).is_err());
    assert!(Rules::gravity(6, -1, 4, true).is_err());
    assert!(Rules::gravity(6, 6, -1, true).is_err());
    assert!(Rules::gravity(i32::MAX, i32::MAX, 4, true).is_err());

    let no_cells = Rules::gravity(0, 6, 4, true).unwrap();
    let p = Position::initial(&no_cells);
    assert!(p.is_terminal());
    assert_eq!(None, p.winner());
    assert!(p.is_draw());

    let k0 = Rules::new(3, 3, 0, Placement::Free, dirs::ALL, false).unwrap();
    let p = Position::initial(&k0);
    assert_eq!(Some(Player::One), p.winner());
    assert_eq!(Some(Outcome::Win), p.outcome_for_side_to_move());
    assert!(p.legal_moves().is_empty());
    let k0_misere = Rules::new(3, 3, 0, Placement::Free, dirs::ALL, true).unwrap();
    let p = Position::initial(&k0_misere);
    assert_eq!(Some(Player::Two), p.winner());
    assert_eq!(Some(Outcome::Loss), p.outcome_for_side_to_move());
}

#[test]
fn elementary_theory_edge_cases() {
    let k1 = Rules::free(2, 2, 1, true).unwrap();
    let p = Position::initial(&k1);
    let t = theory::lookup(&p).unwrap();
    assert_eq!(Outcome::Win, t.outcome);
    assert_eq!(Some(1), t.plies_to_end);
    assert!(t.proven_best_move.is_some());
    let k1_misere = Rules::new(2, 2, 1, Placement::Free, dirs::ALL, true).unwrap();
    let t = theory::lookup(&Position::initial(&k1_misere)).unwrap();
    assert_eq!(Outcome::Loss, t.outcome);

    let no_line = Rules::gravity(5, 5, 10, true).unwrap();
    let p = Position::initial(&no_line);
    assert!(!p.has_line(Player::One));
    assert_eq!(0, p.count_winning_spots(Player::One));
    let result = ConnectKSolver::new().solve(&p, &SolveRequest::defaults());
    assert_eq!(Certainty::ProvenByTheory, result.certainty);
    assert_eq!(Some(Outcome::Draw), result.outcome);
    assert_eq!(0, result.nodes);

    let single = Rules::gravity(1, 6, 4, true).unwrap();
    let p = play_cols(&single, &[0, 0, 0]);
    let t = theory::lookup(&p).unwrap();
    assert_eq!(Outcome::Draw, t.outcome);
    assert_eq!(Outcome::Draw, exact(&p, 0, 10_000).outcome.unwrap());
}

#[test]
fn pairing_and_mnk_theory_match_search_on_small_boards() {
    let pairing = Rules::free(5, 2, 3, true).unwrap();
    let p = Position::initial(&pairing);
    assert_eq!(Outcome::Draw, theory::lookup(&p).unwrap().outcome);
    assert_eq!(0, exact(&p, 0, 30_000).score);

    let vertical_only = Rules::new(2, 5, 3, Placement::Free, dirs::VERTICAL, false).unwrap();
    let p = Position::initial(&vertical_only);
    assert_eq!(Outcome::Draw, theory::lookup(&p).unwrap().outcome);
    assert_eq!(0, exact(&p, 0, 30_000).score);

    let mnk_win = Rules::free(4, 3, 3, true).unwrap();
    let p = Position::initial(&mnk_win);
    assert_eq!(Outcome::Win, theory::lookup(&p).unwrap().outcome);
    assert!(exact(&p, 0, 30_000).score > 0);

    let k2 = Rules::free(3, 3, 2, true).unwrap();
    let p = Position::initial(&k2);
    let t = theory::lookup(&p).unwrap();
    assert_eq!(Outcome::Win, t.outcome);
    assert_eq!(Some(3), t.plies_to_end);
    assert_eq!(7, exact(&p, 0, 10_000).score);

    let k2_gravity = Rules::gravity(2, 2, 2, true).unwrap();
    let p = Position::initial(&k2_gravity);
    assert_eq!(Outcome::Win, theory::lookup(&p).unwrap().outcome);
    assert!(exact(&p, 0, 10_000).score > 0);
}

#[test]
fn symbolic_theory_handles_i32_sized_boards() {
    let draw =
        theory::lookup_empty_symbolic(i32::MAX, i32::MAX, 9, Placement::Free, dirs::ALL, false)
            .unwrap();
    assert_eq!(Outcome::Draw, draw.outcome);
    assert_eq!(Some(i32::MAX as i64 * i32::MAX as i64), draw.plies_to_end);

    let win =
        theory::lookup_empty_symbolic(i32::MAX, i32::MAX, 3, Placement::Free, dirs::ALL, false)
            .unwrap();
    assert_eq!(Outcome::Win, win.outcome);

    let k2 = theory::lookup_empty_symbolic(i32::MAX, 2, 2, Placement::Gravity, dirs::ALL, false)
        .unwrap();
    assert_eq!(Outcome::Win, k2.outcome);
    assert_eq!(Some(Move::cell(1, 0)), k2.proven_best_move);

    let single =
        theory::lookup_empty_symbolic(1, i32::MAX, 4, Placement::Gravity, dirs::ALL, false)
            .unwrap();
    assert_eq!(Outcome::Draw, single.outcome);
}

#[test]
fn illegal_moves() {
    let mut p = Position::initial(&Rules::gravity(2, 2, 2, true).unwrap());
    p.play_column(0).unwrap();
    p.play_column(0).unwrap();
    assert!(p.play_column(0).is_err(), "full column rejected");
    let mut f = Position::initial(&Rules::free(3, 3, 3, true).unwrap());
    f.play(Move::cell(1, 1)).unwrap();
    assert!(f.play(Move::cell(1, 1)).is_err(), "occupied cell rejected");
    assert!(
        f.play(Move::column(1)).is_err(),
        "unresolved move rejected in free placement"
    );
}

#[test]
fn win_directions() {
    let r = Rules::standard_connect4();
    let v = play_cols(&r, &[0, 1, 0, 1, 0, 1, 0]);
    assert_eq!(Some(Player::One), v.winner(), "vertical win");
    let h = play_cols(&r, &[0, 0, 1, 1, 2, 2, 3]);
    assert_eq!(Some(Player::One), h.winner(), "horizontal win");
    let d = play_cols(&r, &[0, 1, 1, 2, 3, 2, 2, 3, 4, 3, 3]);
    assert_eq!(Some(Player::One), d.winner(), "diagonal win, board:\n{d}");
    assert_eq!(
        4,
        d.completed_line_cells().len(),
        "winning line cells reported"
    );
}

#[test]
fn diagonals_off() {
    let no_diag = Rules::new(4, 4, 4, Placement::Free, dirs::ORTHOGONAL, false).unwrap();
    let p = Position::from_board(&no_diag, &["XOO.", ".XO.", "..XO", "...X"]).unwrap();
    assert!(
        !p.has_line(Player::One),
        "diagonal must not count without diagonals"
    );
    let diag = Rules::new(4, 4, 4, Placement::Free, dirs::ALL, false).unwrap();
    let mut q = Position::initial(&diag);
    q.play(Move::cell(0, 0)).unwrap();
    q.play(Move::cell(3, 0)).unwrap();
    q.play(Move::cell(1, 1)).unwrap();
    q.play(Move::cell(3, 1)).unwrap();
    q.play(Move::cell(2, 2)).unwrap();
    q.play(Move::cell(0, 3)).unwrap();
    q.play(Move::cell(3, 3)).unwrap();
    assert_eq!(
        Some(Player::One),
        q.winner(),
        "same diagonal wins with diagonals on"
    );
}

#[test]
fn free_win() {
    let mut p = Position::initial(&Rules::free(5, 5, 4, true).unwrap());
    p.play(Move::cell(1, 1)).unwrap();
    p.play(Move::cell(0, 4)).unwrap();
    p.play(Move::cell(2, 2)).unwrap();
    p.play(Move::cell(0, 3)).unwrap();
    p.play(Move::cell(3, 3)).unwrap();
    p.play(Move::cell(4, 0)).unwrap();
    assert!(!p.is_terminal(), "three diagonal is not terminal");
    p.play(Move::cell(4, 4)).unwrap();
    assert_eq!(Some(Player::One), p.winner(), "free-placement diagonal win");
}

#[test]
fn multi_word_win() {
    let r = Rules::free(10, 7, 4, true).unwrap();
    assert!(r.bit_count() > 64, "board really is multi-word");
    let mut p = Position::initial(&r);
    p.play(Move::cell(0, 0)).unwrap();
    p.play(Move::cell(9, 6)).unwrap();
    p.play(Move::cell(1, 1)).unwrap();
    p.play(Move::cell(8, 6)).unwrap();
    p.play(Move::cell(2, 2)).unwrap();
    p.play(Move::cell(9, 5)).unwrap();
    p.play(Move::cell(3, 3)).unwrap();
    assert_eq!(Some(Player::One), p.winner(), "multi-word diagonal win");
}

#[test]
fn misere_loses() {
    let r = Rules::standard_connect4().with_misere(true);
    let p = play_cols(&r, &[0, 4, 1, 5, 2, 6, 3]);
    assert_eq!(Some(Player::One), p.line_maker(), "X made the line");
    assert_eq!(
        Some(Player::Two),
        p.winner(),
        "in misere the line maker loses"
    );
}

#[test]
fn from_board() {
    let r = Rules::standard_connect4();
    let p = Position::from_board(
        &r,
        &[
            ".......", ".......", ".......", ".......", ".......", "XXX.OOO",
        ],
    )
    .unwrap();
    assert_eq!(Player::One, p.side_to_move(), "equal counts -> X to move");
    assert!(
        p.line_completing_moves(Player::One)
            .contains(&Move::cell(3, 0)),
        "X can complete at column 4"
    );
    assert!(
        Position::from_board(
            &r,
            &[".......", ".......", ".......", ".......", "X......", "......."]
        )
        .is_err(),
        "floating stone rejected"
    );
    assert!(
        Position::from_board(
            &r,
            &[".......", ".......", ".......", ".......", ".......", "XX....."]
        )
        .is_err(),
        "impossible stone counts rejected"
    );
}

#[test]
fn mirror_key() {
    let r = Rules::standard_connect4();
    let a = play_cols(&r, &[0, 1, 2, 2]);
    let b = play_cols(&r, &[6, 5, 4, 4]);
    assert_eq!(
        a.canonical_key(),
        b.canonical_key(),
        "mirrored positions share canonical key"
    );
    assert_ne!(a.raw_key(), b.raw_key(), "raw keys differ");
}

#[test]
fn line_completing_matches_simulation() {
    let variants = [
        Rules::gravity(5, 4, 4, true).unwrap(),
        Rules::gravity(5, 4, 4, false).unwrap(),
        Rules::gravity(6, 5, 5, true).unwrap(),
        Rules::gravity(8, 6, 4, true).unwrap(),
        Rules::free(5, 5, 4, true).unwrap(),
    ];
    let mut rng = JavaRandom::new(42);
    for r in &variants {
        for _ in 0..40 {
            let plies = 3 + rng.next_int(8);
            let p = random_position(r, &mut rng, plies);
            if p.is_terminal() {
                continue;
            }
            let side = p.side_to_move();
            let reported = p.line_completing_moves(side);
            for m in p.legal_moves() {
                let mut q = p.clone();
                q.play(m).unwrap();
                let actually = q.has_line(side);
                assert_eq!(
                    actually,
                    reported.contains(&m),
                    "lineCompleting mismatch for {m} in\n{p}\nrules={r}"
                );
            }
        }
    }
}

#[test]
fn exact_win_in_one() {
    let p = Position::from_board(
        &Rules::standard_connect4(),
        &[
            ".......", ".......", ".......", ".......", ".......", "XXX.OOO",
        ],
    )
    .unwrap();
    let res = exact(&p, 1_000_000, 10_000);
    assert_eq!(Certainty::ProvenExact, res.certainty);
    assert_eq!(Some(Outcome::Win), res.outcome);
    assert_eq!(3, res.best_move.unwrap().column);
    assert_eq!(Some(1), res.plies_to_end);
    assert!(res.move_is_proven_optimal);
}

#[test]
fn exact_win_in_one_k5() {
    let r = Rules::gravity(7, 6, 5, true).unwrap();
    let p = play_cols(&r, &[0, 0, 1, 1, 2, 2, 3, 3]);
    let res = exact(&p, 1_000_000, 10_000);
    assert_eq!(Some(Outcome::Win), res.outcome);
    assert_eq!(4, res.best_move.unwrap().column);
    assert_eq!(Some(1), res.plies_to_end);
}

#[test]
fn exact_double_threat() {
    let p = play_cols(&Rules::standard_connect4(), &[6, 1, 5, 2, 6, 3]);
    let res = exact(&p, 1_000_000, 10_000);
    assert_eq!(Certainty::ProvenExact, res.certainty);
    assert_eq!(Some(Outcome::Loss), res.outcome);
    assert_eq!(Some(2), res.plies_to_end);
}

#[test]
fn exact_4x4_draw() {
    let p = Position::initial(&Rules::gravity(4, 4, 4, true).unwrap());
    let res = exact(&p, 20_000_000, 30_000);
    assert_eq!(
        Certainty::ProvenExact,
        res.certainty,
        "4x4 proven, got: {res:?}"
    );
    assert_eq!(
        Some(Outcome::Draw),
        res.outcome,
        "4x4 is a draw (published table)"
    );
}

#[test]
fn exact_tic_tac_toe() {
    let p = Position::initial(&Rules::free(3, 3, 3, true).unwrap());
    let res = exact(&p, 5_000_000, 20_000);
    assert_eq!(Some(Outcome::Draw), res.outcome, "tic-tac-toe draw");
}

#[test]
fn exact_budget_unknown() {
    let p = Position::initial(&Rules::free(5, 5, 4, true).unwrap());
    let res = exact(&p, 100, 20_000);
    assert_eq!(
        Certainty::UnknownLimitReached,
        res.certainty,
        "tiny budget -> unknown"
    );
    assert!(res.best_move.is_none(), "no move claimed");
}

fn brute(p: &mut Position, mut alpha: i32, beta: i32, cells: i32) -> i32 {
    if p.is_terminal() {
        return match p.winner() {
            None => 0,
            Some(w) => {
                let mag = cells - p.moves_played() as i32 + 1;
                if w == p.side_to_move() {
                    mag
                } else {
                    -mag
                }
            }
        };
    }
    let mut best = i32::MIN / 2;
    for m in p.legal_moves() {
        p.play(m).unwrap();
        let s = -brute(p, -beta, -alpha, cells);
        p.undo().unwrap();
        best = best.max(s);
        alpha = alpha.max(best);
        if alpha >= beta {
            break;
        }
    }
    best
}

#[test]
fn exact_vs_brute_force() {
    let variants = [
        Rules::gravity(4, 4, 4, true).unwrap(),
        Rules::gravity(4, 4, 4, false).unwrap(),
        Rules::gravity(5, 5, 5, true).unwrap(),
        Rules::gravity(4, 4, 4, true).unwrap().with_misere(true),
        Rules::new(4, 4, 3, Placement::Free, dirs::ALL, false).unwrap(),
    ];
    let mut rng = JavaRandom::new(7);
    let mut compared = 0;
    for r in &variants {
        for _ in 0..25 {
            let target = (r.cells() - 10).max(4);
            let p = random_position(r, &mut rng, target);
            if p.is_terminal() {
                continue;
            }
            let mut copy = p.clone();
            let expected = brute(&mut copy, -10_000, 10_000, r.cells());
            let res = exact(&p, 5_000_000, 20_000);
            assert_eq!(
                Certainty::ProvenExact,
                res.certainty,
                "exact must finish on small position\n{p}\nrules={r}"
            );
            assert_eq!(
                expected, res.score,
                "score mismatch vs brute force\n{p}\nrules={r}"
            );
            compared += 1;
        }
    }
    assert!(
        compared >= 50,
        "enough comparisons actually ran: {compared}"
    );
}

#[test]
fn fast_vs_generic() {
    assert!(FastExactSolver::supports(&Rules::standard_connect4()));
    assert!(!FastExactSolver::supports(
        &Rules::standard_connect4().with_misere(true)
    ));
    assert!(!FastExactSolver::supports(
        &Rules::new(4, 4, 3, Placement::Free, dirs::ALL, false).unwrap()
    ));
    assert!(!FastExactSolver::supports(
        &Rules::gravity(10, 7, 4, true).unwrap()
    ));

    let variants = [
        Rules::standard_connect4(),
        Rules::gravity(7, 6, 4, false).unwrap(),
        Rules::gravity(6, 5, 5, true).unwrap(),
        Rules::gravity(4, 4, 4, true).unwrap(),
        Rules::new(
            5,
            4,
            4,
            Placement::Gravity,
            dirs::VERTICAL | dirs::HORIZONTAL | dirs::DIAGONAL_UP,
            false,
        )
        .unwrap(),
    ];
    let mut rng = JavaRandom::new(99);
    let mut compared = 0;
    for r in &variants {
        assert!(
            FastExactSolver::supports(r),
            "variant should be fast-supported: {r}"
        );
        for _ in 0..20 {
            let target = (r.cells() - 12).max(4);
            let p = random_position(r, &mut rng, target);
            if p.is_terminal() {
                continue;
            }
            let generic = exact(&p, 20_000_000, 30_000);
            let fast = fast_exact(&p, 20_000_000, 30_000);
            assert_eq!(
                Certainty::ProvenExact,
                fast.certainty,
                "fast exact must finish\n{p}\nrules={r}"
            );
            assert_eq!(
                generic.score, fast.score,
                "fast/generic score mismatch\n{p}\nrules={r}"
            );
            assert_eq!(
                generic.outcome, fast.outcome,
                "fast/generic outcome mismatch\n{p}\nrules={r}"
            );
            let mut cursor = p.clone();
            for m in &fast.principal_line {
                cursor.play(*m).unwrap();
            }
            compared += 1;
        }
    }
    assert!(
        compared >= 60,
        "enough fast/generic comparisons ran: {compared}"
    );
}

#[test]
fn fast_known_results() {
    let draw = fast_exact(
        &Position::initial(&Rules::gravity(5, 4, 4, true).unwrap()),
        0,
        30_000,
    );
    assert_eq!(Certainty::ProvenExact, draw.certainty, "5x4 proven");
    assert_eq!(Some(Outcome::Draw), draw.outcome, "5x4 is a draw");

    let loss = fast_exact(
        &Position::initial(&Rules::gravity(6, 4, 4, true).unwrap()),
        0,
        60_000,
    );
    assert_eq!(Certainty::ProvenExact, loss.certainty, "6x4 proven");
    assert_eq!(
        Some(Outcome::Loss),
        loss.outcome,
        "6x4 is a second-player win"
    );

    let seq = [3, 4, 3, 3, 0, 4, 0, 2, 2, 4, 4, 2, 1, 1, 1, 2];
    let std = fast_exact(&play_cols(&Rules::standard_connect4(), &seq), 0, 60_000);
    assert_eq!(Some(Outcome::Loss), std.outcome, "standard: proven loss");
    assert_eq!(Some(6), std.plies_to_end, "standard: loss in 6 plies");
    let no_diag = fast_exact(
        &play_cols(&Rules::gravity(7, 6, 4, false).unwrap(), &seq),
        0,
        60_000,
    );
    assert_eq!(
        Some(Outcome::Draw),
        no_diag.outcome,
        "no-diagonals: proven draw"
    );
}

#[test]
fn standard_specialized_matches_generic_and_fast() {
    let r = Rules::standard_connect4();
    assert!(StandardSolver::supports(&r));
    assert!(!StandardSolver::supports(&r.with_misere(true)));
    assert!(!StandardSolver::supports(
        &Rules::gravity(7, 6, 4, false).unwrap()
    ));
    assert!(!StandardSolver::supports(
        &Rules::gravity(6, 5, 4, true).unwrap()
    ));

    let seq = [3, 4, 3, 3, 0, 4, 0, 2, 2, 4, 4, 2, 1, 1, 1, 2];
    let known = standard_exact(&play_cols(&r, &seq), 0, 60_000);
    assert_eq!(Certainty::ProvenExact, known.certainty);
    assert_eq!(Some(Outcome::Loss), known.outcome);
    assert_eq!(Some(6), known.plies_to_end);

    let immediate = Position::from_board(
        &r,
        &[
            ".......", ".......", ".......", ".......", ".......", "XXX.OOO",
        ],
    )
    .unwrap();
    let win = standard_exact(&immediate, 0, 60_000);
    assert_eq!(Some(Outcome::Win), win.outcome);
    assert_eq!(Some(1), win.plies_to_end);
    assert_eq!(3, win.best_move.unwrap().column);
    assert!(win.move_is_proven_optimal);

    let mut solver = StandardSolver::new(0);
    let mut rng = JavaRandom::new(2026);
    let mut compared = 0;
    for _ in 0..400 {
        if compared >= 24 {
            break;
        }
        let p = random_position(&r, &mut rng, 30);
        if p.is_terminal() {
            continue;
        }
        let generic = exact(&p, 20_000_000, 30_000);
        let fast = fast_exact(&p, 20_000_000, 30_000);
        let standard = solver.solve(&p, SearchControl::new(20_000_000, 30_000));
        assert_eq!(
            Certainty::ProvenExact,
            standard.certainty,
            "standard exact must finish\n{p}"
        );
        assert_eq!(
            generic.score, standard.score,
            "standard/generic score mismatch\n{p}"
        );
        assert_eq!(
            fast.score, standard.score,
            "standard/fast score mismatch\n{p}"
        );
        assert_eq!(generic.outcome, standard.outcome, "outcome mismatch\n{p}");
        assert!(
            standard.move_is_proven_optimal,
            "standard move certified\n{p}"
        );
        let best = standard.best_move.unwrap();
        let mut child = p.clone();
        child.play(best).unwrap();
        if !child.is_terminal() {
            let child_res = solver.solve(&child, SearchControl::new(20_000_000, 30_000));
            assert_eq!(
                -standard.score, child_res.score,
                "standard best move does not achieve its score\n{p}"
            );
        } else {
            assert_eq!(Some(1), standard.plies_to_end);
        }
        let at_score = solver.search_bound(
            &p,
            standard.score,
            standard.score + 1,
            SearchControl::new(20_000_000, 30_000),
        );
        assert!(!at_score.aborted && at_score.value <= standard.score);
        let below_score = solver.search_bound(
            &p,
            standard.score - 1,
            standard.score,
            SearchControl::new(20_000_000, 30_000),
        );
        assert!(!below_score.aborted && below_score.value >= standard.score);
        let mut cursor = p.clone();
        for m in &standard.principal_line {
            cursor.play(*m).unwrap();
        }
        compared += 1;
    }
    assert!(
        compared >= 20,
        "enough standard comparisons ran: {compared}"
    );
}

#[test]
fn standard_solver_uses_opening_book() {
    let r = Rules::standard_connect4();
    let immediate = Position::from_board(
        &r,
        &[
            ".......", ".......", ".......", ".......", ".......", "XXX.OOO",
        ],
    )
    .unwrap();
    let tiny = StandardSolver::new(1).solve(&immediate, SearchControl::new(0, 10_000));
    assert_eq!(Certainty::ProvenExact, tiny.certainty);
    assert_eq!(3, tiny.best_move.unwrap().column);

    let mut reps: BTreeMap<(u64, u64), Position> = BTreeMap::new();
    let mut rng = JavaRandom::new(77);
    for _ in 0..400 {
        if reps.len() >= 8 {
            break;
        }
        let pos = random_position(&r, &mut rng, 30);
        if pos.is_terminal() || pos.moves_played() != 30 {
            continue;
        }
        let (cur, mask, _) = FastExactSolver::encode_canonical(&pos);
        reps.entry((cur, mask)).or_insert(pos);
    }
    assert_eq!(8, reps.len(), "enough standard book positions generated");

    let mut solver = StandardSolver::new(0);
    let mut text = "# connectk-book v1 depth=30\n".to_string();
    let mut expected = Vec::new();
    for pos in reps.values() {
        let result = solver.solve(pos, SearchControl::new(0, 30_000));
        assert_eq!(Certainty::ProvenExact, result.certainty);
        let (cur, mask, flipped) = FastExactSolver::encode_canonical(pos);
        let best_col = result.best_move.unwrap().column;
        let canonical_col = if flipped { 6 - best_col } else { best_col };
        text.push_str(&format!(
            "{cur:016x} {mask:016x} {} {}\n",
            result.score, canonical_col
        ));
        expected.push((pos.clone(), result.score, best_col));
    }
    let book = Arc::new(OpeningBook::parse(&text).unwrap());
    let mut with_book = StandardSolver::new(0);
    with_book.set_opening_book(Some(book));
    for (pos, score, best_col) in expected {
        let result = with_book.solve(&pos, SearchControl::new(0, 30_000));
        assert_eq!(Certainty::ProvenExact, result.certainty);
        assert_eq!(score, result.score);
        assert_eq!(best_col, result.best_move.unwrap().column);
        assert_eq!(0, result.nodes, "book hit must not search");

        let mirrored_cols: Vec<i32> = pos.move_history().iter().map(|mv| 6 - mv.column).collect();
        let mirrored = play_cols(&r, &mirrored_cols);
        let mirrored_result = with_book.solve(&mirrored, SearchControl::new(0, 30_000));
        assert_eq!(score, mirrored_result.score);
        assert_eq!(
            6 - best_col,
            mirrored_result.best_move.unwrap().column,
            "mirrored book move"
        );
        assert_eq!(0, mirrored_result.nodes);
    }
}

fn top_up_to_even_columns(pos: &mut Position, rng: &mut JavaRandom) {
    for _ in 0..12 {
        if pos.is_terminal() {
            break;
        }
        let odd: Vec<i32> = (0..pos.rules().width)
            .filter(|&c| pos.column_count(c) % 2 == 1 && pos.column_count(c) < pos.rules().height)
            .collect();
        if odd.len() <= 1 {
            return;
        }
        let c = odd[rng.next_int(odd.len() as i32) as usize];
        pos.play_column(c).unwrap();
    }
}

fn check_claim_even_along_playouts(
    r: &Rules,
    rng: &mut JavaRandom,
    target_checked: i32,
) -> (i32, i32) {
    let (mut fired, mut negative, mut checked) = (0, 0, 0);
    for game in 0..600 {
        if checked >= target_checked {
            break;
        }
        let mut pos = Position::initial(r);
        let mut last_column = -1;
        let mode = game % 3;
        while !pos.is_terminal() {
            let empties = r.cells() - pos.moves_played() as i32;
            if (7..=14).contains(&empties) {
                let bound = FastExactSolver::claim_even_upper_bound(&pos);
                if bound != i32::MAX {
                    let ex = exact(&pos, 20_000_000, 30_000);
                    if ex.certainty == Certainty::ProvenExact {
                        assert!(
                            ex.score <= bound,
                            "claim-even bound violated: bound={bound} exact={}\n{pos}\nrules={r}",
                            ex.score
                        );
                        fired += 1;
                        checked += 1;
                        if bound < 0 {
                            negative += 1;
                        }
                        if checked >= target_checked {
                            break;
                        }
                    }
                }
            }
            let legal = pos.legal_moves();
            if legal.is_empty() {
                break;
            }
            let claimer_turn = (mode == 1 && pos.moves_played() % 2 == 1)
                || (mode == 2 && pos.moves_played().is_multiple_of(2));
            let mut chosen: Option<Move> = None;
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
    (fired, negative)
}

#[test]
fn claim_even_bound_sound() {
    assert_eq!(
        i32::MAX,
        FastExactSolver::claim_even_upper_bound(&Position::initial(&Rules::standard_connect4())),
        "no claim-even cap on the empty board"
    );
    assert_eq!(
        i32::MAX,
        FastExactSolver::claim_even_upper_bound(&play_cols(
            &Rules::gravity(6, 5, 4, true).unwrap(),
            &[2, 2, 3]
        )),
        "odd height -> no bound"
    );

    let mut rng = JavaRandom::new(2026);
    let (mut fired, mut negative) = (0, 0);
    for r in [
        Rules::standard_connect4(),
        Rules::gravity(7, 6, 4, false).unwrap(),
        Rules::gravity(7, 6, 5, true).unwrap(),
    ] {
        let (f, n) = check_claim_even_along_playouts(&r, &mut rng, 12);
        fired += f;
        negative += n;
    }
    for r in [
        Rules::gravity(6, 6, 5, true).unwrap(),
        Rules::gravity(6, 4, 4, true).unwrap(),
        Rules::gravity(5, 4, 4, false).unwrap(),
    ] {
        for trial in 0..60 {
            let mut pos = random_position(&r, &mut rng, (r.cells() - 12).max(6));
            if trial % 2 == 0 {
                top_up_to_even_columns(&mut pos, &mut rng);
            }
            if pos.is_terminal() {
                continue;
            }
            let bound = FastExactSolver::claim_even_upper_bound(&pos);
            if bound == i32::MAX {
                continue;
            }
            let ex = exact(&pos, 20_000_000, 30_000);
            assert_eq!(
                Certainty::ProvenExact,
                ex.certainty,
                "reference exact must finish\n{pos}\nrules={r}"
            );
            assert!(
                ex.score <= bound,
                "claim-even bound violated: bound={bound} exact={}\n{pos}\nrules={r}",
                ex.score
            );
            fired += 1;
            if bound < 0 {
                negative += 1;
            }
        }
    }
    for r in [
        Rules::standard_connect4(),
        Rules::gravity(7, 6, 4, false).unwrap(),
        Rules::gravity(6, 6, 5, true).unwrap(),
        Rules::gravity(6, 4, 4, true).unwrap(),
    ] {
        for _ in 0..10 {
            let pos = random_position(&r, &mut rng, (r.cells() - 13).max(6));
            if pos.is_terminal() {
                continue;
            }
            assert_eq!(
                exact(&pos, 20_000_000, 30_000).score,
                fast_exact(&pos, 20_000_000, 30_000).score,
                "fast(claim-even)/generic mismatch\n{pos}\nrules={r}"
            );
        }
    }
    assert!(fired >= 25, "bound exercised often enough: fired={fired}");
    assert!(
        negative >= 3,
        "negative (claimer-win) caps exercised: {negative}"
    );
}

#[test]
fn parallel_vs_sequential() {
    let draw = parallel_exact(
        &Position::initial(&Rules::gravity(5, 4, 4, true).unwrap()),
        2,
        0,
        30_000,
    );
    assert_eq!(
        Certainty::ProvenExact,
        draw.certainty,
        "parallel 5x4 proven"
    );
    assert_eq!(Some(Outcome::Draw), draw.outcome, "parallel 5x4 draw");
    let loss = parallel_exact(
        &Position::initial(&Rules::gravity(6, 4, 4, true).unwrap()),
        2,
        0,
        60_000,
    );
    assert_eq!(Some(Outcome::Loss), loss.outcome, "parallel 6x4 loss");

    let ttt = parallel_exact(
        &Position::initial(&Rules::new(3, 3, 3, Placement::Free, dirs::ALL, false).unwrap()),
        2,
        0,
        30_000,
    );
    assert_eq!(
        Some(Outcome::Draw),
        ttt.outcome,
        "parallel tic-tac-toe draw"
    );

    let variants = [
        Rules::standard_connect4(),
        Rules::gravity(7, 6, 4, false).unwrap(),
        Rules::gravity(5, 4, 4, true).unwrap().with_misere(true),
        Rules::new(4, 3, 3, Placement::Free, dirs::ALL, false).unwrap(),
    ];
    let mut rng = JavaRandom::new(7);
    let mut compared = 0;
    for r in &variants {
        for _ in 0..8 {
            let target = (r.cells() - 12).max(4);
            let pos = random_position(r, &mut rng, target);
            if pos.is_terminal() {
                continue;
            }
            let seq = parallel_exact(&pos, 1, 20_000_000, 30_000);
            let par = parallel_exact(&pos, 2, 20_000_000, 30_000);
            assert_eq!(
                Certainty::ProvenExact,
                par.certainty,
                "parallel must finish\n{pos}\nrules={r}"
            );
            assert_eq!(
                seq.score, par.score,
                "parallel/sequential score mismatch\n{pos}\nrules={r}"
            );
            let mut cursor = pos.clone();
            for m in &par.principal_line {
                cursor.play(*m).unwrap();
            }
            compared += 1;
        }
    }
    assert!(
        compared >= 20,
        "enough parallel comparisons ran: {compared}"
    );
}

#[test]
fn parallel_tiny_budget_unknown() {
    let p = play_cols(&Rules::standard_connect4(), &[3, 3]);
    let res = parallel_exact(&p, 2, 0, 1);
    assert_eq!(
        Certainty::UnknownLimitReached,
        res.certainty,
        "1 ms parallel budget -> unknown, got {res:?}"
    );
}

#[test]
fn opening_book_round_trip() {
    let r = Rules::gravity(5, 4, 4, true).unwrap();
    let book_depth = 6;
    let mut reps: BTreeMap<(u64, u64), Position> = BTreeMap::new();
    let mut rng = JavaRandom::new(42);
    while reps.len() < 40 {
        let pos = random_position(&r, &mut rng, book_depth);
        if pos.is_terminal() || pos.moves_played() as i32 != book_depth {
            continue;
        }
        let (cur, mask, _) = FastExactSolver::encode_canonical(&pos);
        reps.entry((cur, mask)).or_insert(pos);
    }
    let mut text = format!("# connectk-book v1 depth={book_depth}\n");
    let mut expected: BTreeMap<(u64, u64), SearchResult> = BTreeMap::new();
    for (key, pos) in &reps {
        let res = fast_exact(pos, 0, 30_000);
        assert_eq!(
            Certainty::ProvenExact,
            res.certainty,
            "bookgen position proven"
        );
        let (cur, mask, flipped) = FastExactSolver::encode_canonical(pos);
        let col = res.best_move.unwrap().column;
        let canonical_col = if flipped { r.width - 1 - col } else { col };
        text.push_str(&format!(
            "{cur:016x} {mask:016x} {} {}\n",
            res.score, canonical_col
        ));
        expected.insert(*key, res);
    }
    let book = OpeningBook::parse(&text).unwrap();
    assert_eq!(reps.len(), book.size(), "book entry count");
    assert_eq!(book_depth, book.depth(), "book depth");
    let book = Arc::new(book);
    for (key, pos) in &reps {
        let mut with_book = FastExactSolver::new();
        with_book.set_opening_book(Some(book.clone()));
        let res = with_book.solve(pos, SearchControl::new(0, 30_000));
        let exp = &expected[key];
        assert_eq!(Certainty::ProvenExact, res.certainty, "book hit proven");
        assert_eq!(exp.score, res.score, "book score matches search");
        assert!(res.move_is_proven_optimal, "book move certified optimal");
        assert!(
            res.nodes <= 2,
            "book hit should not search, nodes={}",
            res.nodes
        );
        let mut child = pos.clone();
        child.play(res.best_move.unwrap()).unwrap();
        if !child.is_terminal() {
            let child_res = fast_exact(&child, 0, 30_000);
            assert_eq!(-exp.score, child_res.score, "book move achieves the score");
        }
    }
    let mut with_book = FastExactSolver::new();
    with_book.set_opening_book(Some(book));
    let empty = with_book.solve(&Position::initial(&r), SearchControl::new(0, 30_000));
    assert_eq!(
        Some(Outcome::Draw),
        empty.outcome,
        "5x4 still a draw with book attached"
    );
}

#[test]
fn depth_takes_win() {
    let p = Position::from_board(
        &Rules::standard_connect4(),
        &[
            ".......", ".......", ".......", ".......", ".......", "XXX.OOO",
        ],
    )
    .unwrap();
    let res = DepthLimitedSolver::new().solve(&p, 4, SearchControl::new(0, 10_000));
    assert_eq!(
        3,
        res.best_move.unwrap().column,
        "depth search takes the win"
    );
}

#[test]
fn depth_blocks() {
    let p = play_cols(&Rules::standard_connect4(), &[0, 4, 0, 5, 1, 6]);
    let res = DepthLimitedSolver::new().solve(&p, 5, SearchControl::new(0, 10_000));
    assert_eq!(
        3,
        res.best_move.unwrap().column,
        "depth search blocks the threat, pv={:?}",
        res.principal_line
    );
}

#[test]
fn depth_labelled_heuristic() {
    let p = Position::initial(&Rules::standard_connect4());
    let res = ConnectKSolver::new().solve(&p, &SolveRequest::depth_limited(6, 10_000));
    assert_eq!(
        Certainty::HeuristicDepthLimited,
        res.certainty,
        "labelled heuristic"
    );
    assert!(!res.move_is_proven_optimal, "move not claimed proven");
    assert!(res.best_move.is_some(), "a move is still recommended");
}

#[test]
fn theory_standard() {
    let t = connectk_core::theory::lookup(&Position::initial(&Rules::standard_connect4())).unwrap();
    assert_eq!(Outcome::Win, t.outcome, "first player wins 7x6");
    assert_eq!(Some(41), t.plies_to_end, "win on ply 41");
    assert_eq!(3, t.proven_best_move.unwrap().column, "centre opening");
}

#[test]
fn theory_8x8() {
    let r = Rules::gravity(8, 8, 4, true).unwrap();
    let t = connectk_core::theory::lookup(&Position::initial(&r)).unwrap();
    assert_eq!(Outcome::Loss, t.outcome, "8x8: first player loses");
}

#[test]
fn theory_after_corner() {
    let p = play_cols(&Rules::standard_connect4(), &[0]);
    let t = connectk_core::theory::lookup(&p).unwrap();
    assert_eq!(
        Outcome::Win,
        t.outcome,
        "after corner opening, player to move (P2) wins"
    );
    assert_eq!(Some(39), t.plies_to_end, "game ends on ply 40");
}

#[test]
fn theory_not_for_no_diagonals() {
    assert!(
        connectk_core::theory::lookup(&Position::initial(&Rules::gravity(7, 6, 4, false).unwrap()))
            .is_none(),
        "no theory claimed for no-diagonals variant"
    );
}

#[test]
fn facade_auto_theory() {
    let res = ConnectKSolver::new().solve(
        &Position::initial(&Rules::standard_connect4()),
        &SolveRequest::defaults(),
    );
    assert_eq!(Certainty::ProvenByTheory, res.certainty, "theory certainty");
    assert_eq!(Some(Outcome::Win), res.outcome);
    assert_eq!(3, res.best_move.unwrap().column, "centre");
    assert!(res.move_is_proven_optimal, "theory move proven");
    assert_eq!(0, res.nodes, "no search needed");
}

#[test]
fn shipped_standard_book_is_discovered_and_auto_plays_perfectly() {
    let rules = Rules::standard_connect4();
    let bundled_books = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets");
    assert!(bundled_books.join("std7x6-d10.book").is_file());
    assert!(OpeningBook::candidate_directories().contains(&bundled_books));
    let (path, book) = OpeningBook::discover_for_rules(&rules)
        .expect("tracked standard depth-10 book must be discovered automatically");
    assert_eq!(
        Some("std7x6-d10.book"),
        path.file_name().and_then(|name| name.to_str()),
        "GUI/CLI auto-discovery must find the shipped standard book"
    );
    assert_eq!(809_464, book.size(), "complete depth-10 frontier");
    assert_eq!(10, book.depth());

    let book = Arc::new(book);
    let solver = ConnectKSolver::new();
    let auto_book = solver
        .book_for_rules(&rules)
        .expect("the default solver must auto-discover the shipped book");
    assert_eq!(809_464, auto_book.size());
    assert_eq!(10, auto_book.depth());
    assert!(
        ConnectKSolver::without_book()
            .book_for_rules(&rules)
            .is_none(),
        "explicit book-free construction must stay book-free"
    );
    let mut position = Position::initial(&rules);
    while !position.is_terminal() {
        let request = SolveRequest::automatic_with_book(&position, true, Some(&book));
        let result = solver.solve(&position, &request);
        assert!(
            result.certainty.is_proof() && result.move_is_proven_optimal,
            "AUTO must play a proven move at ply {}:\n{}\n{} ({})",
            position.moves_played(),
            position,
            result.explanation,
            result.certainty
        );
        position.play(result.best_move.unwrap()).unwrap();
    }
    assert_eq!(41, position.moves_played(), "perfect standard game length");
    assert_eq!(
        Some(Player::One),
        position.winner(),
        "the first player must win perfect AI-vs-AI standard play"
    );
}

#[test]
fn facade_exact() {
    let p = Position::from_board(
        &Rules::standard_connect4(),
        &[
            ".......", ".......", ".......", ".......", ".......", "XXX.OOO",
        ],
    )
    .unwrap();
    let res = ConnectKSolver::new().solve(&p, &SolveRequest::exact(10_000, 1_000_000));
    assert_eq!(Certainty::ProvenExact, res.certainty, "proven by search");
    assert_eq!(3, res.best_move.unwrap().column, "winning move");
}

#[test]
fn evaluator_basics() {
    let r = Rules::standard_connect4();
    let ev = Evaluator::new(&r);
    assert_eq!(69, ev.window_count(), "7x6 K4 has 69 potential lines");
    let empty = Position::initial(&r);
    assert_eq!(0, ev.evaluate(&empty), "empty board evaluates to 0");
    let terminal = Position::from_board(
        &r,
        &[
            ".......", ".......", ".......", ".......", ".......", "XXXXOOO",
        ],
    )
    .unwrap();
    let s = ev.evaluate(&terminal);
    assert!(s < 0, "side to move has lost");
    assert!(
        connectk_core::eval::is_losing_score(s),
        "terminal magnitude"
    );
}

#[test]
fn winning_spots_match_brute_force() {
    let mut rng = JavaRandom::new(1234);
    for r in [
        Rules::standard_connect4(),
        Rules::gravity(6, 5, 4, false).unwrap(),
        Rules::free(6, 5, 4, true).unwrap(),
        Rules::gravity(9, 7, 4, true).unwrap(),
    ] {
        for _ in 0..25 {
            let p = random_position(&r, &mut rng, 10);
            if p.is_terminal() {
                continue;
            }
            for player in [Player::One, Player::Two] {
                let spots = p.winning_spots(player);
                let mut count = 0;
                for c in 0..r.width {
                    for row in 0..r.height {
                        if !p.is_empty(c, row) {
                            continue;
                        }
                        let idx = r.index(c, row);
                        let in_spots = connectk_core::bits::test(&spots, idx);

                        let mut stones = p.stone_bits(player);
                        connectk_core::bits::set(&mut stones, idx);
                        let wins = brute_line(&r, &stones);
                        assert_eq!(
                            wins, in_spots,
                            "winning spot mismatch at {c}:{row} player {player}\n{p}\nrules={r}"
                        );
                        count += in_spots as u32;
                    }
                }
                assert_eq!(count, p.count_winning_spots(player));
            }
        }
    }
}

fn brute_line(r: &Rules, stones: &[u64]) -> bool {
    let k = r.connect;
    for c in 0..r.width {
        for row in 0..r.height {
            for &(dc, dr) in &[(0, 1), (1, 0), (1, 1), (1, -1)] {
                let bit = match (dc, dr) {
                    (0, 1) => dirs::VERTICAL,
                    (1, 0) => dirs::HORIZONTAL,
                    (1, 1) => dirs::DIAGONAL_UP,
                    _ => dirs::DIAGONAL_DOWN,
                };
                if r.directions & bit == 0 {
                    continue;
                }
                let mut all = true;
                for i in 0..k {
                    let (cc, rr) = (c + dc * i, row + dr * i);
                    if cc < 0
                        || cc >= r.width
                        || rr < 0
                        || rr >= r.height
                        || !connectk_core::bits::test(stones, r.index(cc, rr))
                    {
                        all = false;
                        break;
                    }
                }
                if all {
                    return true;
                }
            }
        }
    }
    false
}

#[test]
fn canonical_single_matches_position_key() {
    let mut rng = JavaRandom::new(5);
    for r in [
        Rules::standard_connect4(),
        Rules::gravity(7, 6, 4, false).unwrap(),
        Rules::new(
            5,
            4,
            4,
            Placement::Gravity,
            dirs::VERTICAL | dirs::HORIZONTAL | dirs::DIAGONAL_UP,
            false,
        )
        .unwrap(),
    ] {
        for _ in 0..30 {
            let p = random_position(&r, &mut rng, 12);
            let (cur, mask, _) = FastExactSolver::encode_canonical(&p);
            let key = p.canonical_key();
            assert_eq!(cur, key.current[0], "canonical cur mismatch\n{p}");
            assert_eq!(mask, key.mask[0], "canonical mask mismatch\n{p}");
        }
    }
}

#[test]
fn universal_book_sections_and_generic_solver() {
    use connectk_core::book::BookEntry;

    fn entry_for(
        pos: &Position,
    ) -> (
        connectk_core::position::PositionKey,
        BookEntry,
        SearchResult,
    ) {
        let result = exact(pos, 0, 30_000);
        assert_eq!(
            Certainty::ProvenExact,
            result.certainty,
            "position solved\n{pos}"
        );
        let key = pos.canonical_key();
        let flipped = key != pos.raw_key();
        let best = result.best_move.map(|m| {
            if flipped {
                Move::cell(pos.rules().width - 1 - m.column, m.row)
            } else {
                m
            }
        });
        (
            key,
            BookEntry {
                score: result.score,
                best_move: best,
            },
            result,
        )
    }

    let free_rules = Rules::new(3, 3, 3, Placement::Free, dirs::ALL, false).unwrap();
    let mut free_pos = Position::initial(&free_rules);
    for mv in [
        Move::cell(1, 1),
        Move::cell(0, 0),
        Move::cell(2, 0),
        Move::cell(0, 2),
    ] {
        free_pos.play(mv).unwrap();
    }
    let (free_key, free_entry, free_expected) = entry_for(&free_pos);

    let misere_rules = Rules::gravity(4, 4, 4, true).unwrap().with_misere(true);
    let misere_pos = play_cols(&misere_rules, &[0, 1, 2, 3]);
    let (misere_key, misere_entry, misere_expected) = entry_for(&misere_pos);

    let free_book = OpeningBook::from_entries(
        vec![(free_key.clone(), free_entry)],
        4,
        Some(free_rules.clone()),
    )
    .unwrap();
    let misere_book = OpeningBook::from_entries(
        vec![(misere_key.clone(), misere_entry)],
        4,
        Some(misere_rules.clone()),
    )
    .unwrap();

    let path = std::env::temp_dir().join(format!(
        "connectk-universal-test-{}.book",
        std::process::id()
    ));
    OpeningBook::save_universal(&path, &[free_book, misere_book]).unwrap();

    let sections = OpeningBook::load_sections(&path).unwrap();
    assert_eq!(2, sections.len(), "two universal sections");

    for (rules, pos, expected, key) in [
        (&free_rules, &free_pos, &free_expected, &free_key),
        (&misere_rules, &misere_pos, &misere_expected, &misere_key),
    ] {
        let reconstructed = Position::from_words(rules, &key.current, &key.mask).unwrap();
        assert_eq!(key, &reconstructed.canonical_key(), "from_words round trip");

        let book = OpeningBook::load_for_rules(&path, rules)
            .unwrap()
            .expect("matching universal section");
        assert_eq!(1, book.size());
        let mut solver = ExactSolver::new();
        solver.set_opening_book(Some(Arc::new(book)));
        let got = solver.solve(pos, SearchControl::new(0, 30_000));
        assert_eq!(Certainty::ProvenExact, got.certainty);
        assert_eq!(expected.score, got.score);
        assert_eq!(expected.best_move, got.best_move);
        assert_eq!(1, got.nodes, "root book hit\n{pos}");
    }

    assert!(
        OpeningBook::load_for_rules(&path, &Rules::standard_connect4())
            .unwrap()
            .is_none(),
        "no accidental cross-rules hit"
    );
    let _ = std::fs::remove_file(&path);
}

#[test]
fn existing_books_load_and_hit() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../books/std5x4-d12.book");
    if !path.exists() {
        return;
    }
    let book = Arc::new(OpeningBook::load(&path).unwrap());
    assert!(book.size() > 0);
    assert_eq!(12, book.depth());
    let r = Rules::gravity(5, 4, 4, true).unwrap();
    let mut solver = FastExactSolver::new();
    solver.set_opening_book(Some(book));
    let res = solver.solve(&Position::initial(&r), SearchControl::new(0, 30_000));
    assert_eq!(Certainty::ProvenExact, res.certainty);
    assert_eq!(
        Some(Outcome::Draw),
        res.outcome,
        "5x4 draw with Java-generated book"
    );
}
