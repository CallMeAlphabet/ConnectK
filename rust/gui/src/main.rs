mod json;
mod native;
mod server;

use server::AppState;
use std::io::Read;
use std::net::TcpListener;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy, PartialEq, Eq)]
enum LaunchMode {
    Native,
    Browser,
    Web,
    Server,
}

struct Options {
    mode: LaunchMode,
    port: u16,
}

fn main() {
    let options = match parse_args() {
        Ok(Some(options)) => options,
        Ok(None) => return,
        Err(error) => {
            eprintln!("error: {error}\n");
            print_usage();
            std::process::exit(2);
        }
    };

    let state = Arc::new(AppState::new(make_token()));
    let bind_address = if std::env::var("CONNECTK_GUI_PREVIEW").as_deref() == Ok("1") {
        "0.0.0.0"
    } else {
        "127.0.0.1"
    };
    let listener = match TcpListener::bind((bind_address, options.port)) {
        Ok(listener) => listener,
        Err(error) => {
            eprintln!(
                "error: could not bind the local GUI server on port {}: {error}",
                options.port
            );
            std::process::exit(1);
        }
    };
    let address = listener
        .local_addr()
        .expect("bound listener has a local address");
    let url = if options.mode == LaunchMode::Web {
        format!("http://{address}/web")
    } else {
        format!("http://{address}/")
    };
    println!("ConnectK GUI: {url}");

    match options.mode {
        LaunchMode::Server => server::serve(listener, state),
        LaunchMode::Browser | LaunchMode::Web => {
            open_browser(&url);
            server::serve(listener, state);
        }
        LaunchMode::Native => {
            let server_state = state.clone();
            let server_thread = std::thread::spawn(move || server::serve(listener, server_state));
            match native::run_window(&url) {
                Ok(()) => {}
                Err(error) => {
                    eprintln!("Native window unavailable ({error}); opening a browser instead.");
                    open_browser(&url);
                    let _ = server_thread.join();
                }
            }
        }
    }
}

fn parse_args() -> Result<Option<Options>, String> {
    let mut mode = match std::env::var("CONNECTK_GUI_MODE").ok().as_deref() {
        Some("browser") => LaunchMode::Browser,
        Some("web") => LaunchMode::Web,
        Some("server") => LaunchMode::Server,
        _ => LaunchMode::Native,
    };
    let mut port = std::env::var("CONNECTK_GUI_PORT")
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(0);
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if let Some(value) = arg.strip_prefix("--port=") {
            port = value
                .parse::<u16>()
                .map_err(|_| "invalid --port value".to_string())?;
            continue;
        }
        match arg.as_str() {
            "--help" | "-h" | "help" => {
                print_usage();
                return Ok(None);
            }
            "--native" => mode = LaunchMode::Native,
            "--browser" => mode = LaunchMode::Browser,
            "--web" => mode = LaunchMode::Web,
            "--server" | "--no-window" => mode = LaunchMode::Server,
            "--port" => {
                let value = args.next().ok_or("--port requires a value")?;
                port = value
                    .parse::<u16>()
                    .map_err(|_| "invalid --port value".to_string())?;
            }
            _ => return Err(format!("unknown argument: {arg}")),
        }
    }
    Ok(Some(Options { mode, port }))
}

fn print_usage() {
    println!(
        "ConnectK GUI\n\nUsage: connectk-gui [--native|--browser|--web|--server] [--port N]\n\n  --native   open the GTK/WebKit desktop window (default)\n  --browser  open the full GUI in the default browser\n  --web      open the minimal web GUI in the default browser\n  --server   only serve the GUI on loopback and print its URL\n  --port N   loopback port (default: an ephemeral port)"
    );
}

fn open_browser(url: &str) {
    let opened = Command::new("xdg-open")
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    if opened.is_err() {
        eprintln!("Open this URL in a browser: {url}");
    }
}

fn make_token() -> String {
    let mut bytes = [0u8; 16];
    if std::fs::File::open("/dev/urandom")
        .and_then(|mut file| file.read_exact(&mut bytes))
        .is_err()
    {
        let mut value = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos() as u64)
            .unwrap_or(0)
            ^ ((std::process::id() as u64) << 32);
        for byte in &mut bytes {
            value ^= value << 13;
            value ^= value >> 7;
            value ^= value << 17;
            *byte = value as u8;
        }
    }
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut token = String::with_capacity(32);
    for byte in bytes {
        token.push(HEX[(byte >> 4) as usize] as char);
        token.push(HEX[(byte & 0x0f) as usize] as char);
    }
    token
}
