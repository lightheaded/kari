//! Install, show, and remove kari hooks. Point CLAUDE_CONFIG_DIR at a scratch dir to test.
//!
//! `repair` shows what a kari upgrade does: it strips the held event from an
//! install, then lets kari put the entries back the way a start does.
use kari_core::hooks;
fn main() -> anyhow::Result<()> {
    let arg = std::env::args().nth(1).unwrap_or_else(|| "cycle".into());
    // This example writes settings.json and removes the relay script, so it
    // must never run against the real install.
    if std::env::var_os("CLAUDE_CONFIG_DIR").is_none() {
        anyhow::bail!("set CLAUDE_CONFIG_DIR to a scratch directory first");
    }
    let path = kari_core::paths::claude_dir().join("settings.json");
    if arg == "install" || arg == "cycle" {
        let cmd = hooks::install(47311)?;
        println!("installed relay {cmd}");
        println!("installed(): {}", hooks::installed());
        println!("entries_current(): {}", hooks::entries_current(47311));
        println!("{}", std::fs::read_to_string(&path)?);
    }
    if arg == "repair" {
        // Make the file look like an install from an older kari.
        let mut v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path)?)?;
        if let Some(h) = v.get_mut("hooks").and_then(|h| h.as_object_mut()) {
            h.remove(hooks::HELD_EVENT);
        }
        std::fs::write(&path, serde_json::to_string_pretty(&v)? + "\n")?;
        println!(
            "entries_current() after the downgrade: {}",
            hooks::entries_current(47311)
        );
        println!("repair() wrote settings: {}", hooks::repair(47311)?);
        println!("entries_current(): {}", hooks::entries_current(47311));
        println!("repair() again: {}", hooks::repair(47311)?);
    }
    if arg == "uninstall" || arg == "cycle" {
        hooks::uninstall()?;
        println!("installed(): {}", hooks::installed());
        println!("repair() with no install: {}", hooks::repair(47311)?);
        println!("{}", std::fs::read_to_string(&path)?);
    }
    Ok(())
}
