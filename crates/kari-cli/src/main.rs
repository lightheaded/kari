//! `kari-node`: kari without a window.
//!
//! `kari-node serve` runs the same engine as the desktop app and serves it on
//! loopback for a kari desktop app on another machine, which reaches it through
//! an SSH port forward. The other subcommands install the Claude Code hooks and
//! the status line wrapper on this host, and read the board of a running node.

mod service;
mod update;

use clap::{Parser, Subcommand};
use kari_core::{api, hooks, paths, statusline, Engine};
use std::net::SocketAddr;
use std::sync::Arc;

#[derive(Parser)]
#[command(name = "kari-node", version, about = "Headless kari node")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run the engine and serve the API. Stops on SIGINT or SIGTERM.
    Serve {
        /// Address to bind. Default: 127.0.0.1 and the hooks port from settings.
        /// A different port is saved, because the hook relay must post to it.
        /// Repeat the flag to listen on more than one address; the first one
        /// is the one the hook relay posts to.
        #[arg(long)]
        listen: Vec<SocketAddr>,
        /// Bind an address that is not loopback. The token is then the only
        /// guard, so put the node behind a private network. Keep loopback in
        /// the bind, for example `0.0.0.0`: the hook relay posts to
        /// 127.0.0.1 and stops working on an address that leaves it out.
        #[arg(long)]
        allow_remote: bool,
        /// Answer on every private address of this machine as well, so a hub on
        /// a phone can reach the node without an SSH forward. The list is read
        /// again every 20 seconds. A public address is never bound.
        #[arg(long)]
        private: bool,
        /// Set the node name other kari instances show. Empty keeps the host name.
        #[arg(long)]
        name: Option<String>,
        /// Turn on the OAuth usage endpoint as a quota source and keep it on.
        #[arg(long)]
        usage_endpoint: bool,
        /// Ask Haiku for session summaries on this node. `--summaries false`
        /// leaves the summaries to another node and saves quota.
        #[arg(long)]
        summaries: Option<bool>,
        /// Install the Claude Code hooks for this user before serving.
        #[arg(long)]
        install_hooks: bool,
        /// Install the status line wrapper for this user before serving.
        #[arg(long)]
        install_statusline: bool,
        /// Base URL of a kari server to link to, such as `http://kari:47312`.
        /// The node dials out and holds one socket, so it needs no reachable
        /// address of its own. Without this the node only serves --listen.
        #[arg(long)]
        server: Option<String>,
        /// File holding the token this server expects. Default: the value of
        /// KARI_SERVER_TOKEN, else ~/.config/kari/server-token.
        #[arg(long)]
        server_token_file: Option<std::path::PathBuf>,
        /// Replace this binary when a newer release appears, then stop so that
        /// the service manager starts the new one. Off unless asked for: a
        /// host that pins a version means that pin, and a node that changed
        /// itself under it would make the pin a lie.
        ///
        /// The unit must restart the node on a clean exit. systemd needs
        /// `Restart=always`; `Restart=on-failure` leaves the host with no node
        /// after the first update.
        #[arg(long)]
        auto_update: bool,
        /// Hours between update checks while serving.
        #[arg(long, default_value_t = 6)]
        update_every_hours: u64,
    },
    /// Manage the Claude Code hooks that report session events to this node.
    Hooks {
        #[command(subcommand)]
        action: HookAction,
    },
    /// Manage the status line wrapper that records rate limits.
    Statusline {
        #[command(subcommand)]
        action: StatuslineAction,
    },
    /// Print the board of the node that runs on this host, as JSON.
    Board {
        /// Port of the running node. Default: the hooks port from settings.
        #[arg(long)]
        port: Option<u16>,
    },
    /// Print what this node says on /kari/health.
    Identity,
    /// Run the node at login, and keep it running.
    Service {
        #[command(subcommand)]
        cmd: ServiceCmd,
    },
    /// Replace this binary with the newest release.
    Update {
        /// Say what would happen and change nothing.
        #[arg(long)]
        check: bool,
    },
}

