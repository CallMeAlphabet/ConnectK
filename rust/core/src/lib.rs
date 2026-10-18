//! ConnectK core engine: generalized Connect-k (any board, any k,
//! gravity or free placement, optional diagonals, misère) with exact
//! game solving where mathematically available — published theory and
//! exhaustive proof search — and clearly labelled depth-limited
//! heuristic search everywhere else. Zero external dependencies.

pub mod app;
pub mod bits;
pub mod book;
pub mod eval;
pub mod hash;
pub mod model;
pub mod position;
pub mod solver;
pub mod theory;

pub use app::GameSession;
pub use book::OpeningBook;
pub use eval::Evaluator;
pub use model::{dirs, Move, Outcome, Placement, Player, Rules};
pub use position::{Position, PositionKey};
pub use solver::facade::ConnectKSolver;
pub use solver::{Certainty, SearchControl, SearchResult, SolveMode, SolveRequest};
pub use theory::TheoryResult;
