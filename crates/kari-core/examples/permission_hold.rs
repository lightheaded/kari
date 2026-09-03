//! The held permission prompt, end to end against a real node process.
//!
//! Run: `cargo run -p kari-core --example permission_hold`
//!
//! Starts `kari-node serve` in a scratch home, turns Away mode on, and plays
//! the relay: it posts a `PermissionRequest` hook payload and waits for the
//! answer, as the relay script does. A second thread answers it through the
//! API, as a phone does. Then the same with no answer and a short hold, and
//! once more with Away mode off. It needs `cargo build -p kari-cli` first.

use kari_core::client::ApiClient;
use kari_core::hooks;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

struct Node {
    child: Child,
    dir: std::path::PathBuf,
}

impl Drop for Node {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn payload(session: &str) -> serde_json::Value {
    serde_json::json!({
        "session_id": session,
        "hook_event_name": hooks::HELD_EVENT,
        "cwd": "/tmp",
        "tool_name": "Bash",
        "tool_input": { "command": "cargo test --workspace" },
    })
}

/// What the relay does: post the payload, return the body.
fn relay(port: u16, token: &str, body: &serde_json::Value) -> anyhow::Result<serde_json::Value> {
    let http = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(hooks::HELD_TIMEOUT_SECS))
        .build()?;
    let r = http
        .post(format!("http://127.0.0.1:{port}{}", hooks::HOOK_PATH))
        .header(hooks::TOKEN_HEADER, token)
        .json(body)
        .send()?;
    Ok(r.json()?)
}

fn main() -> anyhow::Result<()> {
    let bin = std::path::Path::new("target/debug/kari-node");
    if !bin.exists() {
        anyhow::bail!("build the node first: cargo build -p kari-cli");
    }
    let port = kari_core::tunnel::free_port()?;
    let home = std::env::temp_dir().join(format!("kari-perm-example-{port}"));
    std::fs::create_dir_all(home.join(".claude/projects"))?;
    let child = Command::new(bin)
        .args(["serve", "--listen", &format!("127.0.0.1:{port}"), "--summaries", "false"])
        .env("HOME", &home)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    let _node = Node {
        child,
        dir: home.clone(),
    };
    let token_file = home.join(".config/kari/hook-token");
    let deadline = Instant::now() + Duration::from_secs(30);
    let token = loop {
        if let Ok(t) = std::fs::read_to_string(&token_file) {
            if t.trim().len() >= 32 {
                break t.trim().to_string();
            }
        }
        if Instant::now() > deadline {
            anyhow::bail!("the node wrote no token file");
        }
        std::thread::sleep(Duration::from_millis(200));
    };
    let client = ApiClient::at(&format!("http://127.0.0.1:{port}"), &token);
    let deadline = Instant::now() + Duration::from_secs(20);
    while client.health().is_err() {
        if Instant::now() > deadline {
            anyhow::bail!("node did not answer health");
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    println!("node       up on {port}");

    // 1. Away mode off: the hook answers at once with no decision.
    let t0 = Instant::now();
    let r = relay(port, &token, &payload("sess-off-000001"))?;
    anyhow::ensure!(r == serde_json::json!({}), "expected no decision, got {r}");
    anyhow::ensure!(t0.elapsed() < Duration::from_secs(2), "the hook waited with Away mode off");
    println!("desk       Away mode off: no hold, no decision ({:?})", t0.elapsed());

    // 2. Away mode on, the phone allows.
    let mut s = client.settings()?;
    s.away_mode = true;
    s.away_hold_secs = 30;
    client.set_settings(&s)?;
    let (p2, t2) = (port, token.clone());
    let held = std::thread::spawn(move || relay(p2, &t2, &payload("sess-allow-00001")));
    let deadline = Instant::now() + Duration::from_secs(10);
    let pending = loop {
        let list = client.permissions()?;
        if let Some(p) = list.first() {
            break p.clone();
        }
        if Instant::now() > deadline {
            anyhow::bail!("the node holds no prompt");
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    println!(
        "held       {} on {} until {}",
        pending.tool_name,
        pending.session_id,
        pending.until.format("%H:%M:%S")
    );
    let board = client.board()?;
    let card = board
        .cards
        .iter()
        .find(|c| c.card.session_id.as_deref() == Some("sess-allow-00001"))
        .ok_or_else(|| anyhow::anyhow!("no card for the held session"))?;
    anyhow::ensure!(card.permission.is_some(), "the card does not carry the prompt");
    println!("card       {:?} carries the prompt", card.state);
    client.answer_permission(&pending.id, "allow")?;
    let r = held.join().expect("relay thread")?;
    anyhow::ensure!(r == hooks::decision_json("allow"), "unexpected relay output {r}");
    anyhow::ensure!(client.permissions()?.is_empty(), "the prompt is still listed");
    println!("allowed    the relay got {r}");

    // 3. Away mode on, nobody answers: the hold runs out with no decision.
    let mut s = client.settings()?;
    s.away_hold_secs = 5;
    client.set_settings(&s)?;
    let t0 = Instant::now();
    let r = relay(port, &token, &payload("sess-quiet-00001"))?;
    anyhow::ensure!(r == serde_json::json!({}), "expected no decision after the hold, got {r}");
    anyhow::ensure!(t0.elapsed() >= Duration::from_secs(5), "the hold ended early");
    anyhow::ensure!(client.permissions()?.is_empty(), "the timed-out prompt is still listed");
    println!("timeout    no answer in 5 s: no decision, prompt dropped ({:?})", t0.elapsed());

    // 4. A late answer is refused.
    let err = client.answer_permission(&pending.id, "allow").unwrap_err().to_string();
    anyhow::ensure!(err.contains("no longer held"), "{err}");
    println!("late       a second answer is refused: {err}");
    println!("ok");
    Ok(())
}