#[derive(Subcommand)]
enum ServiceCmd {
    /// Write the service file and start the node.
    Install {
        /// Link the node to this server, as `serve --server` does.
        #[arg(long)]
        server: Option<String>,
        /// Let the node replace its own binary while it serves.
        #[arg(long)]
        auto_update: bool,
    },
    /// Stop the node and remove the service file.
    Uninstall,
    /// Say whether the service is installed and running.
    Status,
}

#[derive(Subcommand)]
enum HookAction {
    Install,
    Uninstall,
    /// Post one hook event to the node on this host. Claude Code runs this on
    /// every event and hands it the payload on stdin. It is what Windows
    /// registers in settings.json in place of the `sh` relay script, and it
    /// always exits 0, so a stopped node never disturbs a session.
    Relay {
        /// Port of the running node. Default: the hooks port from settings.
        #[arg(long)]
        port: Option<u16>,
    },
}

#[derive(Subcommand)]
enum StatuslineAction {
    Install,
    Uninstall,
    /// Record the rate limits from one status line refresh, then run the
    /// command kari wrapped and print what it prints. Claude Code runs this and
    /// hands it the payload on stdin. The Windows counterpart of the `bash`
    /// wrapper script.
    Capture,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("kari_core=info".parse().unwrap())
                .add_directive("kari_node=info".parse().unwrap()),
        )
        .with_writer(std::io::stderr)
        .init();
    match cli.cmd {
        Cmd::Serve {
            listen,
            allow_remote,
            private,
            name,
            usage_endpoint,
            summaries,
            install_hooks,
            install_statusline,
            server,
            server_token_file,
            auto_update,
            update_every_hours,
        } => serve(Serve {
            listen,
            allow_remote,
            private,
            name,
            usage_endpoint,
            summaries,
            install_hooks,
            install_statusline,
            server,
            server_token_file,
            auto_update,
            update_every_hours,
        }),
        Cmd::Hooks { action } => match action {
            // The relay runs on every hook event, so it touches the database
            // only when the registered command carries no port.
            HookAction::Relay { port } => {
                let payload = read_stdin()?;
                let port = match port {
                    Some(p) => p,
                    None => Engine::open()?.settings().hooks_port,
                };
                print!("{}", hooks::relay(&payload, port));
                Ok(())
            }
            HookAction::Install => {
                println!("{}", Engine::open()?.install_hooks()?);
                Ok(())
            }
            HookAction::Uninstall => {
                Engine::open()?.uninstall_hooks()?;
                println!("hooks removed");
                Ok(())
            }
        },
        Cmd::Statusline { action } => match action {
            StatuslineAction::Capture => {
                print!("{}", statusline::capture(&read_stdin()?));
                Ok(())
            }
            StatuslineAction::Install => {
                println!("{}", statusline::install()?);
                Ok(())
            }
            StatuslineAction::Uninstall => {
                println!("{}", statusline::uninstall()?);
                Ok(())
            }
        },
        Cmd::Board { port } => {
            let port = match port {
                Some(p) => p,
                None => Engine::open()?.settings().hooks_port,
            };
            let token = std::fs::read_to_string(paths::hook_token_file())
                .map_err(|_| anyhow::anyhow!("no token file; is a node running on this host?"))?;
            let client = kari_core::client::ApiClient::new(port, token.trim());
            let board = client.board()?;
            println!("{}", serde_json::to_string_pretty(&board)?);
            Ok(())
        }
        Cmd::Identity => {
            let engine = Engine::open()?;
            println!("{}", serde_json::to_string_pretty(&engine.identity())?);
            Ok(())
        }
        Cmd::Service { cmd } => {
            let msg = match cmd {
                ServiceCmd::Install {
                    server,
                    auto_update,
                } => service::install(service::Options {
                    server,
                    auto_update,
                })?,
                ServiceCmd::Uninstall => service::uninstall()?,
                ServiceCmd::Status => service::status()?,
            };
            println!("{msg}");
            Ok(())
        }
        Cmd::Update { check } => {
            let running = update::current();
            if check {
                let latest = update::latest()?;
                if update::is_newer(&latest.version, running) {
                    println!("{running} is running; {} is out", latest.version);
                } else {
                    println!("{running} is the newest release");
                }
                return Ok(());
            }
            let out = update::update()?;
            if out.replaced {
                println!(
                    "updated {} to {}. Start kari-node again to run it.",
                    out.from, out.to
                );
            } else {
                println!("{} is the newest release", out.from);
            }
            Ok(())
        }
    }
}

