use crate::json::Json;
use crate::native;
use connectk_core::app::GameSession;
use connectk_core::book::{rules_line, OpeningBook};
use connectk_core::model::{dirs, Move, Placement, Player, Rules};
use connectk_core::solver::facade::ConnectKSolver;
use connectk_core::solver::{ProgressFn, SearchResult, SolveRequest};
use connectk_core::theory;
use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

const INDEX_HTML: &str = include_str!("../ui/index.html");
const WEB_HTML: &str = include_str!("../ui/web.html");
const ICON_PNG: &[u8] = include_bytes!("../ui/icon.png");
const MAX_HEADER_BYTES: usize = 64 * 1024;
const MAX_BODY_BYTES: usize = 1024 * 1024;

struct Inner {
    session: GameSession,
    book_cache: HashMap<String, Option<Arc<OpeningBook>>>,
    active_book_name: Option<String>,
    suggested: Option<Move>,
    cancel: Option<Arc<AtomicBool>>,
    searching: bool,
}

#[derive(Clone, Copy)]
struct Progress {
    nodes: u64,
    depth: i32,
    best: Option<Move>,
}

pub struct AppState {
    inner: Mutex<Inner>,
    progress: Arc<Mutex<Option<Progress>>>,
    token: String,
}

impl AppState {
    pub fn new(token: String) -> AppState {
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
            progress: Arc::new(Mutex::new(None)),
            token,
        }
    }

    fn handle_command(&self, command: &str, args: &Json) -> Result<Json, String> {
        match command {
            "get_state" => Ok(state_json(&lock(&self.inner))),
            "symbolic_theory" => self.symbolic_theory(args),
            "new_game" => self.new_game(args),
            "play" => self.play(args),
            "undo" => self.undo(),
            "cancel" => {
                self.cancel();
                Ok(Json::object(vec![]))
            }
            "analyse" => self.run_search(args, false),
            "ai_move" => self.run_search(args, true),
            "set_fullscreen" => {
                let enabled = args
                    .get("enabled")
                    .and_then(Json::as_bool)
                    .ok_or("set_fullscreen.enabled must be a boolean")?;
                Ok(Json::object(vec![(
                    "native",
                    Json::Bool(native::set_fullscreen(enabled)),
                )]))
            }
            _ => Err(format!("unknown command: {command}")),
        }
    }

    fn new_game(&self, args: &Json) -> Result<Json, String> {
        let rules_json = args.get("rules").ok_or("new_game.rules is required")?;
        let rules = rules_from_json(rules_json)?;
        let mut inner = lock(&self.inner);
        if let Some(flag) = &inner.cancel {
            flag.store(true, Ordering::Relaxed);
        }
        let (solver, name) = solver_for(&mut inner.book_cache, &rules);
        inner.active_book_name = name;
        inner.suggested = None;
        inner.session = GameSession::with_solver(rules, solver);
        *lock(&self.progress) = None;
        Ok(state_json(&inner))
    }

    fn symbolic_theory(&self, args: &Json) -> Result<Json, String> {
        let rules_json = args
            .get("rules")
            .ok_or("symbolic_theory.rules is required")?;
        let raw = raw_rules_from_json(rules_json)?;
        let Some(result) = theory::lookup_empty_symbolic(
            raw.width,
            raw.height,
            raw.connect,
            raw.placement,
            raw.directions,
            raw.misere,
        ) else {
            return Ok(Json::Null);
        };
        Ok(Json::object(vec![
            ("outcome", Json::String(result.outcome.to_string())),
            (
                "plies_to_end",
                result.plies_to_end.map(Json::int).unwrap_or(Json::Null),
            ),
            (
                "plies_to_end_text",
                result
                    .plies_to_end
                    .map(|plies| Json::String(plies.to_string()))
                    .unwrap_or(Json::Null),
            ),
            ("best_move", optional_move_json(result.proven_best_move)),
            ("source", Json::from(result.source)),
        ]))
    }

    fn play(&self, args: &Json) -> Result<Json, String> {
        let column = args
            .get("column")
            .and_then(Json::as_i32)
            .ok_or("play.column must be an integer")?;
        let row = match args.get("row") {
            Some(value) if !value.is_null() => Some(
                value
                    .as_i32()
                    .ok_or("play.row must be an integer or null")?,
            ),
            _ => None,
        };
        let mut inner = lock(&self.inner);
        if inner.searching {
            return Err("search in progress".into());
        }
        let mv = match row {
            Some(row) => Move::cell(column, row),
            None => Move::column(column),
        };
        inner.session.play(mv)?;
        inner.suggested = None;
        Ok(state_json(&inner))
    }

    fn undo(&self) -> Result<Json, String> {
        let mut inner = lock(&self.inner);
        if inner.searching {
            return Err("search in progress".into());
        }
        inner.session.undo()?;
        inner.suggested = None;
        Ok(state_json(&inner))
    }

    fn cancel(&self) {
        let inner = lock(&self.inner);
        if let Some(flag) = &inner.cancel {
            flag.store(true, Ordering::Relaxed);
        }
    }

    fn progress_json(&self) -> Json {
        lock(&self.progress)
            .map(|progress| {
                Json::object(vec![
                    ("nodes", Json::uint(progress.nodes)),
                    ("depth", Json::int(progress.depth as i64)),
                    ("best", optional_move_json(progress.best)),
                ])
            })
            .unwrap_or(Json::Null)
    }

    fn run_search(&self, args: &Json, play_move: bool) -> Result<Json, String> {
        let request_json = args.get("req").unwrap_or(args);
        let use_theory = request_json
            .get("use_theory")
            .and_then(Json::as_bool)
            .unwrap_or(true);

        let (snapshot, book, cancel_flag) = {
            let mut inner = lock(&self.inner);
            if inner.searching {
                return Err("search already running".into());
            }
            inner.searching = true;
            let flag = Arc::new(AtomicBool::new(false));
            inner.cancel = Some(flag.clone());
            (
                inner.session.position().clone(),
                inner
                    .session
                    .solver()
                    .book_for_rules(inner.session.position().rules()),
                flag,
            )
        };
        *lock(&self.progress) = None;

        let progress = self.progress.clone();
        let last_update = Arc::new(Mutex::new(Instant::now()));
        let listener: ProgressFn = Arc::new(move |nodes, depth, best| {
            let mut last = lock(&last_update);
            if last.elapsed() < Duration::from_millis(50) && depth < 0 {
                return;
            }
            *last = Instant::now();
            drop(last);
            *lock(&progress) = Some(Progress { nodes, depth, best });
        });

        let request = SolveRequest::automatic_with_book(&snapshot, use_theory, book.as_deref());
        let solver = ConnectKSolver::with_book_and_threads(book, 1);
        let result = solver.solve_with(
            &snapshot,
            &request,
            Some(cancel_flag.clone()),
            Some(listener),
        );
        let cancelled = cancel_flag.load(Ordering::Relaxed);
        let result_json = search_result_json(&result);

        {
            let mut inner = lock(&self.inner);
            inner.searching = false;
            inner.cancel = None;
            let current = inner.session.position();
            let unchanged =
                current.rules() == snapshot.rules() && current.raw_key() == snapshot.raw_key();
            if !cancelled && unchanged {
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
        *lock(&self.progress) = None;

        if cancelled {
            Err("cancelled".into())
        } else {
            Ok(result_json)
        }
    }
}

pub fn serve(listener: TcpListener, state: Arc<AppState>) {
    let port = listener.local_addr().map(|addr| addr.port()).unwrap_or(0);
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let state = state.clone();
                std::thread::spawn(move || {
                    let _ = handle_connection(stream, state, port);
                });
            }
            Err(error) => eprintln!("GUI server accept failed: {error}"),
        }
    }
}

