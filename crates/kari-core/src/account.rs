//! Which Claude Code account a node is logged in to.
//!
//! Quota belongs to the account, not to the machine. Two nodes signed in to the
//! same login share one 5-hour window and one 7-day window, so showing a meter
//! per node reads as two budgets where there is one, and "fill the quota" on
//! one node spends the other's. The board groups by the id this module reads.
//!
//! Claude Code writes the account into `.claude.json` on login. kari only reads
//! it. Nothing here fails loudly: a node whose account cannot be read is shown
//! on its own, which is what kari did before it knew about accounts at all.

use crate::paths;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Mutex;

/// The account a node's Claude Code is signed in to.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct AccountIdentity {
    /// Stable id of the account, and the key the board groups on.
    pub id: String,
    /// Login address. Shown when the account has no alias and no display name.
    pub email: Option<String>,
    /// The name on the account. The friendliest label kari has before the user
    /// gives the account one of their own.
    pub display_name: Option<String>,
    /// The organization the login belongs to. Two logins in one organization
    /// still hold separate quota, so this labels rather than groups.
    pub organization_id: Option<String>,
}

impl AccountIdentity {
    /// The best label kari can offer without an alias from the user.
    pub fn label(&self) -> String {
        self.display_name
            .clone()
            .filter(|s| !s.trim().is_empty())
            .or_else(|| self.email.clone().filter(|s| !s.trim().is_empty()))
            .unwrap_or_else(|| self.id.clone())
    }
}

/// Where Claude Code keeps the account. With `CLAUDE_CONFIG_DIR` set it lives
/// in that directory; otherwise beside the home directory, not inside
/// `~/.claude`. Both are tried, so a host that moved either way still answers.
fn config_files() -> Vec<PathBuf> {
    let mut v = vec![paths::claude_dir().join(".claude.json")];
    let home = paths::home().join(".claude.json");
    if !v.contains(&home) {
        v.push(home);
    }
    v
}

/// Pull the account out of a parsed `.claude.json`. Separate from the file read
/// so the shape can be tested without one.
pub fn from_config(v: &Value) -> Option<AccountIdentity> {
    let a = v.get("oauthAccount")?;
    let s = |k: &str| {
        a.get(k)
            .and_then(|x| x.as_str())
            .map(str::trim)
            .filter(|x| !x.is_empty())
            .map(str::to_string)
    };
    // Without an id there is nothing to group on, and a group keyed on an empty
    // string would merge every unknown node into one account.
    let id = s("accountUuid")?;
    Some(AccountIdentity {
        id,
        email: s("emailAddress"),
        display_name: s("displayName"),
        organization_id: s("organizationUuid"),
    })
}

/// The last account read, with the file and mtime it came from.
static CACHE: Mutex<Option<(PathBuf, std::time::SystemTime, Option<AccountIdentity>)>> =
    Mutex::new(None);

/// The account this host's Claude Code is signed in to, or None when it has
/// not logged in or the file cannot be read.
///
/// `.claude.json` is tens of kilobytes and the board is built often, so the
/// answer is cached until the file changes. A re-login rewrites it, which is
/// exactly when the answer must change.
pub fn read() -> Option<AccountIdentity> {
    for path in config_files() {
        let Ok(mtime) = std::fs::metadata(&path).and_then(|m| m.modified()) else {
            continue;
        };
        {
            let cache = CACHE.lock().unwrap();
            if let Some((p, t, v)) = cache.as_ref() {
                if *p == path && *t == mtime {
                    return v.clone();
                }
            }
        }
        let found = std::fs::read_to_string(&path)
            .ok()
            .and_then(|t| serde_json::from_str::<Value>(&t).ok())
            .as_ref()
            .and_then(from_config);
        *CACHE.lock().unwrap() = Some((path, mtime, found.clone()));
        return found;
    }
    None
}

// ------------------------------------------------------------------ grouping

/// The prefix of a kv key that holds one account's alias.
pub const ALIAS_KEY_PREFIX: &str = "account.alias.";

