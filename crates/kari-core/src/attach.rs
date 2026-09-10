//! Files attached to cards.
//!
//! A prompt sometimes needs a picture: a screenshot of the bug, a mock-up of
//! the screen. Claude Code takes a file only when the prompt names a path, and
//! the file is on the host that runs the session. So an attachment lives on
//! the node that owns the card, beside the store, and never on a server.
//!
//! A server, when one is configured, carries the bytes down the link to that
//! node and keeps no copy. That follows rule 2 in `AGENTS.md`: a server adds a
//! path, it does not become the party that holds the state. It also keeps
//! attachments working on the default setup, which has no server at all.
//!
//! The directory is the store. There is no table beside it, because a row and
//! a file can disagree and then a card offers a file that a run cannot read.

use crate::model::{Attachment, Card, MAX_ATTACHMENT_BYTES};
use crate::paths;
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine as _;
use chrono::{DateTime, Duration, Utc};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use tracing::warn;

/// How long the files of a card that no longer exists are kept.
///
/// A delete is undoable from the toast, and `restore_card` puts the card back
/// with its id. If the files went with the card, the undo would give back a
/// card whose attachments are gone. So an orphan directory waits a day.
const ORPHAN_GRACE_HOURS: i64 = 24;

/// The content type of a file, from its extension. The list holds what a
/// person attaches to a prompt; everything else is bytes.
pub fn mime_for(name: &str) -> String {
    let ext = Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    let m = match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        "heic" => "image/heic",
        "pdf" => "application/pdf",
        "json" => "application/json",
        "txt" | "log" => "text/plain",
        "md" => "text/markdown",
        "csv" => "text/csv",
        "html" | "htm" => "text/html",
        _ => "application/octet-stream",
    };
    m.to_string()
}

fn describe(entry: &Path) -> Option<Attachment> {
    let name = entry.file_name()?.to_str()?.to_string();
    let meta = std::fs::metadata(entry).ok()?;
    if !meta.is_file() {
        return None;
    }
    let at: DateTime<Utc> = meta
        .modified()
        .ok()
        .map(DateTime::from)
        .unwrap_or_else(Utc::now);
    Some(Attachment {
        mime: mime_for(&name),
        bytes: meta.len(),
        path: entry.to_string_lossy().into_owned(),
        name,
        at,
    })
}

/// The files of one card, oldest first.
pub fn list(card_id: &str) -> Vec<Attachment> {
    let dir = paths::card_attachments_dir(card_id);
    let Ok(rd) = std::fs::read_dir(&dir) else {
        return vec![];
    };
    let mut out: Vec<Attachment> = rd.flatten().filter_map(|e| describe(&e.path())).collect();
    out.sort_by(|a, b| a.at.cmp(&b.at).then_with(|| a.name.cmp(&b.name)));
    out
}

/// The card ids that hold at least one file. One `read_dir` of the root, so a
/// board of hundreds of cards costs one call and not one per card.
pub fn cards_with_files() -> HashSet<String> {
    let Ok(rd) = std::fs::read_dir(paths::attachments_dir()) else {
        return HashSet::new();
    };
    rd.flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| e.file_name().to_str().map(str::to_string))
        .collect()
}

/// A name no file in the directory holds yet. `shot.png` becomes `shot-2.png`.
fn unique_name(dir: &Path, name: &str) -> String {
    let safe = paths::safe_file_name(name);
    if !dir.join(&safe).exists() {
        return safe;
    }
    let p = Path::new(&safe);
    let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("file");
    let ext = p.extension().and_then(|s| s.to_str());
    for n in 2..1000 {
        let candidate = match ext {
            Some(e) => format!("{stem}-{n}.{e}"),
            None => format!("{stem}-{n}"),
        };
        if !dir.join(&candidate).exists() {
            return candidate;
        }
    }
    format!("{stem}-{}", Utc::now().timestamp_millis())
}

/// Write one file for a card. The name that comes back can differ from the one
/// that went in: it is made safe, and a collision gets a number.
pub fn add(card_id: &str, name: &str, data: &[u8]) -> anyhow::Result<Attachment> {
    if data.is_empty() {
        anyhow::bail!("the file is empty");
    }
    if data.len() as u64 > MAX_ATTACHMENT_BYTES {
        anyhow::bail!(
            "the file is {} and the limit is {}",
            human_bytes(data.len() as u64),
            human_bytes(MAX_ATTACHMENT_BYTES)
        );
    }
    let dir = paths::card_attachments_dir(card_id);
    std::fs::create_dir_all(&dir)?;
    let name = unique_name(&dir, name);
    let path = dir.join(&name);
    std::fs::write(&path, data)?;
    describe(&path).ok_or_else(|| anyhow::anyhow!("the file was written but cannot be read"))
}

