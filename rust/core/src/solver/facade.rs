use super::depth::DepthLimitedSolver;
use super::fast::FastExactSolver;
use super::generic::ExactSolver;
use super::parallel::ParallelExactSolver;
use super::standard::StandardSolver;
use super::{Certainty, ProgressFn, SearchControl, SearchResult, SolveMode, SolveRequest};
use crate::book::OpeningBook;
use crate::model::{Outcome, Placement, Rules};
use crate::position::Position;
use crate::theory::{self, TheoryResult};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Instant;

enum BookChoice {
    Auto,
    Explicit(Option<Arc<OpeningBook>>),
}

struct AutoBookEntry {
    rules: Rules,
    signature: String,
    book: Option<Arc<OpeningBook>>,
}

pub struct ConnectKSolver {
    book_choice: BookChoice,
    auto_books: Mutex<Vec<AutoBookEntry>>,
    threads: usize,
}

impl Default for ConnectKSolver {
    fn default() -> Self {
        Self::new()
    }
}

impl ConnectKSolver {
    pub fn new() -> ConnectKSolver {
        ConnectKSolver::with_threads(1)
    }

    pub fn with_threads(threads: usize) -> ConnectKSolver {
        ConnectKSolver {
            book_choice: BookChoice::Auto,
            auto_books: Mutex::new(Vec::new()),
            threads: threads.max(1),
        }
    }

    pub fn without_book() -> ConnectKSolver {
        ConnectKSolver::with_book(None)
    }

    pub fn without_book_and_threads(threads: usize) -> ConnectKSolver {
        ConnectKSolver::with_book_and_threads(None, threads)
    }

    pub fn with_book(book: Option<Arc<OpeningBook>>) -> ConnectKSolver {
        ConnectKSolver::with_book_and_threads(book, 1)
    }

    pub fn with_book_and_threads(book: Option<Arc<OpeningBook>>, threads: usize) -> ConnectKSolver {
        ConnectKSolver {
            book_choice: BookChoice::Explicit(book),
            auto_books: Mutex::new(Vec::new()),
            threads: threads.max(1),
        }
    }
    pub fn threads(&self) -> usize {
        self.threads
    }
    pub fn book(&self) -> Option<&Arc<OpeningBook>> {
        match &self.book_choice {
            BookChoice::Explicit(book) => book.as_ref(),
            BookChoice::Auto => None,
        }
    }

    pub fn book_for_rules(&self, rules: &Rules) -> Option<Arc<OpeningBook>> {
        match &self.book_choice {
            BookChoice::Explicit(book) => book
                .clone()
                .filter(|book| book.rules().map(|r| r == rules).unwrap_or(true)),
            BookChoice::Auto => {
                let signature = OpeningBook::candidate_signature_for_rules(rules);
                {
                    let cache = self.auto_books.lock().unwrap();
                    if let Some(entry) = cache
                        .iter()
                        .find(|entry| entry.rules == *rules && entry.signature == signature)
                    {
                        return entry.book.clone();
                    }
                }
                let book =
                    OpeningBook::discover_all_for_rules(rules).map(|(_, book)| Arc::new(book));
                let mut cache = self.auto_books.lock().unwrap();
                cache.retain(|entry| entry.rules != *rules);
                cache.push(AutoBookEntry {
                    rules: rules.clone(),
                    signature,
                    book: book.clone(),
                });
                book
            }
        }
    }

    pub fn solve(&self, position: &Position, request: &SolveRequest) -> SearchResult {
        self.solve_with(position, request, None, None)
    }