fn handle_connection(mut stream: TcpStream, state: Arc<AppState>, port: u16) -> io::Result<()> {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(30)));
    let Some(request) = read_request(&mut stream)? else {
        return Ok(());
    };
    let response = route(request, &state, port);
    write_response(&mut stream, response)
}

struct Request {
    method: String,
    path: String,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

struct Response {
    status: u16,
    content_type: &'static str,
    body: Vec<u8>,
    extra_headers: Vec<(&'static str, String)>,
}

impl Response {
    fn json(status: u16, value: Json) -> Response {
        Response {
            status,
            content_type: "application/json; charset=utf-8",
            body: value.encode().into_bytes(),
            extra_headers: Vec::new(),
        }
    }

    fn error(status: u16, message: impl Into<String>) -> Response {
        Response::json(
            status,
            Json::object(vec![("error", Json::String(message.into()))]),
        )
    }
}

fn route(request: Request, state: &AppState, port: u16) -> Response {
    if !valid_host(request.headers.get("host"), port) {
        return Response::error(403, "invalid Host header");
    }
    let path = request.path.split('?').next().unwrap_or("/");
    match (request.method.as_str(), path) {
        ("GET", "/") | ("GET", "/index.html") | ("GET", "/web") | ("GET", "/web.html") => {
            let template = if path.starts_with("/web") {
                WEB_HTML
            } else {
                INDEX_HTML
            };
            let html = template.replace("__CONNECTK_TOKEN__", &state.token);
            Response {
                status: 200,
                content_type: "text/html; charset=utf-8",
                body: html.into_bytes(),
                extra_headers: vec![(
                    "Content-Security-Policy",
                    "default-src 'self'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src 'self' data:; connect-src 'self'; base-uri 'none'; frame-ancestors 'none'".into(),
                )],
            }
        }
        ("GET", "/favicon.ico") | ("GET", "/icon.png") => Response {
            status: 200,
            content_type: "image/png",
            body: ICON_PNG.to_vec(),
            extra_headers: Vec::new(),
        },
        ("GET", "/api/progress") => {
            if !authorized(&request, state) {
                return Response::error(403, "missing or invalid GUI token");
            }
            Response::json(200, state.progress_json())
        }
        ("POST", path) if path.starts_with("/api/") => {
            if !authorized(&request, state) {
                return Response::error(403, "missing or invalid GUI token");
            }
            let command = &path["/api/".len()..];
            let args = if request.body.is_empty() {
                Json::object(vec![])
            } else {
                match std::str::from_utf8(&request.body)
                    .map_err(|_| "request body is not UTF-8".to_string())
                    .and_then(Json::parse)
                {
                    Ok(args) => args,
                    Err(error) => return Response::error(400, error),
                }
            };
            match state.handle_command(command, &args) {
                Ok(value) => Response::json(200, value),
                Err(error) if error == "cancelled" => Response::error(409, error),
                Err(error) => Response::error(400, error),
            }
        }
        _ => Response::error(404, "not found"),
    }
}

fn authorized(request: &Request, state: &AppState) -> bool {
    request
        .headers
        .get("x-connectk-token")
        .is_some_and(|token| token == &state.token)
}

fn valid_host(host: Option<&String>, port: u16) -> bool {
    let Some(host) = host else {
        return false;
    };
    let host = host.trim().to_ascii_lowercase();
    let (name, host_port) = if let Some(rest) = host.strip_prefix("[::1]") {
        let parsed = if rest.is_empty() {
            None
        } else {
            rest.strip_prefix(':').and_then(|p| p.parse::<u16>().ok())
        };
        if !rest.is_empty() && parsed.is_none() {
            return false;
        }
        ("[::1]", parsed)
    } else if let Some((name, port_text)) = host.rsplit_once(':') {
        if port_text.chars().all(|ch| ch.is_ascii_digit()) && !port_text.is_empty() {
            (name, port_text.parse::<u16>().ok())
        } else {
            (host.as_str(), None)
        }
    } else {
        (host.as_str(), None)
    };
    let local_host = matches!(name, "127.0.0.1" | "localhost" | "[::1]");
    let preview_host =
        std::env::var("CONNECTK_GUI_PREVIEW").as_deref() == Ok("1") && name.ends_with(".e2b.app");
    (local_host || preview_host)
        && match host_port {
            Some(host_port) => host_port == port || port == 0,
            None => true,
        }
}

fn read_request(stream: &mut TcpStream) -> io::Result<Option<Request>> {
    let mut data = Vec::with_capacity(8192);
    let mut chunk = [0u8; 8192];
    let (header_end, body_start) = loop {
        if let Some(found) = find_header_end(&data) {
            break found;
        }
        if data.len() > MAX_HEADER_BYTES {
            return Ok(None);
        }
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            return Ok(None);
        }
        data.extend_from_slice(&chunk[..read]);
    };