/// The bytes of one file, with its content type.
pub fn read(card_id: &str, name: &str) -> anyhow::Result<(String, Vec<u8>)> {
    let name = paths::safe_file_name(name);
    let path = paths::card_attachments_dir(card_id).join(&name);
    let data = std::fs::read(&path)
        .map_err(|e| anyhow::anyhow!("this card has no attachment called {name}: {e}"))?;
    Ok((mime_for(&name), data))
}

/// Take one file off a card.
pub fn remove(card_id: &str, name: &str) -> anyhow::Result<()> {
    let name = paths::safe_file_name(name);
    let dir = paths::card_attachments_dir(card_id);
    std::fs::remove_file(dir.join(&name))
        .map_err(|e| anyhow::anyhow!("this card has no attachment called {name}: {e}"))?;
    // An empty directory is an orphan the sweep would have to think about.
    let _ = std::fs::remove_dir(&dir);
    Ok(())
}

/// Take every file off a card.
pub fn remove_all(card_id: &str) {
    let dir = paths::card_attachments_dir(card_id);
    if dir.exists() {
        if let Err(e) = std::fs::remove_dir_all(&dir) {
            warn!("attachments of {card_id} not removed: {e}");
        }
    }
}

/// Remove the files kari no longer needs. Returns how many cards were cleared.
///
/// Three rules, in this order:
///
/// 1. An archived card loses its files at once. An archive takes the card off
///    the board and nothing runs it again.
/// 2. A card marked done loses its files once `keep_days` have passed. Not at
///    the moment it is done: a done card can come back, and a file that is
///    gone cannot.
/// 3. A directory that belongs to no card at all waits a day, because a delete
///    is undoable from its toast.
pub fn sweep(cards: &[Card], keep_days: i64, now: DateTime<Utc>) -> usize {
    let mut cleared = 0;
    for id in cards_with_files() {
        let Some(card) = cards.iter().find(|c| c.id == id) else {
            if orphan_is_old(&id, now) {
                remove_all(&id);
                cleared += 1;
            }
            continue;
        };
        let expired = card.archived
            || card
                .done_at
                .is_some_and(|d| now - d >= Duration::days(keep_days.max(0)));
        if expired {
            remove_all(&card.id);
            cleared += 1;
        }
    }
    cleared
}

fn orphan_is_old(card_id: &str, now: DateTime<Utc>) -> bool {
    let dir: PathBuf = paths::card_attachments_dir(card_id);
    let Ok(meta) = std::fs::metadata(&dir) else {
        return false;
    };
    let Ok(modified) = meta.modified() else {
        return false;
    };
    now - DateTime::<Utc>::from(modified) >= Duration::hours(ORPHAN_GRACE_HOURS)
}

/// Bytes as base64. An attachment travels this way over HTTP and over the
/// link, both of which carry JSON.
pub fn b64_encode(data: &[u8]) -> String {
    B64.encode(data)
}

/// The reverse. A browser hands a file over as a data URL, so the prefix that
/// a data URL carries is taken off rather than refused.
pub fn b64_decode(text: &str) -> anyhow::Result<Vec<u8>> {
    let body = match text.find(";base64,") {
        Some(i) if text.starts_with("data:") => &text[i + ";base64,".len()..],
        _ => text,
    };
    Ok(B64.decode(body.trim())?)
}

