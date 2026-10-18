//! Headless game-session layer shared by front ends (Tauri GUI, tests):
//! owns the current position and a solver, exposes play/undo/analyse
//! operations without any UI concerns.

use crate::model::{Move, Rules};
use crate::position::Position;
use crate::solver::facade::ConnectKSolver;
use crate::solver::{ProgressFn, SearchResult, SolveRequest};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

pub struct GameSession {
    position: Position,
    solver: ConnectKSolver,
}

impl GameSession {
    pub fn new(rules: Rules) -> GameSession {
        GameSession {
            position: Position::initial(&rules),
            solver: ConnectKSolver::new(),
        }
    }

    pub fn with_solver(rules: Rules, solver: ConnectKSolver) -> GameSession {
        GameSession {
            position: Position::initial(&rules),
            solver,
        }
    }

    pub fn solver(&self) -> &ConnectKSolver {
        &self.solver
    }
    pub fn set_solver(&mut self, solver: ConnectKSolver) {
        self.solver = solver;
    }
    pub fn position(&self) -> &Position {
        &self.position
    }
    pub fn rules(&self) -> &Rules {
        self.position.rules()
    }

    pub fn reset(&mut self, rules: Rules) {
        self.position = Position::initial(&rules);
    }

    pub fn play(&mut self, mv: Move) -> Result<Move, String> {
        self.position.play(mv)
    }
    pub fn play_column(&mut self, column: i32) -> Result<Move, String> {
        self.position.play_column(column)
    }
    pub fn undo(&mut self) -> Result<Move, String> {
        self.position.undo()
    }

    pub fn analyse(&self, request: &SolveRequest) -> SearchResult {
        self.solver.solve(&self.position, request)
    }
    pub fn analyse_with(
        &self,
        request: &SolveRequest,
        cancel: Option<Arc<AtomicBool>>,
        listener: Option<ProgressFn>,
    ) -> SearchResult {
        self.solver
            .solve_with(&self.position, request, cancel, listener)
    }

    /// Analyse and play the recommended move, if any.
    pub fn play_best(&mut self, request: &SolveRequest) -> Result<(Move, SearchResult), String> {
        let result = self.analyse(request);
        match result.best_move {
            Some(mv) => {
                let cell = self.position.play(mv)?;
                Ok((cell, result))
            }
            None => Err("no move available (game over or no conclusion)".into()),
        }
    }

    pub fn theory(&self) -> Option<crate::theory::TheoryResult> {
        crate::theory::lookup(&self.position)
    }
}