    let header_text = String::from_utf8_lossy(&data[..header_end]);
    let mut lines = header_text.lines();
    let Some(request_line) = lines.next() else {
        return Ok(None);
    };
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("").to_ascii_uppercase();
    let path = parts.next().unwrap_or("/").to_string();
    let mut headers = HashMap::new();
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
        }
    }
    let content_length = headers
        .get("content-length")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0)
        .min(MAX_BODY_BYTES);
    let mut body = data.split_off(body_start);
    while body.len() < content_length {
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..read]);
    }
    body.truncate(content_length);
    Ok(Some(Request {
        method,
        path,
        headers,
        body,
    }))
}

fn find_header_end(data: &[u8]) -> Option<(usize, usize)> {
    data.windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|index| (index, index + 4))
        .or_else(|| {
            data.windows(2)
                .position(|window| window == b"\n\n")
                .map(|index| (index, index + 2))
        })
}

fn write_response(stream: &mut TcpStream, response: Response) -> io::Result<()> {
    let status_text = match response.status {
        200 => "OK",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        _ => "Internal Server Error",
    };
    let mut head = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n",
        response.status,
        status_text,
        response.content_type,
        response.body.len()
    );
    for (name, value) in response.extra_headers {
        head.push_str(name);
        head.push_str(": ");
        head.push_str(&value);
        head.push_str("\r\n");
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes())?;
    stream.write_all(&response.body)?;
    stream.flush()
}

