pub mod depth;
pub mod facade;
pub mod fast;
pub mod generic;
pub mod parallel;
pub mod standard;

use crate::book::OpeningBook;
use crate::model::{Move, Outcome, Placement};
use crate::position::Position;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Certainty {
    ProvenExact,
    ProvenByTheory,
    HeuristicDepthLimited,
    UnknownLimitReached,
}

impl Certainty {
    pub fn is_proof(self) -> bool {
        matches!(self, Certainty::ProvenExact | Certainty::ProvenByTheory)
    }
    pub fn name(self) -> &'static str {
        match self {
            Certainty::ProvenExact => "PROVEN_EXACT",
            Certainty::ProvenByTheory => "PROVEN_BY_THEORY",
            Certainty::HeuristicDepthLimited => "HEURISTIC_DEPTH_LIMITED",
            Certainty::UnknownLimitReached => "UNKNOWN_LIMIT_REACHED",
        }
    }
}

impl core::fmt::Display for Certainty {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", self.name())
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SolveMode {
    Auto,

    Exact,

    DepthLimited,
}

#[derive(Clone, Debug)]
pub struct SolveRequest {
    pub mode: SolveMode,
    pub max_depth: i32,
    pub time_limit_millis: u64,
    pub node_limit: u64,
    pub exact_time_limit_millis: u64,
    pub exact_node_limit: u64,
    pub use_theory: bool,
}

impl Default for SolveRequest {
    fn default() -> SolveRequest {
        SolveRequest::defaults()
    }
}

impl SolveRequest {
    pub fn defaults() -> SolveRequest {
        SolveRequest {
            mode: SolveMode::Auto,
            max_depth: 10,
            time_limit_millis: 5000,
            node_limit: 20_000_000,
            exact_time_limit_millis: 1500,
            exact_node_limit: 5_000_000,
            use_theory: true,
        }
    }
    pub fn exact(time_limit_millis: u64, node_limit: u64) -> SolveRequest {
        SolveRequest {
            mode: SolveMode::Exact,
            max_depth: 1,
            time_limit_millis,
            node_limit: 0,
            exact_time_limit_millis: time_limit_millis,
            exact_node_limit: node_limit,
            use_theory: true,
        }
    }
    pub fn automatic(position: &Position, use_theory: bool) -> SolveRequest {
        SolveRequest::automatic_with_book(position, use_theory, None)
    }
    pub fn automatic_with_book(
        position: &Position,
        use_theory: bool,
        book: Option<&OpeningBook>,
    ) -> SolveRequest {
        let rules = position.rules();
        let cells = rules.cells();
        let empties = (cells - position.moves_played() as i32).max(0);
        let branching = position.legal_moves().len() as i32;
        let mut total_millis = if cells <= 30 || empties <= 22 {
            3000
        } else if rules.placement == Placement::Free || branching > 9 || cells > 49 {
            5000
        } else {
            3000
        };
        let mut exact_millis = if fast::FastExactSolver::supports(rules) {
            if empties <= 28 || cells <= 30 {
                2000
            } else {
                800
            }
        } else if empties <= 18 {
            1000
        } else if empties <= 22 {
            300
        } else {
            0
        };
        let matching_book =
            book.filter(|book| book.rules().map(|r| r == position.rules()).unwrap_or(true));

        if matching_book
            .map(|book| {
                book.covers_ply(position.moves_played() as i32)
                    || standard::StandardSolver::supports(rules)
            })
            .unwrap_or(false)
        {
            exact_millis = 10_000;
            total_millis = 12_000;
        }
        SolveRequest {
            mode: SolveMode::Auto,
            max_depth: empties.clamp(1, 48),
            time_limit_millis: total_millis,
            node_limit: 0,
            exact_time_limit_millis: exact_millis,
            exact_node_limit: 0,
            use_theory,
        }
    }
    pub fn depth_limited(depth: i32, time_limit_millis: u64) -> SolveRequest {
        SolveRequest {
            mode: SolveMode::DepthLimited,
            max_depth: depth,
            time_limit_millis,
            node_limit: 0,
            exact_time_limit_millis: 0,
            exact_node_limit: 0,
            use_theory: true,
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.max_depth < 1 {
            return Err("maxDepth must be >= 1".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct SearchResult {
    pub certainty: Certainty,

    pub outcome: Option<Outcome>,
    pub best_move: Option<Move>,
    pub move_is_proven_optimal: bool,

    pub score: i32,
    pub plies_to_end: Option<i32>,
    pub depth_reached: i32,
    pub nodes: u64,
    pub elapsed_millis: u64,
    pub principal_line: Vec<Move>,
    pub explanation: String,
}

impl SearchResult {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        certainty: Certainty,
        outcome: Option<Outcome>,
        best_move: Option<Move>,
        move_is_proven_optimal: bool,
        score: i32,
        plies_to_end: Option<i32>,
        depth_reached: i32,
        nodes: u64,
        elapsed_millis: u64,
        principal_line: Vec<Move>,
        explanation: String,
    ) -> SearchResult {
        SearchResult {
            certainty,
            outcome,
            best_move,
            move_is_proven_optimal,
            score,
            plies_to_end,
            depth_reached,
            nodes,
            elapsed_millis,
            principal_line,
            explanation,
        }
    }
    pub fn unknown(nodes: u64, elapsed_millis: u64, explanation: String) -> SearchResult {
        SearchResult::new(
            Certainty::UnknownLimitReached,
            None,
            None,
            false,
            0,
            None,
            0,
            nodes,
            elapsed_millis,
            Vec::new(),
            explanation,
        )
    }
    pub fn is_proven_outcome(&self) -> bool {
        self.certainty.is_proof()
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum AbortReason {
    NodeLimit,
    TimeLimit,
    Cancelled,
}

impl AbortReason {
    pub fn name(self) -> &'static str {
        match self {
            AbortReason::NodeLimit => "NODE_LIMIT",
            AbortReason::TimeLimit => "TIME_LIMIT",
            AbortReason::Cancelled => "CANCELLED",
        }
    }
}

#[derive(Debug)]
pub(crate) struct Abort(pub AbortReason);

pub type ProgressFn = Arc<dyn Fn(u64, i32, Option<Move>) + Send + Sync>;

#[derive(Clone)]
pub struct SearchControl {
    pub node_limit: u64,
    pub deadline: Option<Instant>,
    pub cancelled: Option<Arc<AtomicBool>>,
    pub listener: Option<ProgressFn>,
}

impl Default for SearchControl {
    fn default() -> SearchControl {
        SearchControl::none()
    }
}

impl SearchControl {
    pub fn none() -> SearchControl {
        SearchControl {
            node_limit: 0,
            deadline: None,
            cancelled: None,
            listener: None,
        }
    }

    pub fn new(node_limit: u64, time_limit_millis: u64) -> SearchControl {
        SearchControl {
            node_limit,
            deadline: if time_limit_millis > 0 {
                Some(Instant::now() + Duration::from_millis(time_limit_millis))
            } else {
                None
            },
            cancelled: None,
            listener: None,
        }
    }
    pub fn with_cancel(mut self, flag: Arc<AtomicBool>) -> SearchControl {
        self.cancelled = Some(flag);
        self
    }
    pub fn with_listener(mut self, f: ProgressFn) -> SearchControl {
        self.listener = Some(f);
        self
    }
    pub fn is_cancelled(&self) -> bool {
        match &self.cancelled {
            Some(f) => f.load(Ordering::Relaxed),
            None => false,
        }
    }
    pub fn report(&self, nodes: u64, depth: i32, best: Option<Move>) {
        if let Some(f) = &self.listener {
            f(nodes, depth, best);
        }
    }

    #[inline]
    pub(crate) fn check_exact(&self, nodes: u64) -> Result<(), Abort> {
        if self.node_limit > 0 && nodes > self.node_limit {
            return Err(Abort(AbortReason::NodeLimit));
        }
        if nodes & 1023 == 0 {
            self.check_slow()?;
        }
        if nodes & 65535 == 0 {
            self.report(nodes, -1, None);
        }
        Ok(())
    }

    #[inline]
    pub(crate) fn check_depth(&self, nodes: u64) -> Result<(), Abort> {
        if self.node_limit > 0 && nodes > self.node_limit {
            return Err(Abort(AbortReason::NodeLimit));
        }
        if nodes & 1023 == 0 {
            self.check_slow()?;
        }
        Ok(())
    }
    fn check_slow(&self) -> Result<(), Abort> {
        if self.is_cancelled() {
            return Err(Abort(AbortReason::Cancelled));
        }
        if let Some(d) = self.deadline {
            if Instant::now() > d {
                return Err(Abort(AbortReason::TimeLimit));
            }
        }
        Ok(())
    }
    pub fn past_deadline(&self) -> bool {
        matches!(self.deadline, Some(d) if Instant::now() > d)
    }
}

#[inline]
pub(crate) fn stone_count_bounds(cells: i32, moves: i32, connect: i32) -> (i32, i32) {
    if connect <= 0 {
        return (-(cells + 1), cells + 1);
    }

    let cells = cells as i64;
    let moves = moves as i64;
    let connect = connect as i64;
    let side_stones = moves / 2;
    let opp_stones = moves - side_stones;
    let side_needed = (connect - side_stones).max(1);
    let side_final_ply = moves + 2 * side_needed - 1;
    let upper = if side_final_ply > cells {
        0
    } else {
        cells - side_final_ply + 1
    };
    let opp_needed = (connect - opp_stones).max(1);
    let opp_final_ply = moves + 2 * opp_needed;
    let lower = if opp_final_ply > cells {
        0
    } else {
        -(cells - opp_final_ply + 1)
    };
    (lower as i32, upper as i32)
}

pub fn center_out_order(width: i32) -> Vec<i32> {
    let mut cols: Vec<i32> = (0..width).collect();
    cols.sort_by_key(|&c| (2 * c - (width - 1)).abs());
    cols
}

impl core::fmt::Display for SearchResult {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let mut s = match self.certainty {
            Certainty::ProvenExact => "PROVEN (exact search)".to_string(),
            Certainty::ProvenByTheory => "PROVEN (theory)".to_string(),
            Certainty::HeuristicDepthLimited => {
                format!("Heuristic, depth {} (not a proof)", self.depth_reached)
            }
            Certainty::UnknownLimitReached => "Unknown (limit reached)".to_string(),
        };
        if let Some(o) = self.outcome {
            s.push_str(&format!(" | outcome for side to move: {o}"));
        }
        if let Some(m) = self.best_move {
            s.push_str(&format!(
                " | best move: {}{}",
                m.notation(),
                if self.move_is_proven_optimal {
                    " (proven optimal)"
                } else {
                    ""
                }
            ));
        }
        s.push_str(&format!(" | score: {}", self.score));
        if let Some(p) = self.plies_to_end {
            s.push_str(&format!(" | ends in {p} plies"));
        }
        s.push_str(&format!(
            " | nodes: {} | {} ms",
            self.nodes, self.elapsed_millis
        ));
        if !self.principal_line.is_empty() {
            s.push_str("\n  PV: ");
            let parts: Vec<String> = self.principal_line.iter().map(|m| m.notation()).collect();
            s.push_str(&parts.join(" "));
        }
        s.push_str("\n  ");
        s.push_str(&self.explanation);
        write!(f, "{s}")
    }
}
