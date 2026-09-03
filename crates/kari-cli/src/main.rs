//! `kari-node`: kari without a window.
//!
//! `kari-node serve` runs the same engine as the desktop app and serves it on
//! loopback for a kari desktop app on another machine, which reaches it through
//! an SSH port forward. The other subcommands install the Claude Code hooks and
//! the status line wrapper on this host, and read the board of a running node.

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
    },
    /// Manage the Claude Code hooks that report session events to this node.
    Hooks {
        #[command(subcommand)]
        action: Action,
    },
    /// Manage the status line wrapper that records rate limits.
    Statusline {
        #[command(subcommand)]
        action: Action,
    },
    /// Print the board of the node that runs on this host, as JSON.
    Board {
        /// Port of the running node. Default: the hooks port from settings.
        #[arg(long)]
        port: Option<u16>,
    },
    /// Print what this node says on /kari/health.
    Identity,
}

#[derive(Subcommand)]
enum Action {
    Install,
    Uninstall,
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
        } => serve(Serve {
            listen,
            allow_remote,
            private,
            name,
            usage_endpoint,
            summaries,
            install_hooks,
            install_statusline,
        }),
        Cmd::Hooks { action } => {
            let engine = Engine::open()?;
            match action {
                Action::Install => println!("{}", engine.install_hooks()?),
                Action::Uninstall => {
                    engine.uninstall_hooks()?;
                    println!("hooks removed");
                }
            }
            Ok(())
        }
        Cmd::Statusline { action } => {
            match action {
                Action::Install => println!("{}", statusline::install()?),
                Action::Uninstall => println!("{}", statusline::uninstall()?),
            }
            Ok(())
        }
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
    }
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
    } = opt;
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

    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async move {
        let server = api::serve_dynamic(Arc::clone(&engine), addrs, allow_remote, private);
        tokio::select! {
            r = server => r,
            _ = shutdown() => {
                tracing::info!("stopping");
                Ok(())
            }
        }
    })
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