/// One node's contribution to the board's quota rows.
#[derive(Debug, Clone)]
pub struct NodeSample {
    pub node_id: String,
    pub node_name: String,
    pub account: Option<AccountIdentity>,
    pub quota: Option<crate::model::QuotaSample>,
    pub calibration: Option<crate::model::Calibration>,
}

/// The key a node's samples group under: its account, or the node itself when
/// the account is unknown. Never an empty string, which would merge every
/// unknown node into one budget.
pub fn group_key(node_id: &str, account: Option<&AccountIdentity>) -> String {
    match account.filter(|a| !a.id.is_empty()) {
        Some(a) => a.id.clone(),
        None => format!("node:{node_id}"),
    }
}

/// Fold one row per node into one row per account, in the order the nodes
/// arrived, so the local node stays first.
///
/// Within a group the freshest sample wins: every node on the login reports the
/// same two windows, and the newest reading is the one closest to the truth. A
/// node that has never reported one contributes its name and nothing else.
pub fn group(
    rows: &[NodeSample],
    aliases: &std::collections::HashMap<String, String>,
) -> Vec<crate::model::AccountQuota> {
    let mut out: Vec<crate::model::AccountQuota> = Vec::new();
    for r in rows {
        let key = group_key(&r.node_id, r.account.as_ref());
        let idx = match out.iter().position(|g| g.key == key) {
            Some(i) => i,
            None => {
                let alias = aliases
                    .get(&key)
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty());
                let label = alias
                    .clone()
                    .or_else(|| r.account.as_ref().map(|a| a.label()))
                    .unwrap_or_else(|| r.node_name.clone());
                out.push(crate::model::AccountQuota {
                    key: key.clone(),
                    label,
                    alias,
                    account: r.account.clone(),
                    node_ids: Vec::new(),
                    node_names: Vec::new(),
                    quota: None,
                    calibration: None,
                });
                out.len() - 1
            }
        };
        let g = &mut out[idx];
        g.node_ids.push(r.node_id.clone());
        g.node_names.push(r.node_name.clone());
        let newer = match (&g.quota, &r.quota) {
            (_, None) => false,
            (None, Some(_)) => true,
            (Some(have), Some(new)) => new.at > have.at,
        };
        if newer {
            g.quota = r.quota.clone();
            g.calibration = r.calibration.clone();
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(extra: &str) -> Value {
        serde_json::from_str(&format!(r#"{{"oauthAccount":{{{extra}}}}}"#)).unwrap()
    }

    #[test]
    fn reads_the_account_claude_code_writes() {
        let v = config(
            r#""accountUuid":"acc-1","emailAddress":"you@example.com","displayName":"You","organizationUuid":"org-1""#,
        );
        let a = from_config(&v).unwrap();
        assert_eq!(a.id, "acc-1");
        assert_eq!(a.email.as_deref(), Some("you@example.com"));
        assert_eq!(a.organization_id.as_deref(), Some("org-1"));
        assert_eq!(a.label(), "You");
    }

    #[test]
    fn falls_back_through_the_labels_it_has() {
        let mut a = AccountIdentity {
            id: "acc-1".into(),
            email: Some("you@example.com".into()),
            display_name: Some("You".into()),
            organization_id: None,
        };
        assert_eq!(a.label(), "You");
        a.display_name = None;
        assert_eq!(a.label(), "you@example.com");
        a.email = None;
        assert_eq!(a.label(), "acc-1");
    }

    #[test]
    fn an_account_without_an_id_is_no_account() {
        // Grouping on an empty id would merge every unknown node into one
        // account and show them a quota none of them owns.
        assert!(from_config(&config(r#""emailAddress":"you@example.com""#)).is_none());
        assert!(from_config(&config(r#""accountUuid":"  ""#)).is_none());
        assert!(from_config(&serde_json::json!({"other": 1})).is_none());
    }

    #[test]
    fn blank_fields_do_not_become_labels() {
        let a = from_config(&config(r#""accountUuid":"acc-1","displayName":"  ""#)).unwrap();
        assert_eq!(a.display_name, None);
        assert_eq!(a.label(), "acc-1");
    }

    // ------------------------------------------------------------- grouping

    use crate::model::{Calibration, QuotaSample, QuotaWindow};
    use chrono::{Duration, Utc};
    use std::collections::HashMap;

    fn account(id: &str) -> AccountIdentity {
        AccountIdentity {
            id: id.into(),
            email: Some(format!("{id}@example.com")),
            display_name: None,
            organization_id: None,
        }
    }

    fn sample(pct: f64, age_secs: i64) -> QuotaSample {
        QuotaSample {
            at: Utc::now() - Duration::seconds(age_secs),
            five_hour: Some(QuotaWindow {
                used_percentage: pct,
                resets_at: None,
            }),
            seven_day: None,
            source: "statusline".into(),
        }
    }

    fn node(id: &str, acc: Option<AccountIdentity>, quota: Option<QuotaSample>) -> NodeSample {
        NodeSample {
            node_id: id.into(),
            node_name: id.to_uppercase(),
            account: acc,
            quota,
            calibration: Some(Calibration::default()),
        }
    }

    #[test]
    fn two_nodes_on_one_login_share_one_row() {
        // The bug this guards: two machines on one subscription showed two
        // meters, which reads as two budgets where there is one.
        let rows = [
            node("local", Some(account("acc-1")), Some(sample(40.0, 10))),
            node("mgr", Some(account("acc-1")), Some(sample(41.0, 5))),
        ];
        let g = group(&rows, &HashMap::new());
        assert_eq!(g.len(), 1);
        assert_eq!(g[0].node_names, ["LOCAL", "MGR"]);
        // The freshest reading wins: both describe the same window.
        let w = g[0].quota.as_ref().unwrap().five_hour.as_ref().unwrap();
        assert_eq!(w.used_percentage, 41.0);
    }

    #[test]
    fn two_logins_keep_two_rows() {
        let rows = [
            node("local", Some(account("acc-1")), Some(sample(40.0, 5))),
            node("other", Some(account("acc-2")), Some(sample(7.0, 5))),
        ];
        let g = group(&rows, &HashMap::new());
        assert_eq!(g.len(), 2);
        assert_eq!(g[0].key, "acc-1");
        assert_eq!(g[1].key, "acc-2");
    }

    #[test]
    fn an_unknown_account_never_merges_with_another() {
        // A node running an older kari reports no account. Folding those
        // together would show one budget for two unrelated subscriptions.
        let rows = [
            node("old-one", None, Some(sample(40.0, 5))),
            node("old-two", None, Some(sample(7.0, 5))),
        ];
        let g = group(&rows, &HashMap::new());
        assert_eq!(g.len(), 2);
        assert_eq!(g[0].key, "node:old-one");
        // With no account to name it, the row falls back to the node's name.
        assert_eq!(g[0].label, "OLD-ONE");
    }

    #[test]
    fn an_alias_wins_over_every_other_label() {
        let mut aliases = HashMap::new();
        aliases.insert("acc-1".to_string(), "tom".to_string());
        let rows = [node("local", Some(account("acc-1")), None)];
        let g = group(&rows, &aliases);
        assert_eq!(g[0].label, "tom");
        assert_eq!(g[0].alias.as_deref(), Some("tom"));
        // Without one, the account's own label shows.
        let g = group(&rows, &HashMap::new());
        assert_eq!(g[0].label, "acc-1@example.com");
        assert_eq!(g[0].alias, None);
    }

    #[test]
    fn a_node_without_a_sample_still_joins_its_account() {
        // It spends the same budget, so it belongs in the row even with
        // nothing to report yet.
        let rows = [
            node("local", Some(account("acc-1")), Some(sample(40.0, 5))),
            node("fresh", Some(account("acc-1")), None),
        ];
        let g = group(&rows, &HashMap::new());
        assert_eq!(g.len(), 1);
        assert_eq!(g[0].node_names, ["LOCAL", "FRESH"]);
        assert!(g[0].quota.is_some());
    }

    #[test]
    fn a_blank_alias_does_not_hide_the_account() {
        let mut aliases = HashMap::new();
        aliases.insert("acc-1".to_string(), "   ".to_string());
        let rows = [node("local", Some(account("acc-1")), None)];
        let g = group(&rows, &aliases);
        assert_eq!(g[0].alias, None);
        assert_eq!(g[0].label, "acc-1@example.com");
    }
}
