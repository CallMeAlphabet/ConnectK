//! Tauri backend for ConnectK: a thin adapter over `connectk-core`'s
//! GameSession. All engine logic lives in the core crate; deleting
//! this directory removes the GUI without touching the engine or CLI.

use connectk_core::app::GameSession;
use connectk_core::book::OpeningBook;
use connectk_core::model::{dirs, Move, Placement, Player, Rules};
use connectk_core::solver::facade::ConnectKSolver;
use connectk_core::solver::{SearchResult, SolveMode, SolveRequest};
use connectk_core::theory;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, State};

// ---------------- state ----------------

struct Inner {
    session: GameSession,
    book_cache: HashMap<String, Option<Arc<OpeningBook>>>,
    active_book_name: Option<String>,
    suggested: Option<Move>,
    cancel: Option<Arc<AtomicBool>>,
    searching: bool,
}

pub struct AppState {
    inner: Mutex<Inner>,
}

impl AppState {
    fn new() -> AppState {
        let rules = Rules::standard_connect4();
        let mut inner = Inner {
            session: GameSession::new(rules.clone()),
            book_cache: HashMap::new(),
            active_book_name: None,
            suggested: None,
            cancel: None,
            searching: false,
        };
        let (solver, name) = solver_for(&mut inner.book_cache, &rules);
        inner.session.set_solver(solver);
        inner.active_book_name = name;
        AppState {
            inner: Mutex::new(inner),
        }
    }
}

/// Facade for a game, with a generated opening book attached when
/// ./books contains one whose file name encodes exactly these rules
/// (`std7x4-d10.book`, `nodiag6x4-d12.book`, optional `-kK`). Books are
/// never used across rulesets: their keys are bitboards of one board.
fn solver_for(
    cache: &mut HashMap<String, Option<Arc<OpeningBook>>>,
    rules: &Rules,
) -> (ConnectKSolver, Option<String>) {
    let dir = PathBuf::from("books");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return (ConnectKSolver::new(), None);
    };
    let mut best: Option<PathBuf> = None;
    let mut best_depth = -1i32;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some((book_rules, depth)) = parse_book_name(&name) else {
            continue;
        };
        if &book_rules == rules && depth > best_depth {
            best_depth = depth;
            best = Some(entry.path());
        }
    }
    let Some(path) = best else {
        return (ConnectKSolver::new(), None);
    };
    let key = path.to_string_lossy().into_owned();
    let book = cache
        .entry(key)
        .or_insert_with(|| OpeningBook::load(&path).ok().map(Arc::new))
        .clone();
    match book {
        Some(book) => {
            let label = format!(
                "{} ({} entries)",
                path.file_name().unwrap().to_string_lossy(),
                book.size()
            );
            (ConnectKSolver::with_book(Some(book)), Some(label))
        }
        None => (ConnectKSolver::new(), None),
    }
}

/// Parse `^(std|nodiag)(\d+)x(\d+)(?:-k(\d+))?-d(\d+)\.book$` by hand.
fn parse_book_name(name: &str) -> Option<(Rules, i32)> {
    let stem = name.strip_suffix(".book")?;
    let (diag, rest) = if let Some(r) = stem.strip_prefix("std") {
        (true, r)
    } else if let Some(r) = stem.strip_prefix("nodiag") {
        (false, r)
    } else {
        return None;
    };
    let (w_str, rest) = take_digits(rest)?;
    let rest = rest.strip_prefix('x')?;
    let (h_str, rest) = take_digits(rest)?;
    let (k, rest) = if let Some(r) = rest.strip_prefix("-k") {
        let (k_str, r) = take_digits(r)?;
        (k_str.parse().ok()?, r)
    } else {
        (4, rest)
    };
    let rest = rest.strip_prefix("-d")?;
    let (d_str, rest) = take_digits(rest)?;
    if !rest.is_empty() {
        return None;
    }
    let rules = Rules::new(
        w_str.parse().ok()?,
        h_str.parse().ok()?,
        k,
        Placement::Gravity,
        if diag { dirs::ALL } else { dirs::ORTHOGONAL },
        false,
    )
    .ok()?;
    Some((rules, d_str.parse().ok()?))
}

fn take_digits(s: &str) -> Option<(&str, &str)> {
    let end = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    if end == 0 {
        return None;
    }
    Some((&s[..end], &s[end..]))
}

// ---------------- DTOs ----------------

#[derive(Deserialize)]
pub struct RulesDto {
    width: i32,
    height: i32,
    connect: i32,
    placement: String, // "gravity" | "free"
    diagonals: bool,
    misere: bool,
}

impl RulesDto {
    fn to_rules(&self) -> Result<Rules, String> {
        let placement = match self.placement.as_str() {
            "free" => Placement::Free,
            _ => Placement::Gravity,
        };
        Rules::new(
            self.width,
            self.height,
            self.connect,
            placement,
            if self.diagonals { dirs::ALL } else { dirs::ORTHOGONAL },
            self.misere,
        )
    }
}

