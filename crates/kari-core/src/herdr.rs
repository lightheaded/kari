//! herdr client. Reads over the newline-JSON socket, acts through the `herdr` CLI.

use crate::model::HerdrAgent;
#[cfg(unix)]
use crate::paths;
use serde_json::{json, Value};
#[cfg(unix)]
use std::io::{BufRead, BufReader, Write};
#[cfg(unix)]
use std::os::unix::net::UnixStream;
use std::time::Duration;

#[cfg(unix)]
pub fn available() -> bool {
    paths::herdr_socket().exists()
}

/// herdr is a terminal multiplexer for Unix and speaks over a Unix socket, so
/// a Windows node never has one. Every entry point below routes through
/// `call`, so this one stub keeps the rest of the module honest.
#[cfg(not(unix))]
pub fn available() -> bool {
    false
}

#[cfg(not(unix))]
fn call(_method: &str, _params: Value) -> anyhow::Result<Value> {
    anyhow::bail!("herdr does not run on this platform")
}

#[cfg(unix)]
fn call(method: &str, params: Value) -> anyhow::Result<Value> {
    let sock = paths::herdr_socket();
    let mut stream = UnixStream::connect(&sock)?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    let req = json!({"id": format!("kari:{method}"), "method": method, "params": params});
    stream.write_all(serde_json::to_string(&req)?.as_bytes())?;
    stream.write_all(b"\n")?;
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let v: Value = serde_json::from_str(&line)?;
    if let Some(err) = v.get("error") {
        anyhow::bail!("herdr {method}: {err}");
    }
    Ok(v.get("result").cloned().unwrap_or(Value::Null))
}

pub fn agents() -> anyhow::Result<Vec<HerdrAgent>> {
    let ws = call("workspace.list", json!({})).ok();
    let labels: std::collections::HashMap<String, String> = ws
        .as_ref()
        .and_then(|w| w.get("workspaces"))
        .and_then(|w| w.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|w| {
                    Some((
                        w.get("workspace_id")?.as_str()?.to_string(),
                        w.get("label")?.as_str()?.to_string(),
                    ))
                })
                .collect()
        })
        .unwrap_or_default();

    let res = call("agent.list", json!({}))?;
    let Some(arr) = res.get("agents").and_then(|a| a.as_array()) else {
        return Ok(vec![]);
    };
    let mut out = vec![];
    for a in arr {
        let s = |k: &str| a.get(k).and_then(|x| x.as_str()).map(|x| x.to_string());
        let Some(pane_id) = s("pane_id") else {
            continue;
        };
        let workspace_id = s("workspace_id");
        out.push(HerdrAgent {
            workspace_label: workspace_id.as_ref().and_then(|w| labels.get(w).cloned()),
            pane_id,
            tab_id: s("tab_id"),
            workspace_id,
            cwd: s("cwd"),
            agent: s("agent"),
            agent_status: s("agent_status"),
            title: s("terminal_title_stripped").or_else(|| s("terminal_title")),
            focused: a.get("focused").and_then(|f| f.as_bool()).unwrap_or(false),
            session_id: a
                .get("session")
                .and_then(|s| s.get("value"))
                .and_then(|v| v.as_str())
                .map(|v| v.to_string()),
        });
    }
    Ok(out)
}

/// Focus the workspace and the pane that hosts an agent.
pub fn focus(agent: &HerdrAgent) -> anyhow::Result<()> {
    if let Some(ws) = &agent.workspace_id {
        let _ = call("workspace.focus", json!({"workspace_id": ws}));
    }
    // agent.focus marks the tab as seen and raises the pane.
    let _ = call("agent.focus", json!({"pane_id": agent.pane_id}))
        .or_else(|_| call("pane.focus", json!({"pane_id": agent.pane_id})));
    Ok(())
}

/// A pane herdr opened for kari.
#[derive(Debug, Clone)]
pub struct OpenedPane {
    pub pane_id: String,
    pub tab_id: String,
}

/// Find a string field anywhere in the first two levels of a result.
fn dig<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    if let Some(s) = v.get(key).and_then(|x| x.as_str()) {
        return Some(s);
    }
    if let Some(map) = v.as_object() {
        for (_k, child) in map {
            if let Some(s) = child.get(key).and_then(|x| x.as_str()) {
                return Some(s);
            }
        }
    }
    None
}