    pub fn solve_with(
        &self,
        position: &Position,
        request: &SolveRequest,
        cancel: Option<Arc<AtomicBool>>,
        listener: Option<ProgressFn>,
    ) -> SearchResult {
        let start = Instant::now();
        let mk = |node_limit: u64, time_millis: u64| -> SearchControl {
            let mut c = SearchControl::new(node_limit, time_millis);
            c.cancelled = cancel.clone();
            c.listener = listener.clone();
            c
        };

        if position.is_terminal() {
            return ExactSolver::new().solve(position, mk(0, 0));
        }

        let theory_hit = if request.use_theory && request.mode != SolveMode::DepthLimited {
            theory::lookup(position)
        } else {
            None
        };

        if request.mode != SolveMode::DepthLimited {
            if let Some(t) = &theory_hit {
                if let Some(best) = t.proven_best_move {
                    let score = theory_score(position, t);
                    return SearchResult::new(
                        Certainty::ProvenByTheory,
                        Some(t.outcome),
                        Some(best),
                        true,
                        score,
                        t.plies_to_end,
                        t.plies_to_end.unwrap_or(0),
                        0,
                        elapse(start),
                        vec![best],
                        format!("Outcome and move from theory: {}", t.source),
                    );
                }
            }
        }

        let matching_book = if request.mode == SolveMode::DepthLimited {
            None
        } else {
            self.book_for_rules(position.rules())
        };
        let attempt_exact = match request.mode {
            SolveMode::Exact => true,
            SolveMode::DepthLimited => false,
            SolveMode::Auto => {
                looks_tractable(position)
                    || matching_book
                        .as_deref()
                        .map(|book| {
                            book.covers_ply(position.moves_played() as i32)
                                || StandardSolver::supports(position.rules())
                        })
                        .unwrap_or(false)
            }
        };
        let mut exact_attempt: Option<SearchResult> = None;
        if attempt_exact {
            let exact_time = if request.mode == SolveMode::Exact {
                request.time_limit_millis
            } else {
                request.exact_time_limit_millis
            };
            let result = self.solve_exact(
                position,
                mk(request.exact_node_limit, exact_time),
                matching_book.clone(),
            );
            if result.certainty == Certainty::ProvenExact {
                return result;
            }
            if request.mode == SolveMode::Exact {
                if let Some(t) = &theory_hit {
                    return SearchResult::new(
                        Certainty::ProvenByTheory,
                        Some(t.outcome),
                        None,
                        false,
                        theory_score(position, t),
                        t.plies_to_end,
                        0,
                        result.nodes,
                        elapse(start),
                        Vec::new(),
                        format!(
                            "Exact search hit its budget, but the outcome is published \
                             theory: {} No proven move available.",
                            t.source
                        ),
                    );
                }
                return result;
            }
            exact_attempt = Some(result);
        }

        let remaining = remaining_millis(request, start);
        let heuristic = DepthLimitedSolver::new().solve(
            position,
            request.max_depth,
            mk(request.node_limit, remaining),
        );
        let total_nodes = heuristic.nodes + exact_attempt.as_ref().map(|e| e.nodes).unwrap_or(0);
        if let Some(t) = &theory_hit {
            let note = format!(
                "Outcome is published mathematical theory ({}). The recommended move \
                 comes from depth-limited search and is not certified optimal. {}",
                t.source, heuristic.explanation
            );
            return SearchResult::new(
                Certainty::ProvenByTheory,
                Some(t.outcome),
                heuristic.best_move,
                false,
                if t.plies_to_end.is_some() {
                    theory_score(position, t)
                } else {
                    heuristic.score
                },
                t.plies_to_end,
                heuristic.depth_reached,
                total_nodes,
                elapse(start),
                heuristic.principal_line.clone(),
                note,
            );
        }
        if let Some(exact) = &exact_attempt {
            return SearchResult::new(
                heuristic.certainty,
                heuristic.outcome,
                heuristic.best_move,
                false,
                heuristic.score,
                heuristic.plies_to_end,
                heuristic.depth_reached,
                total_nodes,
                elapse(start),
                heuristic.principal_line.clone(),
                format!(
                    "Exact proof did not finish within its budget ({} nodes), so this \
                     position is not mathematically solved here. {}",
                    exact.nodes, heuristic.explanation
                ),
            );
        }
        heuristic
    }

    fn solve_exact(
        &self,
        position: &Position,
        control: SearchControl,
        book: Option<Arc<OpeningBook>>,
    ) -> SearchResult {
        if self.threads > 1 {
            return ParallelExactSolver::new(self.threads, book).solve(position, control);
        }
        if StandardSolver::supports(position.rules()) {
            let mut standard = StandardSolver::new(0);
            standard.set_opening_book(book);
            return standard.solve(position, control);
        }
        if FastExactSolver::supports(position.rules()) {
            let mut fast = FastExactSolver::new();
            fast.set_opening_book(book);
            return fast.solve(position, control);
        }
        let mut generic = ExactSolver::new();
        generic.set_opening_book(book);
        generic.solve(position, control)
    }
}

fn elapse(start: Instant) -> u64 {
    start.elapsed().as_millis() as u64
}

fn remaining_millis(request: &SolveRequest, start: Instant) -> u64 {
    if request.time_limit_millis == 0 {
        return 0;
    }
    let left = request.time_limit_millis as i64 - elapse(start) as i64;
    left.max(50) as u64
}

fn looks_tractable(p: &Position) -> bool {
    let cells = p.rules().cells();
    let remaining = cells - p.moves_played() as i32;
    if remaining <= 22 {
        return true;
    }
    if cells <= 30 {
        return true;
    }
    p.rules().placement == Placement::Gravity && p.rules().width <= 7 && remaining <= 28
}

fn theory_score(p: &Position, t: &TheoryResult) -> i32 {
    if t.outcome == Outcome::Draw {
        return 0;
    }
    let Some(plies) = t.plies_to_end else {
        return 0;
    };
    let terminal_ply = p.moves_played() as i32 + plies;
    let magnitude = p.rules().cells() - terminal_ply + 1;
    if t.outcome == Outcome::Win {
        magnitude
    } else {
        -magnitude
    }
}
