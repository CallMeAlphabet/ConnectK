use connectk_core::model::Rules;
use connectk_core::position::Position;
use connectk_core::solver::depth::DepthLimitedSolver;
use connectk_core::solver::SearchControl;

fn from_digits(digits: &str) -> Position {
    let mut p = Position::initial(&Rules::standard_connect4());
    for ch in digits.chars() {
        p.play_column(ch as i32 - '1' as i32).unwrap();
    }
    p
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let depth: i32 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(12);
    for (name, pos) in [
        ("empty", from_digits("")),
        ("after 4", from_digits("4")),
        ("44337", from_digits("44337")),
    ] {
        let r = DepthLimitedSolver::new().solve(&pos, depth, SearchControl::new(0, 600_000));
        let nps = if r.elapsed_millis > 0 {
            r.nodes as f64 * 1000.0 / r.elapsed_millis as f64
        } else {
            0.0
        };
        println!(
            "{name}: depth {} score {} best {} nodes {} in {} ms ({:.0} nodes/s)",
            r.depth_reached,
            r.score,
            r.best_move.map(|m| m.notation()).unwrap_or_default(),
            r.nodes,
            r.elapsed_millis,
            nps
        );
    }
}
