// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

/// On Windows the app binary is also the hook relay and the status line
/// capture, and this runs before Tauri does.
///
/// `hooks install` and `statusline install` register the command that Claude
/// Code will run on every event. On Unix that command is a small `sh` script
/// kari writes. Windows has no `sh`, so both register
/// `std::env::current_exe()` with a subcommand instead — and inside the app
/// that path is `kari.exe`. Without this branch, installing hooks from
/// Settings would point every hook event at a window, which is worse than
/// having no hooks: Claude Code would wait on a GUI that never answers.
///
/// Only the commands that actually get registered live here, plus the two
/// status line installers, which is how a Windows user gets quota meters at
/// all — the app hands macOS a shell script to run, and that script is no use
/// here. Everything else remains the node's job.
///
/// `windows_subsystem = "windows"` allocates no console, but Claude Code
/// spawns the relay with its pipes attached, so the inherited stdin and stdout
/// are the ones it is reading and writing.
#[cfg(windows)]
fn ran_as_cli() -> bool {
    use kari_core::{engine::Engine, hooks, statusline};
    use std::io::Read;

    /// Whatever Claude Code wrote to this process, or an empty payload. The
    /// relay must never fail: a hook that errors is a hook that blocks a turn.
    fn stdin_payload() -> String {
        let mut s = String::new();
        let _ = std::io::stdin().read_to_string(&mut s);
        s
    }

    let args: Vec<String> = std::env::args().skip(1).collect();
    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    match words.as_slice() {
        // `--port` is always registered by the installer. The fallback reads
        // it from the database, and the node's relay avoids that read for the
        // same reason: this runs on every single hook event.
        ["hooks", "relay", rest @ ..] => {
            let port = rest
                .iter()
                .position(|w| *w == "--port")
                .and_then(|i| rest.get(i + 1))
                .and_then(|p| p.parse::<u16>().ok())
                .or_else(|| Engine::open().ok().map(|e| e.settings().hooks_port));
            let payload = stdin_payload();
            if let Some(port) = port {
                print!("{}", hooks::relay(&payload, port));
            }
            true
        }
        ["statusline", "capture"] => {
            print!("{}", statusline::capture(&stdin_payload()));
            true
        }
        ["statusline", "install"] => {
            match statusline::install() {
                Ok(msg) => println!("{msg}"),
                Err(e) => eprintln!("{e}"),
            }
            true
        }
        ["statusline", "uninstall"] => {
            match statusline::uninstall() {
                Ok(msg) => println!("{msg}"),
                Err(e) => eprintln!("{e}"),
            }
            true
        }
        _ => false,
    }
}

fn main() {
    #[cfg(windows)]
    if ran_as_cli() {
        return;
    }
    kari_lib::run();
}