struct RawRules {
    width: i32,
    height: i32,
    connect: i32,
    placement: Placement,
    directions: u8,
    misere: bool,
}

fn raw_rules_from_json(value: &Json) -> Result<RawRules, String> {
    let width = value
        .get("width")
        .and_then(Json::as_i32)
        .ok_or("rules.width must be a 32-bit integer")?;
    let height = value
        .get("height")
        .and_then(Json::as_i32)
        .ok_or("rules.height must be a 32-bit integer")?;
    let connect = value
        .get("connect")
        .and_then(Json::as_i32)
        .ok_or("rules.connect must be a 32-bit integer")?;
    let placement = match value.get("placement").and_then(Json::as_str) {
        Some("free") => Placement::Free,
        _ => Placement::Gravity,
    };
    let diagonals = value
        .get("diagonals")
        .and_then(Json::as_bool)
        .unwrap_or(true);
    let misere = value.get("misere").and_then(Json::as_bool).unwrap_or(false);
    Ok(RawRules {
        width,
        height,
        connect,
        placement,
        directions: if diagonals {
            dirs::ALL
        } else {
            dirs::ORTHOGONAL
        },
        misere,
    })
}

fn rules_from_json(value: &Json) -> Result<Rules, String> {
    let raw = raw_rules_from_json(value)?;
    Rules::new(
        raw.width,
        raw.height,
        raw.connect,
        raw.placement,
        raw.directions,
        raw.misere,
    )
}

fn move_json(mv: Move) -> Json {
    Json::object(vec![
        ("column", Json::from(mv.column)),
        ("row", Json::from(mv.row)),
        ("notation", Json::String(mv.notation())),
    ])
}

fn optional_move_json(mv: Option<Move>) -> Json {
    mv.map(move_json).unwrap_or(Json::Null)
}

fn optional_i32_json(value: Option<i32>) -> Json {
    value.map(Json::from).unwrap_or(Json::Null)
}

