//! `kari-server`: the hub on a host that stays up.
//!
//! Nodes dial this process and hold one socket each; it calls their APIs back
//! down those sockets and answers for all of them at once. It runs no Claude
//! Code, holds no login and reads no transcript — the only thing it needs from
//! the host is a directory to keep its token in.
//!
//! It binds a private address and checks one token. That is the same trust a
//! node on a private address has always had: the network carries it, and the
//! token keeps other processes on the host out.

use clap::{Parser, Subcommand};
use kari_core::{hooks, link::LinkRegistry, paths, server};
use std::net::SocketAddr;
use std::sync::Arc;

/// The port a server listens on, one above the node's, so both can run on one
/// host without a flag.
const DEFAULT_PORT: u16 = 47312;

#[derive(Parser)]
#[command(
    name = "kari-server",
    version,
    about = "kari server: one board over many nodes"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Accept node links and serve the API. Stops on SIGINT or SIGTERM.
    Serve {
        /// Address to bind. Default: every address on this host, port 47312.
        /// A public address is refused; see --allow-public.
        #[arg(long)]
        listen: Option<SocketAddr>,
        /// Bind an address that is not loopback and not private. The server
        /// carries no TLS and checks one token, so this is not a supported
        /// deployment — put it behind a VPN instead.
        #[arg(long)]
        allow_public: bool,
    },
    /// Print the token a node needs to dial this server, creating it if this is
    /// the first run.
    Token,
    /// Print what a running server says on /kari/health.
    Health {
        /// Base URL of the server. Default: this host, port 47312.
        #[arg(long)]
        url: Option<String>,
    },
    /// Print the roster of a running server as JSON.
    Nodes {
        #[arg(long)]
        url: Option<String>,
    },
}

fn token() -> anyhow::Result<String> {
    hooks::token_in(&paths::server_token_file())
}

fn base(url: Option<String>) -> String {
    url.unwrap_or_else(|| format!("http://127.0.0.1:{DEFAULT_PORT}"))
        .trim_end_matches('/')
        .to_string()
}

/// Ask a running server one question, with the token this host holds.
fn ask(url: Option<String>, path: &str) -> anyhow::Result<serde_json::Value> {
    let base = base(url);
    let resp = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()?
        .get(format!("{base}{path}"))
        .header(hooks::TOKEN_HEADER, token()?)
        .send()?;
    if !resp.status().is_success() {
        anyhow::bail!("{}: {}", resp.status(), resp.text().unwrap_or_default());
    }
    Ok(resp.json()?)
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("kari_core=info".parse().unwrap())
                .add_directive("kari_server=info".parse().unwrap()),
        )
        .with_writer(std::io::stderr)
        .init();

    match cli.cmd {
        Cmd::Token => {
            println!("{}", token()?);
            Ok(())
        }
        Cmd::Health { url } => {
            println!("{:#}", ask(url, "/kari/health")?);
            Ok(())
        }
        Cmd::Nodes { url } => {
            println!("{:#}", ask(url, "/kari/v1/nodes")?);
            Ok(())
        }
        Cmd::Serve {
            listen,
            allow_public,
        } => {
            let addr = listen.unwrap_or_else(|| SocketAddr::from(([0, 0, 0, 0], DEFAULT_PORT)));
            let token = token()?;
            let registry = Arc::new(LinkRegistry::new());
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()?
                .block_on(async move {
                    tokio::select! {
                        r = server::serve(registry, addr, token, allow_public) => r,
                        _ = shutdown() => Ok(()),
                    }
                })
        }
    }
}

/// SIGINT or SIGTERM, so a service manager can stop this cleanly.
async fn shutdown() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut term = signal(SignalKind::terminate()).expect("sigterm");
        let mut int = signal(SignalKind::interrupt()).expect("sigint");
        tokio::select! {
            _ = term.recv() => {}
            _ = int.recv() => {}
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}