/// Everything Claude Code wrote to this process. Both the relay and the status
/// line capture are handed one JSON payload that way.
fn read_stdin() -> anyhow::Result<String> {
    use std::io::Read;
    let mut s = String::new();
    std::io::stdin().read_to_string(&mut s)?;
    Ok(s)
}

/// What `serve` was asked to do. A struct, because a service unit sets most of it.
struct Serve {
    listen: Vec<SocketAddr>,
    allow_remote: bool,
    private: bool,
    name: Option<String>,
    usage_endpoint: bool,
    summaries: Option<bool>,
    install_hooks: bool,
    install_statusline: bool,
    server: Option<String>,
    server_token_file: Option<std::path::PathBuf>,
    auto_update: bool,
    update_every_hours: u64,
}

fn serve(opt: Serve) -> anyhow::Result<()> {
    let Serve {
        listen,
        allow_remote,
        private,
        name,
        usage_endpoint,
        summaries,
        install_hooks,
        install_statusline,
        server,
        server_token_file,
        auto_update,
        update_every_hours,
    } = opt;
    // A Windows update leaves the binary it replaced beside the new one. The
    // first start after that is the earliest moment nothing holds it open.
    update::sweep();
    let engine = Engine::open()?;
    let mut settings = engine.settings();
    let mut changed = false;
    if let Some(n) = name {
        settings.node_name = n;
        changed = true;
    }
    if usage_endpoint && !settings.usage_endpoint_enabled {
        settings.usage_endpoint_enabled = true;
        changed = true;
    }
    if let Some(on) = summaries {
        if settings.summaries_enabled != on {
            settings.summaries_enabled = on;
            changed = true;
        }
    }
    let addrs = if listen.is_empty() {
        vec![SocketAddr::from(([127, 0, 0, 1], settings.hooks_port))]
    } else {
        listen
    };
    if addrs[0].port() != settings.hooks_port {
        settings.hooks_port = addrs[0].port();
        changed = true;
    }

    // One engine per machine, and the window wins. A daemon that started while
    // the app was open would bind nothing, flicker the machine on every other
    // client's board and plan against a budget it shares — see `owner`. So it
    // waits here, for as long as the app is open, and serves nothing until the
    // machine is its own.
    let _owned = kari_core::owner::take_after_wait(kari_core::owner::DAEMON, addrs[0].port())?;
    kari_core::owner::step_down_when_taken(kari_core::owner::DAEMON, addrs[0].port(), |h| {
        tracing::info!(
            "{} (pid {}) took the engine of this machine; stopping so it can serve",
            h.what,
            h.pid
        );
        // Exit rather than close the engine in place: it holds watchers, a
        // port, a link and a planner. Whatever supervises this node starts it
        // again, and it comes back waiting.
        std::process::exit(0);
    });

    if changed {
        engine.set_settings(settings)?;
    }
    if install_hooks {
        match engine.install_hooks() {
            Ok(m) => tracing::info!("{m}"),
            Err(e) => tracing::warn!("hooks not installed: {e}"),
        }
    } else if hooks::installed() && !hooks::held_event_installed() {
        tracing::warn!(
            "hooks installed without the PermissionRequest entry; run `kari-node hooks install` again for Away mode"
        );
    } else if hooks::installed() {
        tracing::info!("hooks installed");
    } else {
        tracing::info!(
            "hooks not installed; run `kari-node hooks install` for live session events"
        );
    }
    if install_statusline {
        match statusline::install() {
            Ok(m) => tracing::info!("{m}"),
            Err(e) => tracing::warn!("status line wrapper not installed: {e}"),
        }
    }
    let identity = engine.identity();
    tracing::info!(
        "node {} ({}) v{}",
        identity.node_name,
        identity.node_id,
        identity.version
    );
    engine.start_watchers();

    // The link is additional, never instead: the node keeps serving loopback
    // for the hook relay and for a desktop app on this host, whether or not a
    // server is configured or reachable.
    let link = match &server {
        Some(url) => Some((url.clone(), server_token(server_token_file)?)),
        None => None,
    };

    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async move {
        if let Some((url, token)) = link {
            let e = Arc::clone(&engine);
            let local = hooks::token()?;
            tracing::info!("linking to the server at {url}");
            tokio::spawn(kari_core::link::run(e, url, token, local));
        }
        // The updater has one thing to say, once: the version it wrote.
        let (tx, rx) = tokio::sync::oneshot::channel::<String>();
        if auto_update {
            tokio::spawn(watch_for_updates(update_every_hours, tx));
        }
        // With no updater running the sender is already dropped, and an awaited
        // receiver would answer `Err` at once and stop the node on start. This
        // waits for good instead, so the arm below only ever fires on a real
        // update.
        let updated = async move {
            match rx.await {
                Ok(v) => v,
                Err(_) => std::future::pending::<String>().await,
            }
        };

        let server = api::serve_dynamic(Arc::clone(&engine), addrs, allow_remote, private);
        tokio::select! {
            r = server => r,
            _ = shutdown() => {
                tracing::info!("stopping");
                Ok(())
            }
            v = updated => {
                tracing::info!(
                    "kari-node {v} is installed; stopping so the service manager starts it"
                );
                Ok(())
            }
        }
    })
}

