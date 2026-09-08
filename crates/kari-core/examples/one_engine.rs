//! One engine per machine: a daemon holds it, a window takes it, the daemon
//! steps down.
//!
//! Run: `cargo run -p kari-core --example one_engine`
//!
//! Needs the node binary: `cargo build -p kari-cli`.
//!
//! Nothing here touches your own kari. The node gets its own home directory and
//! its own database, and both are deleted when the example ends.
//!
//! What it proves is the arrangement a laptop needs. A daemon watches the host
//! while nobody has a window open. The app opens, claims the engine, and the
//! daemon stops rather than run a second engine beside it — two engines on one
//! host bind one port, share one node id and plan against one login. Then the
//! window closes, the claim goes, and the daemon takes the engine back.

use kari_core::owner;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

struct Proc(Child);

impl Drop for Proc {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("kari_core=info")
        .init();

    let node_bin = std::path::Path::new("target/debug/kari-node");
    if !node_bin.exists() {
        anyhow::bail!("build it first: cargo build -p kari-cli");
    }

    let port = kari_core::tunnel::free_port()?;
    let home = std::env::temp_dir().join(format!("kari-one-engine-{port}"));
    std::fs::create_dir_all(home.join(".claude/projects"))?;
    let kari_dir = home.join(".config/kari");
    std::fs::create_dir_all(&kari_dir)?;

    // Read and write the same claim the node reads and writes.
    kari_core::paths::set_kari_dir(&kari_dir);

    let start_node = || -> anyhow::Result<Proc> {
        Ok(Proc(
            Command::new(node_bin)
                .args([
                    "serve",
                    "--listen",
                    &format!("127.0.0.1:{port}"),
                    "--name",
                    "example-node",
                    "--summaries",
                    "false",
                ])
                .env("HOME", &home)
                .env("XDG_CONFIG_HOME", home.join(".config"))
                .env("CLAUDE_CONFIG_DIR", home.join(".claude"))
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()?,
        ))
    };

    let wait_for_claim = |what: &str| -> anyhow::Result<owner::Holder> {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if let Some(h) = owner::current() {
                if h.what == what {
                    return Ok(h);
                }
            }
            if Instant::now() > deadline {
                anyhow::bail!("no claim by {what} after 30 s");
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    };

    // ---- the daemon takes the engine of an idle machine -------------------

    let node = start_node()?;
    let held = wait_for_claim(owner::DAEMON)?;
    println!(
        "daemon holds the engine: pid {}, port {}",
        held.pid, held.port
    );
    if !owner::wait_for_port(port, Duration::from_millis(2000)) {
        println!("and it serves the API on {port}");
    } else {
        anyhow::bail!("the daemon claimed the engine but binds no port");
    }

    // ---- a window opens ---------------------------------------------------
    //
    // This is what the app does on start: claim the engine, then wait for the
    // daemon's listener to close before binding the same port.

    let took_at = Instant::now();
    let owned = owner::take_when_free(owner::APP, port, Duration::from_secs(12))?;
    println!("\nthe window claimed the engine in {:?}", took_at.elapsed());
    // Not after the timeout. A daemon's claim is taken at once, because the
    // daemon is waiting for this write to know it must step down.
    if took_at.elapsed() > Duration::from_secs(2) {
        anyhow::bail!("the window waited for a daemon that was waiting for the window");
    }

    // The daemon notices within its check interval and exits. It must exit:
    // a daemon that kept its engine would leave two on this host.
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if owner::wait_for_port(port, Duration::from_millis(500)) {
            println!("the daemon stepped down and closed the port");
            break;
        }
        if Instant::now() > deadline {
            anyhow::bail!("the daemon never stepped down");
        }
    }
    if owner::current().is_some() {
        anyhow::bail!("something else still claims the engine while the window holds it");
    }

    // A supervisor starts the node again while the window is open. It must come
    // back waiting, not serving: this is the restart loop that KeepAlive and
    // Restart=always create, and the claim is what makes it harmless.
    let restarted = start_node()?;
    std::thread::sleep(Duration::from_secs(6));
    if owner::current().is_some() {
        anyhow::bail!("a restarted daemon took the engine from the window");
    }
    if !owner::wait_for_port(port, Duration::from_millis(500)) {
        anyhow::bail!("a restarted daemon bound the port while the window held the engine");
    }
    println!("a restarted daemon waits instead of taking over");

    // ---- the window closes ------------------------------------------------

    drop(owned);
    println!("\nthe window released the engine");
    let back = wait_for_claim(owner::DAEMON)?;
    println!("the daemon took it back: pid {}", back.pid);
    if owner::wait_for_port(port, Duration::from_secs(10)) {
        anyhow::bail!("the daemon holds the engine but serves nothing");
    }
    println!("and it serves the API again");

    drop(restarted);
    drop(node);
    let _ = std::fs::remove_dir_all(&home);
    println!("\ndone");
    Ok(())
}