fn search_result_json(result: &SearchResult) -> Json {
    Json::object(vec![
        ("certainty", Json::from(result.certainty.name())),
        (
            "outcome",
            result
                .outcome
                .map(|outcome| Json::String(outcome.to_string()))
                .unwrap_or(Json::Null),
        ),
        ("best_move", optional_move_json(result.best_move)),
        ("move_proven", Json::Bool(result.move_is_proven_optimal)),
        ("score", Json::from(result.score)),
        ("plies_to_end", optional_i32_json(result.plies_to_end)),
        ("depth_reached", Json::from(result.depth_reached)),
        ("nodes", Json::uint(result.nodes)),
        ("elapsed_millis", Json::uint(result.elapsed_millis)),
        (
            "pv",
            Json::Array(
                result
                    .principal_line
                    .iter()
                    .copied()
                    .map(move_json)
                    .collect(),
            ),
        ),
        ("result_text", Json::String(format!("{result}"))),
    ])
}

fn state_json(inner: &Inner) -> Json {
    let position = inner.session.position();
    let rules = position.rules();
    let mut cells = Vec::with_capacity((rules.width * rules.height).max(0) as usize);
    for column in 0..rules.width {
        for row in 0..rules.height {
            cells.push(Json::from(match position.owner_at(column, row) {
                Some(Player::One) => 1,
                Some(Player::Two) => 2,
                None => 0,
            }));
        }
    }
    let status_text = if let Some(winner) = position.winner() {
        let how = if rules.misere {
            format!(" wins (opponent was forced to connect {})", rules.connect)
        } else {
            " wins".to_string()
        };
        format!(
            "{} ({}){} on ply {}",
            winner,
            winner.symbol(),
            how,
            position.moves_played()
        )
    } else if position.is_draw() {
        format!("Draw — board full on ply {}", position.moves_played())
    } else {
        let side = position.side_to_move();
        format!(
            "{} ({}) to move · ply {}",
            side,
            side.symbol(),
            position.moves_played() + 1
        )
    };
    let theory = theory::lookup(position)
        .map(|theory| {
            Json::object(vec![
                ("outcome", Json::String(theory.outcome.to_string())),
                ("plies_to_end", optional_i32_json(theory.plies_to_end)),
                ("best_move", optional_move_json(theory.proven_best_move)),
                ("source", Json::from(theory.source)),
            ])
        })
        .unwrap_or(Json::Null);

    Json::object(vec![
        ("width", Json::from(rules.width)),
        ("height", Json::from(rules.height)),
        ("connect", Json::from(rules.connect)),
        ("gravity", Json::Bool(rules.placement == Placement::Gravity)),
        ("cells", Json::Array(cells)),
        (
            "side_to_move",
            Json::from(match position.side_to_move() {
                Player::One => 1,
                Player::Two => 2,
            }),
        ),
        (
            "moves",
            Json::Array(
                position
                    .move_history()
                    .iter()
                    .copied()
                    .map(move_json)
                    .collect(),
            ),
        ),
        (
            "winner",
            position
                .winner()
                .map(|winner| {
                    Json::from(match winner {
                        Player::One => 1,
                        Player::Two => 2,
                    })
                })
                .unwrap_or(Json::Null),
        ),
        ("draw", Json::Bool(position.is_draw())),
        ("terminal", Json::Bool(position.is_terminal())),
        ("last_move", optional_move_json(position.last_move())),
        (
            "line_cells",
            Json::Array(
                position
                    .completed_line_cells()
                    .into_iter()
                    .map(move_json)
                    .collect(),
            ),
        ),
        ("suggested", optional_move_json(inner.suggested)),
        ("theory", theory),
        (
            "book_name",
            inner
                .active_book_name
                .clone()
                .map(Json::String)
                .unwrap_or(Json::Null),
        ),
        ("rules_text", Json::String(rules.to_string())),
        ("status_text", Json::String(status_text)),
    ])
}