/// The array under `key`, or an empty one.
fn arr<'a>(v: &'a Value, key: &str) -> &'a [Value] {
    v.get(key)
        .and_then(|x| x.as_array())
        .map(|a| a.as_slice())
        .unwrap_or(&[])
}

/// Two paths that name one directory. A trailing slash and a symlink both hide
/// a match that the user can see on the screen.
fn same_dir(a: &str, b: &str) -> bool {
    if a.trim_end_matches('/') == b.trim_end_matches('/') {
        return true;
    }
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => false,
    }
}

/// Choose the workspace that already works in `cwd`.
///
/// A herdr workspace holds no directory of its own, so its panes stand for it:
/// a workspace matches when one of its panes sits in `cwd`. When more than one
/// matches, the workspace with the most recent agent state change wins, and
/// the position on the workspace bar breaks a tie. This function takes the
/// three lists as they arrive, so the choice can be tested with no herdr.
fn pick_workspace(cwd: &str, panes: &Value, agents: &Value, workspaces: &Value) -> Option<String> {
    let mut matched: Vec<&str> = vec![];
    for p in arr(panes, "panes") {
        let Some(ws) = p.get("workspace_id").and_then(|w| w.as_str()) else {
            continue;
        };
        let dir = p
            .get("cwd")
            .or_else(|| p.get("foreground_cwd"))
            .and_then(|c| c.as_str());
        if dir.is_some_and(|d| same_dir(d, cwd)) && !matched.contains(&ws) {
            matched.push(ws);
        }
    }
    // A workspace with no agent in it scores zero, and its place on the bar
    // then decides. herdr appends a new workspace, so the last one is the
    // newest until somebody moves it.
    let seq = |ws: &str| {
        arr(agents, "agents")
            .iter()
            .filter(|a| a.get("workspace_id").and_then(|w| w.as_str()) == Some(ws))
            .filter_map(|a| a.get("state_change_seq").and_then(|s| s.as_u64()))
            .max()
            .unwrap_or(0)
    };
    let number = |ws: &str| {
        arr(workspaces, "workspaces")
            .iter()
            .find(|w| w.get("workspace_id").and_then(|x| x.as_str()) == Some(ws))
            .and_then(|w| w.get("number"))
            .and_then(|n| n.as_u64())
            .unwrap_or(0)
    };
    matched
        .into_iter()
        .max_by_key(|ws| (seq(ws), number(ws)))
        .map(|ws| ws.to_string())
}

/// The workspace to open a tab in, or None when no workspace works in `cwd`.
fn workspace_for_cwd(cwd: &str) -> Option<String> {
    let panes = call("pane.list", json!({})).ok()?;
    let agents = call("agent.list", json!({})).unwrap_or(Value::Null);
    let workspaces = call("workspace.list", json!({})).unwrap_or(Value::Null);
    pick_workspace(cwd, &panes, &agents, &workspaces)
}

/// Open a herdr tab in `cwd` and start an agent in its pane.
/// `args` go to the agent command, for example `["--resume", "<id>"]`.
pub fn open_agent(
    cwd: &str,
    label: &str,
    kind: &str,
    args: &[String],
    focus: bool,
) -> anyhow::Result<OpenedPane> {
    if !available() {
        anyhow::bail!("herdr is not running");
    }
    // A tab.create with no workspace lands in the focused workspace, which is
    // rarely the one that works in `cwd`. So reuse the workspace that already
    // sits there, and give a directory that has none a workspace of its own,
    // named after the directory.
    let created = match workspace_for_cwd(cwd) {
        Some(ws) => call(
            "tab.create",
            json!({ "cwd": cwd, "label": label, "focus": focus, "workspace_id": ws }),
        )?,
        None => {
            let made = call(
                "workspace.create",
                json!({
                    "cwd": cwd,
                    "label": crate::paths::project_display_name(cwd),
                    "focus": focus,
                }),
            )?;
            // herdr names the first tab of a new workspace after its number.
            // The card title belongs there, as on every other tab kari opens.
            if let Some(tab) = dig(&made, "tab_id") {
                let _ = call("tab.rename", json!({ "tab_id": tab, "label": label }));
            }
            made
        }
    };
    let tab_id = dig(&created, "tab_id")
        .ok_or_else(|| anyhow::anyhow!("herdr opened a tab and returned no tab_id"))?
        .to_string();
    // The pane of a fresh tab is the one that carries its tab id.
    let panes = call("pane.list", json!({}))?;
    let pane_id = panes
        .get("panes")
        .and_then(|p| p.as_array())
        .and_then(|arr| {
            arr.iter()
                .find(|p| p.get("tab_id").and_then(|t| t.as_str()) == Some(tab_id.as_str()))
                .and_then(|p| p.get("pane_id"))
                .and_then(|p| p.as_str())
        })
        .ok_or_else(|| anyhow::anyhow!("herdr has no pane for tab {tab_id}"))?
        .to_string();
    // herdr accepts a slug as the agent name, not free text.
    let name = crate::launcher::slugify(label);
    let mut params = json!({ "pane_id": pane_id, "kind": kind, "name": name });
    if !args.is_empty() {
        params["args"] = json!(args);
    }
    // A fresh pane needs a moment before its shell accepts an agent.
    let mut last: Option<anyhow::Error> = None;
    for _ in 0..20 {
        match call("agent.start", params.clone()) {
            Ok(_) => return Ok(OpenedPane { pane_id, tab_id }),
            Err(e) => {
                let busy = e.to_string().contains("agent_pane_busy");
                last = Some(e);
                if !busy {
                    break;
                }
                std::thread::sleep(Duration::from_millis(300));
            }
        }
    }
    // Leave no empty tab behind when the agent cannot start.
    let _ = close_tab(&tab_id);
    Err(last.unwrap_or_else(|| anyhow::anyhow!("herdr agent.start did not answer")))
}

