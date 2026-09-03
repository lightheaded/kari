//! Open a herdr tab, start Claude Code in it, then close the tab again.
//! `cargo run -p kari-core --example herdr_open -- [cwd] [--keep]`

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let keep = args.iter().any(|a| a == "--keep");
    let cwd = args
        .iter()
        .find(|a| !a.starts_with("--"))
        .cloned()
        .unwrap_or_else(|| std::env::temp_dir().to_string_lossy().into_owned());
    println!("herdr available: {}", kari_core::herdr::available());
    let opened = kari_core::herdr::open_agent(&cwd, "kari probe", "claude", &[], false)?;
    println!("opened pane {} in tab {}", opened.pane_id, opened.tab_id);
    std::thread::sleep(std::time::Duration::from_secs(4));
    let agents = kari_core::herdr::agents()?;
    if let Some(a) = agents.iter().find(|a| a.pane_id == opened.pane_id) {
        println!(
            "agent: {:?} status {:?} session {:?}",
            a.agent, a.agent_status, a.session_id
        );
    } else {
        println!("no agent reported for that pane yet");
    }
    if keep {
        println!("tab kept");
    } else {
        kari_core::herdr::close_tab(&opened.tab_id)?;
        println!("tab closed");
    }
    Ok(())
}