fn solver_for(
    cache: &mut HashMap<String, Option<Arc<OpeningBook>>>,
    rules: &Rules,
) -> (ConnectKSolver, Option<String>) {
    let mut loaded: Vec<(PathBuf, Arc<OpeningBook>)> = Vec::new();
    for path in OpeningBook::candidate_paths_for_rules(rules) {
        let metadata = path
            .metadata()
            .map(|m| format!("{}:{:?}", m.len(), m.modified().ok()))
            .unwrap_or_default();
        let key = format!(
            "{}|{}|{}",
            path.to_string_lossy(),
            rules_line(rules),
            metadata
        );
        let book = cache
            .entry(key)
            .or_insert_with(|| {
                OpeningBook::load_for_rules(&path, rules)
                    .ok()
                    .flatten()
                    .filter(|book| book.size() > 0)
                    .map(Arc::new)
            })
            .clone();
        if let Some(book) = book {
            loaded.push((path, book));
        }
    }
    if loaded.is_empty() {
        return (ConnectKSolver::without_book(), None);
    }
    let (book, used_paths) = if loaded.len() == 1 {
        let (path, book) = loaded.into_iter().next().unwrap();
        (book, vec![path])
    } else {
        let merged = {
            let refs: Vec<&OpeningBook> = loaded.iter().map(|(_, book)| book.as_ref()).collect();
            OpeningBook::merged_refs(&refs).ok().map(Arc::new)
        };
        match merged {
            Some(book) => {
                let paths = loaded.into_iter().map(|(path, _)| path).collect();
                (book, paths)
            }
            None => {
                let (path, book) = loaded
                    .into_iter()
                    .max_by_key(|(_, book)| (book.size(), book.depth()))
                    .unwrap();
                (book, vec![path])
            }
        }
    };
    let names = used_paths
        .iter()
        .filter_map(|path| path.file_name())
        .map(|name| name.to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join(" + ");
    let label = format!("{} ({} entries)", names, book.size());
    (ConnectKSolver::with_book(Some(book)), Some(label))
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|error| error.into_inner())
}

#[cfg(test)]
mod tests {
    use super::{authorized, solver_for, valid_host, AppState, Request, Rules};
    use crate::json::Json;
    use std::collections::HashMap;

    #[test]
    fn shipped_standard_book_is_loaded_for_gui() {
        let mut cache = HashMap::new();
        let (solver, label) = solver_for(&mut cache, &Rules::standard_connect4());
        assert!(
            solver.book().is_some(),
            "GUI solver must carry the shipped book"
        );
        assert!(
            label
                .as_deref()
                .is_some_and(|label| label.contains("std7x6-d10.book")),
            "unexpected GUI book label: {label:?}"
        );
        assert!(
            solver.book().map(|book| book.size()).unwrap_or(0) >= 809_464,
            "GUI merged book must include the complete shipped frontier"
        );
    }

    #[test]
    fn host_and_token_checks() {
        assert!(valid_host(Some(&"127.0.0.1:8765".into()), 8765));
        assert!(valid_host(Some(&"localhost:8765".into()), 8765));
        assert!(valid_host(Some(&"[::1]:8765".into()), 8765));
        assert!(!valid_host(Some(&"example.invalid".into()), 8765));
        assert!(!valid_host(Some(&"127.0.0.1:9999".into()), 8765));
        std::env::set_var("CONNECTK_GUI_PREVIEW", "1");
        assert!(valid_host(Some(&"8765-preview.e2b.app".into()), 8765));
        std::env::remove_var("CONNECTK_GUI_PREVIEW");

        let state = AppState::new("secret".into());
        let mut headers = HashMap::new();
        headers.insert("x-connectk-token".into(), "secret".into());
        let request = Request {
            method: "POST".into(),
            path: "/api/get_state".into(),
            headers,
            body: Vec::new(),
        };
        assert!(authorized(&request, &state));
        let state_json = state
            .handle_command("get_state", &Json::object(vec![]))
            .unwrap();
        assert_eq!(state_json.get("width").unwrap().as_i32(), Some(7));
    }
}