/// Look for a newer release for as long as the node serves, and install one.
///
/// The first check waits a minute rather than running on start. An update that
/// does not raise the version this binary reports would otherwise update,
/// stop, start, and update again with nothing between the attempts; a minute
/// makes that a slow, visible loop in the log instead of a spin.
async fn watch_for_updates(every_hours: u64, tx: tokio::sync::oneshot::Sender<String>) {
    let mut wait = std::time::Duration::from_secs(60);
    let every = std::time::Duration::from_secs(every_hours.max(1) * 3600);
    loop {
        tokio::time::sleep(wait).await;
        wait = every;
        // Fetching a release and writing a file both block. Off the runtime
        // threads, or they stop answering the board while a download runs.
        match tokio::task::spawn_blocking(update::update).await {
            Ok(Ok(o)) if o.replaced => {
                tracing::info!("updated kari-node {} to {}", o.from, o.to);
                let _ = tx.send(o.to);
                return;
            }
            Ok(Ok(_)) => {}
            // A check that fails is not a reason to stop serving. The network
            // is down, or GitHub is, and the next check is in a few hours.
            Ok(Err(e)) => tracing::warn!("update check failed: {e}"),
            Err(e) => tracing::warn!("update check did not finish: {e}"),
        }
    }
}

/// The token this node presents to its server. A file the operator placed, the
/// environment, or the default path — in that order, because a service unit
/// sets the environment and a person on the host uses the file.
fn server_token(file: Option<std::path::PathBuf>) -> anyhow::Result<String> {
    if let Some(p) = file {
        let t = std::fs::read_to_string(&p)
            .map_err(|e| anyhow::anyhow!("cannot read {}: {e}", p.display()))?;
        return Ok(t.trim().to_string());
    }
    if let Ok(t) = std::env::var("KARI_SERVER_TOKEN") {
        if !t.trim().is_empty() {
            return Ok(t.trim().to_string());
        }
    }
    let p = paths::server_token_file();
    let t = std::fs::read_to_string(&p).map_err(|_| {
        anyhow::anyhow!(
            "--server needs a token: put the server's token in {} , set KARI_SERVER_TOKEN, \
             or pass --server-token-file. Print it on the server with `kari-server token`.",
            p.display()
        )
    })?;
    Ok(t.trim().to_string())
}

async fn shutdown() {
    let ctrl_c = tokio::signal::ctrl_c();
    #[cfg(unix)]
    {
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("SIGTERM handler");
        tokio::select! {
            _ = ctrl_c => {}
            _ = term.recv() => {}
        }
    }
    #[cfg(not(unix))]
    {
        let _ = ctrl_c.await;
    }
}