/// A size a person reads: `812 kB`, `1.4 MB`.
pub fn human_bytes(n: u64) -> String {
    if n < 1024 {
        return format!("{n} B");
    }
    let kb = n as f64 / 1024.0;
    if kb < 1024.0 {
        return format!("{kb:.0} kB");
    }
    format!("{:.1} MB", kb / 1024.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{CardKind, MAX_ATTACHMENT_BYTES};

    /// The kari directory is one static for the whole process, and `sweep`
    /// reads every directory under it. So these tests take a lock and start
    /// from an empty attachments directory, one at a time.
    static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn setup() -> std::sync::MutexGuard<'static, ()> {
        let held = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let root = std::env::temp_dir().join(format!("kari-attach-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        paths::set_kari_dir(&root);
        let _ = std::fs::remove_dir_all(paths::attachments_dir());
        held
    }

    fn card(id: &str) -> Card {
        Card {
            id: id.into(),
            kind: CardKind::Task,
            title: Some("t".into()),
            session_id: None,
            project_cwd: None,
            priority: 0,
            auto_run: false,
            run_prompt: None,
            permission_mode: None,
            model: None,
            estimate_weighted_tokens: None,
            manual_column: None,
            manual_lock_priority: None,
            tags: vec![],
            notes: None,
            archived: false,
            bg_job_id: None,
            last_job_state: None,
            last_job_at: None,
            scheduled: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            done_at: None,
        }
    }

    /// A name from a client must not walk out of the card's directory.
    #[test]
    fn a_name_cannot_escape_the_directory() {
        for bad in [
            "../../etc/passwd",
            "/tmp/x.png",
            "..",
            "",
            "a\\b.png",
            ".",
            "Screen shot.png",
            "a?b#c.png",
        ] {
            let safe = paths::safe_file_name(bad);
            assert!(!safe.contains('/'), "{bad} -> {safe}");
            assert!(!safe.contains('\\'), "{bad} -> {safe}");
            assert!(!safe.starts_with('.'), "{bad} -> {safe}");
            assert!(!safe.is_empty(), "{bad}");
            // The name is the last segment of a URL that travels raw inside a
            // link frame, and one line of the run prompt. Both need a name
            // that carries no character an encoder would have to touch.
            assert!(
                safe.chars()
                    .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c)),
                "{bad} -> {safe}"
            );
        }
        assert_eq!(paths::safe_file_name("Screen shot.png"), "Screen_shot.png");
        assert_eq!(paths::safe_file_name(".."), "file");
        assert_eq!(paths::safe_file_name(""), "file");
        assert_eq!(paths::safe_file_name("shot.png"), "shot.png");
    }

    #[test]
    fn base64_survives_a_data_url_prefix() {
        let raw = b"hello";
        let plain = b64_encode(raw);
        assert_eq!(b64_decode(&plain).unwrap(), raw);
        let url = format!("data:image/png;base64,{plain}");
        assert_eq!(b64_decode(&url).unwrap(), raw);
        assert!(b64_decode("not base64 !!!").is_err());
    }

    #[test]
    fn mime_comes_from_the_extension() {
        assert_eq!(mime_for("a.PNG"), "image/png");
        assert_eq!(mime_for("a.jpeg"), "image/jpeg");
        assert_eq!(mime_for("a"), "application/octet-stream");
    }

    #[test]
    fn a_second_file_of_one_name_gets_a_number() {
        let _held = setup();
        let first = add("card", "shot.png", b"one").unwrap();
        let second = add("card", "shot.png", b"two").unwrap();
        assert_eq!(first.name, "shot.png");
        assert_eq!(second.name, "shot-2.png");
        assert_eq!(list("card").len(), 2);
        assert_eq!(read("card", "shot-2.png").unwrap().1, b"two");
        remove("card", "shot.png").unwrap();
        assert_eq!(list("card").len(), 1);
        remove_all("card");
        assert!(list("card").is_empty());
    }

    #[test]
    fn a_file_over_the_limit_is_refused() {
        let _held = setup();
        let big = vec![0u8; MAX_ATTACHMENT_BYTES as usize + 1];
        let err = add("card", "big.bin", &big).unwrap_err().to_string();
        assert!(err.contains("limit"), "{err}");
        assert!(add("card", "empty.bin", b"").is_err());
    }

    /// An archived card loses its files now. A done card keeps them until the
    /// retention passes. A live card keeps them.
    #[test]
    fn the_sweep_clears_what_is_finished() {
        let _held = setup();
        let now = Utc::now();
        for id in ["live", "archived", "done-old", "done-new"] {
            add(id, "a.png", b"x").unwrap();
        }
        let mut archived = card("archived");
        archived.archived = true;
        let mut done_old = card("done-old");
        done_old.done_at = Some(now - Duration::days(30));
        let mut done_new = card("done-new");
        done_new.done_at = Some(now - Duration::hours(1));
        let cards = vec![card("live"), archived, done_old, done_new];

        sweep(&cards, 7, now);
        assert_eq!(list("live").len(), 1, "a live card keeps its files");
        assert!(list("archived").is_empty(), "an archive clears now");
        assert!(list("done-old").is_empty(), "retention passed");
        assert_eq!(list("done-new").len(), 1, "still inside retention");
    }

    /// A directory whose card is gone waits a day, so the undo of a delete can
    /// still give the card its files back.
    #[test]
    fn an_orphan_waits_a_day() {
        let _held = setup();
        add("gone", "a.png", b"x").unwrap();
        sweep(&[], 7, Utc::now());
        assert_eq!(list("gone").len(), 1, "too young to sweep");
        sweep(&[], 7, Utc::now() + Duration::hours(ORPHAN_GRACE_HOURS + 1));
        assert!(list("gone").is_empty(), "old enough to sweep");
    }

    /// The prompt names every attached file, because that is the only way a
    /// run reads one.
    #[test]
    fn the_prompt_names_the_files() {
        let a = Attachment {
            name: "shot.png".into(),
            bytes: 3,
            mime: "image/png".into(),
            path: "/x/shot.png".into(),
            at: Utc::now(),
        };
        let out = crate::model::with_attachments("fix this", std::slice::from_ref(&a));
        assert!(out.starts_with("fix this"), "{out}");
        assert!(out.contains("/x/shot.png"), "{out}");
        assert_eq!(crate::model::with_attachments("fix this", &[]), "fix this");
    }
}
