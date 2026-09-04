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
    let created = call(
        "tab.create",
        json!({ "cwd": cwd, "label": label, "focus": focus }),
    )?;
    let tab_id = dig(&created, "tab_id")
        .ok_or_else(|| anyhow::anyhow!("herdr tab.create returned no tab_id"))?
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
