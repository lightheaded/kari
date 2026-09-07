//! Start a server, link a real node to it, and read the node's board back
//! through the link.
//!
//! Run: `cargo run -p kari-core --example link_server`
//!
//! Needs both binaries: `cargo build -p kari-cli`.
//!
//! Nothing here touches your own kari. The server and the node each get their
//! own home directory, so they make their own databases and their own tokens
//! and both are deleted when the example ends. The point it proves is the one
//! the link exists for: the node opens the connection, and the server calls the
//! node's ordinary API back down it.
//!
//! It then proves the second half: the server keeps a *hub* over those links,
//! so `/kari/v1/hub/board` answers with one merged board and the columns the
//! server owns — which is what a client will ask for instead of running a hub
//! of its own.

use kari_core::hubapi::HubApi;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

struct Proc {
    child: Child,
    dir: std::path::PathBuf,
}

impl Drop for Proc {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn put(url: &str, token: &str, body: &serde_json::Value) -> anyhow::Result<()> {
    let resp = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()?
        .put(url)
        .header(kari_core::hooks::TOKEN_HEADER, token)
        .json(body)
        .send()?;
    let status = resp.status();
    if !status.is_success() {
        anyhow::bail!("{status}: {}", resp.text().unwrap_or_default());
    }
    Ok(())
}

fn get(url: &str, token: &str) -> anyhow::Result<serde_json::Value> {
    let resp = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()?
        .get(url)
        .header(kari_core::hooks::TOKEN_HEADER, token)
        .send()?;
    let status = resp.status();
    let text = resp.text().unwrap_or_default();
    if !status.is_success() {
        anyhow::bail!("{status}: {text}");
    }
    Ok(serde_json::from_str(&text)?)
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("kari_core=info")
        .init();

    let server_bin = std::path::Path::new("target/debug/kari-server");
    let node_bin = std::path::Path::new("target/debug/kari-node");
    for b in [server_bin, node_bin] {
        if !b.exists() {
            anyhow::bail!("build them first: cargo build -p kari-cli");
        }
    }

    let port = kari_core::tunnel::free_port()?;
    let node_port = kari_core::tunnel::free_port()?;

    // The server, with its own home so it makes its own token.
    let server_home = std::env::temp_dir().join(format!("kari-link-example-server-{port}"));
    std::fs::create_dir_all(&server_home)?;
    let server = Proc {
        child: Command::new(server_bin)
            .args(["serve", "--listen", &format!("127.0.0.1:{port}")])
            .env("HOME", &server_home)
            .env("XDG_CONFIG_HOME", server_home.join(".config"))
            .stdout(Stdio::null())
            .spawn()?,
        dir: server_home.clone(),
    };
    println!("server     http://127.0.0.1:{port}");

    // Its token, which the node must present. On a real host this is copied by
    // hand, or by whatever manages the node's configuration.
    let token_path = server_home.join(".config/kari/server-token");
    let deadline = Instant::now() + Duration::from_secs(20);
    let token = loop {
        if let Ok(t) = std::fs::read_to_string(&token_path) {
            if t.trim().len() >= 32 {
                break t.trim().to_string();
            }
        }
        if Instant::now() > deadline {
            anyhow::bail!("the server never wrote its token");
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    println!("token      {}…", &token[..8]);

    // The node, which dials the server rather than waiting to be dialled.
    let node_home = std::env::temp_dir().join(format!("kari-link-example-node-{node_port}"));
    std::fs::create_dir_all(node_home.join(".claude/projects"))?;
    let node = Proc {
        child: Command::new(node_bin)
            .args([
                "serve",
                "--listen",
                &format!("127.0.0.1:{node_port}"),
                "--name",
                "example-node",
                "--summaries",
                "false",
                "--server",
                &format!("http://127.0.0.1:{port}"),
            ])
            .env("HOME", &node_home)
            .env("XDG_CONFIG_HOME", node_home.join(".config"))
            .env("KARI_SERVER_TOKEN", &token)
            .stdout(Stdio::null())
            .spawn()?,
        dir: node_home.clone(),
    };

    // Wait for the roster to name it.
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        if let Ok(v) = get(&format!("http://127.0.0.1:{port}/kari/v1/nodes"), &token) {
            if v.as_array()
                .is_some_and(|a| a.iter().any(|n| n["online"].as_bool().unwrap_or(false)))
            {
                println!("\nroster\n{v:#}");
                break;
            }
        }
        if Instant::now() > deadline {
            anyhow::bail!("the node never linked");
        }
        std::thread::sleep(Duration::from_millis(250));
    }

    // The board, fetched by the server over the link the node opened.
    let board = get(&format!("http://127.0.0.1:{port}/kari/v1/board"), &token)?;
    let boards = board["boards"].as_array().cloned().unwrap_or_default();
    println!("\nboards fetched over the link: {}", boards.len());
    for b in &boards {
        let cards = b["board"]["cards"].as_array().map(|a| a.len()).unwrap_or(0);
        let cols = b["board"]["columns"]
            .as_array()
            .map(|a| a.len())
            .unwrap_or(0);
        println!(
            "  {}: {cols} columns, {cards} cards{}",
            b["node_name"].as_str().unwrap_or("?"),
            b["error"]
                .as_str()
                .map(|e| format!("  error: {e}"))
                .unwrap_or_default()
        );
    }
    if boards.is_empty() {
        anyhow::bail!("the server got no board over the link");
    }

    // ---- the hub the server keeps over those links ------------------------

    let base = format!("http://127.0.0.1:{port}");

    // The hub finds the node the same way it finds any other, so give the
    // watcher a moment to have connected and taken the first board.
    let deadline = Instant::now() + Duration::from_secs(60);
    let hub_board = loop {
        let v = get(&format!("{base}/kari/v1/hub/board"), &token)?;
        let nodes = v["nodes"].as_array().cloned().unwrap_or_default();
        if nodes.iter().any(|n| n["online"].as_bool().unwrap_or(false)) {
            break v;
        }
        if Instant::now() > deadline {
            anyhow::bail!("the hub never saw the node online:\n{v:#}");
        }
        std::thread::sleep(Duration::from_millis(250));
    };
    let nodes = hub_board["nodes"].as_array().cloned().unwrap_or_default();
    println!("\nhub board: {} node(s)", nodes.len());
    for n in &nodes {
        println!(
            "  {} online={}",
            n["name"]
                .as_str()
                .or(n["node_name"].as_str())
                .unwrap_or("?"),
            n["online"].as_bool().unwrap_or(false)
        );
    }

    // The columns are the server's, not the node's. Change them here and the
    // hub pushes them down the link, which is the thing a per-client hub could
    // not do without two hubs fighting over one node.
    let cols = get(&format!("{base}/kari/v1/hub/columns"), &token)?;
    let before = cols.as_array().map(|a| a.len()).unwrap_or(0);
    println!("\ncolumns owned by the server: {before}");
    if before == 0 {
        anyhow::bail!("the server's hub has no columns");
    }

    let mut renamed = cols.as_array().cloned().unwrap_or_default();
    let was = renamed[0]["name"].as_str().unwrap_or("").to_string();
    renamed[0]["name"] = serde_json::Value::String(format!("{was} (server)"));
    put(
        &format!("{base}/kari/v1/hub/columns"),
        &token,
        &serde_json::Value::Array(renamed),
    )?;
    let after = get(&format!("{base}/kari/v1/hub/columns"), &token)?;
    let now = after[0]["name"].as_str().unwrap_or("");
    println!("first column: {was:?} -> {now:?}");
    if !now.ends_with("(server)") {
        anyhow::bail!("the column change did not stick: {after:#}");
    }

    // And the node was told, because the hub owns the columns of every node
    // that linked to it.
    let node_token = std::fs::read_to_string(node_home.join(".config/kari/hook-token"))?;
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let n = get(
            &format!("http://127.0.0.1:{node_port}/kari/v1/columns"),
            node_token.trim(),
        )?;
        if n[0]["name"].as_str().unwrap_or("").ends_with("(server)") {
            println!("the node took the server's columns");
            break;
        }
        if Instant::now() > deadline {
            anyhow::bail!("the node never took the server's columns:\n{n:#}");
        }
        std::thread::sleep(Duration::from_millis(250));
    }

    // ---- a client that holds no hub at all --------------------------------
    //
    // This is what the desktop app and the phone become: a RemoteHub behind
    // the same HubApi the in-process hub implements, so the UI above it cannot
    // tell the difference.

    let client_home = std::env::temp_dir().join(format!("kari-link-example-client-{port}"));
    std::fs::create_dir_all(&client_home)?;
    let store = kari_core::Engine::open_at(&client_home.join("kari"))?;
    let remote = kari_core::remote::RemoteHub::connect(&base, &token, store);

    let id = remote.health()?;
    println!(
        "\nclient sees: {} {} ({} node(s) linked)",
        id.app, id.version, id.nodes_online
    );

    // Every one of these goes over HTTP to the server, which asks the hub,
    // which asks the node down the socket the node opened.
    let board = remote.board();
    println!(
        "board through the client: {} node(s), {} column(s), {} card(s)",
        board.nodes.len(),
        board.columns.len(),
        board.cards.len()
    );
    if board.nodes.is_empty() {
        anyhow::bail!("the client saw no nodes");
    }
    if !board.columns[0].name.ends_with("(server)") {
        anyhow::bail!(
            "the client did not see the server's columns: {:?}",
            board.columns[0].name
        );
    }

    // A card added through the client must reach the node, because that is the
    // whole chain: client -> server -> hub -> link -> node.
    let node_id = board.nodes[0].id.clone();
    let projects = remote.projects(&node_id);
    println!("projects on {}: {}", board.nodes[0].name, projects.len());

    let card = remote.add_task(
        &node_id,
        kari_core::NewTask {
            title: "from a client with no hub".into(),
            project_cwd: None,
            run_prompt: None,
            auto_run: false,
            priority: 0,
            notes: None,
            model: None,
            column_id: None,
        },
    )?;
    println!("card {} added through the client", card.id);

    let after = remote.board();
    if !after.cards.iter().any(|c| c.view.card.id == card.id) {
        anyhow::bail!("the card did not come back on the board");
    }
    println!("and it came back on the merged board");

    // The roster methods are refused, on purpose and with a reason.
    match remote.pairing_code() {
        Err(e) => println!("roster refused, as it should be: {e}"),
        Ok(_) => anyhow::bail!("a client of a server should not hand out pairing codes"),
    }

    drop(node);
    drop(server);
    let _ = std::fs::remove_dir_all(&client_home);
    println!("\ndone");
    Ok(())
}
