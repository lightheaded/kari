//! Install, show, and remove kari hooks. Point CLAUDE_CONFIG_DIR at a scratch dir to test.
use kari_core::hooks;
fn main() -> anyhow::Result<()> {
    let arg = std::env::args().nth(1).unwrap_or_else(|| "cycle".into());
    let path = kari_core::paths::claude_dir().join("settings.json");
    if arg == "install" || arg == "cycle" {
        let p = hooks::install(47311)?;
        println!("installed relay {}", p.display());
        println!("installed(): {}", hooks::installed());
        println!("{}", std::fs::read_to_string(&path)?);
    }
    if arg == "uninstall" || arg == "cycle" {
        hooks::uninstall()?;
        println!("installed(): {}", hooks::installed());
        println!("{}", std::fs::read_to_string(&path)?);
    }
    Ok(())
}
