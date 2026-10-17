//! Solvers. Exact search returns mathematical proof (the score IS the
//! game value); the depth-limited solver is always labelled heuristic.
//!
//! Score convention: `±(cells − finalPly + 1)`, 0 = draw, from the side
//! to move. (Pons score = sign · ceil(|ours| / 2).)

pub mod depth;
pub mod facade;
pub mod fast;
pub mod generic;
pub mod parallel;

use crate::model::{Move, Outcome};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// How certain a result is. Only the first two are proofs.
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
    /// Theory where published, exact proof when tractable, else
    /// depth-limited search labelled heuristic.
    Auto,
    /// Exact proof only; UNKNOWN if the budget runs out.
    Exact,
    /// Depth-limited heuristic search only. Never claims a proof.
    DepthLimited,
}

/// Everything a solve run is allowed to spend (0 = no limit per field).
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

/// The result of analysing a position. The certainty field is the most
/// important part: it distinguishes a mathematical proof from a
/// depth-limited heuristic opinion.
#[derive(Clone, Debug)]
pub struct SearchResult {
    pub certainty: Certainty,
    /// Proven outcome if proven; heuristic prediction otherwise; None if unknown.
    pub outcome: Option<Outcome>,
    pub best_move: Option<Move>,
    pub move_is_proven_optimal: bool,
    /// Exact score if proven by search, heuristic evaluation otherwise.
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

/// Why a search stopped early.
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

/// Budgets + cancellation + progress for one search, mirroring the Java
/// (nodeLimit, timeLimitMillis, CancellationToken, SearchListener)
/// parameter bundle. Cheap to clone.
#[derive(Clone)]
pub struct SearchControl {
    pub node_limit: u64, // 0 = unlimited
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
    /// node_limit 0 = unlimited; time 0 = unlimited.
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
    /// The per-node budget check used by the exact solvers (node limit
    /// every node; cancel/deadline every 1024; progress every 65536).
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
    /// The per-node budget check used by the depth-limited solver
    /// (same, but no periodic progress reports).
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

/// Columns ordered centre-out (ties keep left-to-right order).
pub fn center_out_order(width: i32) -> Vec<i32> {
    let mut cols: Vec<i32> = (0..width).collect();
    cols.sort_by_key(|&c| (2 * c - (width - 1)).abs());
    cols
}

impl core::fmt::Display for SearchResult {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let mut s = match self.certainty {
            Certainty::ProvenExact => "PROVEN (exact search)".to_string(),
            Certainty::ProvenByTheory => "PROVEN (published theory)".to_string(),
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
                if self.move_is_proven_optimal { " (proven optimal)" } else { "" }
            ));
        }
        s.push_str(&format!(" | score: {}", self.score));
        if let Some(p) = self.plies_to_end {
            s.push_str(&format!(" | ends in {p} plies"));
        }
        s.push_str(&format!(" | nodes: {} | {} ms", self.nodes, self.elapsed_millis));
        if !self.principal_line.is_empty() {
            s.push_str("\n  PV: ");
            let parts: Vec<String> =
                self.principal_line.iter().map(|m| m.notation()).collect();
            s.push_str(&parts.join(" "));
        }
        s.push_str("\n  ");
        s.push_str(&self.explanation);
        write!(f, "{s}")
    }
}