#[derive(Serialize, Clone)]
pub struct MoveDto {
    column: i32,
    row: i32,
    notation: String,
}

fn move_dto(m: Move) -> MoveDto {
    MoveDto {
        column: m.column,
        row: m.row,
        notation: m.notation(),
    }
}

#[derive(Serialize, Clone)]
pub struct TheoryDto {
    outcome: String,
    plies_to_end: Option<i32>,
    best_move: Option<MoveDto>,
    source: String,
}

#[derive(Serialize, Clone)]
pub struct GameStateDto {
    width: i32,
    height: i32,
    connect: i32,
    gravity: bool,
    /// Flat cells, index = column*height + row (row 0 = bottom):
    /// 0 empty, 1 = player one, 2 = player two.
    cells: Vec<i32>,
    side_to_move: i32, // 1 or 2
    moves: Vec<MoveDto>,
    winner: Option<i32>,
    draw: bool,
    terminal: bool,
    last_move: Option<MoveDto>,
    line_cells: Vec<MoveDto>,
    suggested: Option<MoveDto>,
    theory: Option<TheoryDto>,
    book_name: Option<String>,
    rules_text: String,
    status_text: String,
}

#[derive(Deserialize)]
pub struct AnalyseDto {
    mode: String, // "auto" | "exact" | "depth"
    depth: i32,
    time_seconds: f64,
    use_theory: bool,
}

#[derive(Serialize, Clone)]
pub struct SearchResultDto {
    certainty: String,
    outcome: Option<String>,
    best_move: Option<MoveDto>,
    move_proven: bool,
    score: i32,
    plies_to_end: Option<i32>,
    depth_reached: i32,
    nodes: u64,
    elapsed_millis: u64,
    pv: Vec<MoveDto>,
    result_text: String,
}

#[derive(Serialize, Clone)]
struct ProgressDto {
    nodes: u64,
    depth: i32,
    best: Option<MoveDto>,
}

fn result_dto(r: &SearchResult) -> SearchResultDto {
    SearchResultDto {
        certainty: r.certainty.name().to_string(),
        outcome: r.outcome.map(|o| o.to_string()),
        best_move: r.best_move.map(move_dto),
        move_proven: r.move_is_proven_optimal,
        score: r.score,
        plies_to_end: r.plies_to_end,
        depth_reached: r.depth_reached,
        nodes: r.nodes,
        elapsed_millis: r.elapsed_millis,
        pv: r.principal_line.iter().copied().map(move_dto).collect(),
        result_text: format!("{r}"),
    }
}

fn state_dto(inner: &Inner) -> GameStateDto {
    let pos = inner.session.position();
    let rules = pos.rules();
    let mut cells = vec![0i32; (rules.width * rules.height) as usize];
    for c in 0..rules.width {
        for r in 0..rules.height {
            cells[(c * rules.height + r) as usize] = match pos.owner_at(c, r) {
                Some(Player::One) => 1,
                Some(Player::Two) => 2,
                None => 0,
            };
        }
    }
    let status_text = if let Some(w) = pos.winner() {
        let how = if rules.misere {
            format!(" wins (opponent was forced to connect {})", rules.connect)
        } else {
            " wins".to_string()
        };
        format!("{} ({}){} on ply {}", w, w.symbol(), how, pos.moves_played())
    } else if pos.is_draw() {
        format!("Draw — board full on ply {}", pos.moves_played())
    } else {
        let side = pos.side_to_move();
        format!(
            "{} ({}) to move · ply {}",
            side,
            side.symbol(),
            pos.moves_played() + 1
        )
    };
    GameStateDto {
        width: rules.width,
        height: rules.height,
        connect: rules.connect,
        gravity: rules.placement == Placement::Gravity,
        cells,
        side_to_move: match pos.side_to_move() {
            Player::One => 1,
            Player::Two => 2,
        },
        moves: pos.move_history().iter().copied().map(move_dto).collect(),
        winner: pos.winner().map(|w| match w {
            Player::One => 1,
            Player::Two => 2,
        }),
        draw: pos.is_draw(),
        terminal: pos.is_terminal(),
        last_move: pos.last_move().map(move_dto),
        line_cells: pos.completed_line_cells().into_iter().map(move_dto).collect(),
        suggested: inner.suggested.map(move_dto),
        theory: theory::lookup(pos).map(|t| TheoryDto {
            outcome: t.outcome.to_string(),
            plies_to_end: t.plies_to_end,
            best_move: t.proven_best_move.map(move_dto),
            source: t.source.to_string(),
        }),
        book_name: inner.active_book_name.clone(),
        rules_text: rules.to_string(),
        status_text,
    }
}

// ---------------- commands ----------------

#[tauri::command]
fn get_state(state: State<'_, AppState>) -> GameStateDto {
    state_dto(&state.inner.lock().unwrap())
}