/// Close a tab kari opened. Used when a launch fails halfway.
pub fn close_tab(tab_id: &str) -> anyhow::Result<()> {
    call("tab.close", json!({ "tab_id": tab_id }))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn panes(rows: &[(&str, &str)]) -> Value {
        json!({"panes": rows.iter().map(|(ws, cwd)| json!({
            "workspace_id": ws, "cwd": cwd, "tab_id": format!("{ws}:t1"),
        })).collect::<Vec<_>>()})
    }

    fn agents(rows: &[(&str, u64)]) -> Value {
        json!({"agents": rows.iter().map(|(ws, seq)| json!({
            "workspace_id": ws, "state_change_seq": seq,
        })).collect::<Vec<_>>()})
    }

    fn workspaces(ids: &[&str]) -> Value {
        json!({"workspaces": ids.iter().enumerate().map(|(i, ws)| json!({
            "workspace_id": ws, "number": i + 1,
        })).collect::<Vec<_>>()})
    }

    /// The bug this guards: a jump in opened its tab in whichever workspace
    /// was focused, so the card landed next to unrelated work.
    #[test]
    fn takes_the_workspace_that_works_in_the_directory() {
        let p = panes(&[("w1", "/p/one"), ("w2", "/p/two")]);
        let a = agents(&[("w1", 10), ("w2", 20)]);
        let w = workspaces(&["w1", "w2"]);
        assert_eq!(pick_workspace("/p/two", &p, &a, &w).as_deref(), Some("w2"));
        assert_eq!(pick_workspace("/p/one/", &p, &a, &w).as_deref(), Some("w1"));
    }

    #[test]
    fn a_new_directory_has_no_workspace() {
        let p = panes(&[("w1", "/p/one")]);
        let a = agents(&[("w1", 10)]);
        let w = workspaces(&["w1"]);
        assert_eq!(pick_workspace("/p/three", &p, &a, &w), None);
    }

    #[test]
    fn the_most_recent_agent_wins_over_the_older_one() {
        let p = panes(&[("w1", "/p/one"), ("w2", "/p/one"), ("w3", "/p/one")]);
        let a = agents(&[("w1", 90), ("w2", 300), ("w3", 120)]);
        let w = workspaces(&["w1", "w2", "w3"]);
        assert_eq!(pick_workspace("/p/one", &p, &a, &w).as_deref(), Some("w2"));
    }

    /// With no agent to date a workspace, the newest workspace is the last one
    /// on the bar.
    #[test]
    fn the_workspace_bar_breaks_a_tie() {
        let p = panes(&[("w1", "/p/one"), ("w2", "/p/one")]);
        let w = workspaces(&["w1", "w2"]);
        let picked = pick_workspace("/p/one", &p, &Value::Null, &w);
        assert_eq!(picked.as_deref(), Some("w2"));
    }

    #[test]
    fn an_empty_herdr_picks_nothing() {
        assert_eq!(
            pick_workspace("/p/one", &Value::Null, &Value::Null, &Value::Null),
            None
        );
    }
}