#[tauri::command]
fn new_game(state: State<'_, AppState>, rules: RulesDto) -> Result<GameStateDto, String> {
    let rules = rules.to_rules()?;
    let mut inner = state.inner.lock().unwrap();
    if let Some(flag) = &inner.cancel {
        flag.store(true, Ordering::Relaxed);
    }
    let (solver, name) = solver_for(&mut inner.book_cache, &rules);
    inner.active_book_name = name;
    inner.suggested = None;
    inner.session = GameSession::with_solver(rules, solver);
    Ok(state_dto(&inner))
}

#[tauri::command]
fn play(
    state: State<'_, AppState>,
    column: i32,
    row: Option<i32>,
) -> Result<GameStateDto, String> {
    let mut inner = state.inner.lock().unwrap();
    if inner.searching {
        return Err("search in progress".into());
    }
    let mv = match row {
        Some(r) => Move::cell(column, r),
        None => Move::column(column),
    };
    inner.session.play(mv)?;
    inner.suggested = None;
    Ok(state_dto(&inner))
}

#[tauri::command]
fn undo(state: State<'_, AppState>) -> Result<GameStateDto, String> {
    let mut inner = state.inner.lock().unwrap();
    if inner.searching {
        return Err("search in progress".into());
    }
    inner.session.undo()?;
    inner.suggested = None;
    Ok(state_dto(&inner))
}

#[tauri::command]
fn cancel(state: State<'_, AppState>) {
    let inner = state.inner.lock().unwrap();
    if let Some(flag) = &inner.cancel {
        flag.store(true, Ordering::Relaxed);
    }
}

/// Shared implementation for analyse / ai_move: snapshots the position
/// and solver under the lock, searches off-lock on a blocking thread
/// (progress via events), then applies the suggestion (and the move
/// itself for ai_move) only if the position has not changed meanwhile.
async fn run_search(
    app: AppHandle,
    state: State<'_, AppState>,
    req: AnalyseDto,
    play_move: bool,
) -> Result<SearchResultDto, String> {
    let (snapshot, book, base_plies) = {
        let mut inner = state.inner.lock().unwrap();
        if inner.searching {
            return Err("search already running".into());
        }
        inner.searching = true;
        let flag = Arc::new(AtomicBool::new(false));
        inner.cancel = Some(flag);
        (
            inner.session.position().clone(),
            inner.session.solver().book().cloned(),
            inner.session.position().moves_played() as i32,
        )
    };
    let cancel_flag = state
        .inner
        .lock()
        .unwrap()
        .cancel
        .clone()
        .expect("flag set above");

    let mode = match req.mode.as_str() {
        "exact" => SolveMode::Exact,
        "depth" => SolveMode::DepthLimited,
        _ => SolveMode::Auto,
    };
    let millis = (req.time_seconds * 1000.0).round().max(50.0) as u64;
    let request = SolveRequest {
        mode,
        max_depth: req.depth.max(1),
        time_limit_millis: millis,
        node_limit: 0,
        exact_time_limit_millis: if mode == SolveMode::Auto {
            millis.min(2000)
        } else {
            millis
        },
        exact_node_limit: 0,
        use_theory: req.use_theory,
    };

    let app_for_progress = app.clone();
    let last_emit = Arc::new(Mutex::new(Instant::now()));
    let listener: connectk_core::solver::ProgressFn =
        Arc::new(move |nodes, depth, best| {
            let mut last = last_emit.lock().unwrap();
            if last.elapsed() < Duration::from_millis(50) && depth < 0 {
                return;
            }
            *last = Instant::now();
            let _ = app_for_progress.emit(
                "search-progress",
                ProgressDto {
                    nodes,
                    depth,
                    best: best.map(move_dto),
                },
            );
        });

    let flag_for_task = cancel_flag.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let solver = ConnectKSolver::with_book_and_threads(book, 1);
        solver.solve_with(&snapshot, &request, Some(flag_for_task), Some(listener))
    })
    .await
    .map_err(|e| format!("search task failed: {e}"))?;

    let cancelled = cancel_flag.load(Ordering::Relaxed);
    let dto = result_dto(&result);
    {
        let mut inner = state.inner.lock().unwrap();
        inner.searching = false;
        if !cancelled && inner.session.position().moves_played() as i32 == base_plies {
            inner.suggested = result.best_move;
            if play_move {
                if let Some(mv) = result.best_move {
                    if inner.session.play(mv).is_ok() {
                        inner.suggested = None;
                    }
                }
            }
        }
    }
    if cancelled {
        return Err("cancelled".into());
    }
    Ok(dto)
}

#[tauri::command]
async fn analyse(
    app: AppHandle,
    state: State<'_, AppState>,
    req: AnalyseDto,
) -> Result<SearchResultDto, String> {
    run_search(app, state, req, false).await
}

#[tauri::command]
async fn ai_move(
    app: AppHandle,
    state: State<'_, AppState>,
    req: AnalyseDto,
) -> Result<SearchResultDto, String> {
    run_search(app, state, req, true).await
}

pub fn run() {
    tauri::Builder::default()
        .manage(AppState::new())
        .invoke_handler(tauri::generate_handler![
            get_state, new_game, play, undo, cancel, analyse, ai_move
        ])
        .run(tauri::generate_context!())
        .expect("error while running ConnectK");
}
