//! Replacing this binary with the newest release.
//!
//! The desktop app carries the Tauri updater, which knows how to swap a bundle.
//! A node is one file, so it does the same job the plain way: ask GitHub for
//! the newest release, fetch the binary built for this host, check it against
//! the checksum published beside it, and rename it over the one that is running.
//!
//! The running process is not replaced. A Unix rename leaves this process on
//! the old inode, and Windows renames the file out from under an open handle,
//! so in both cases the new version starts on the next run. `serve` therefore
//! stops itself after an update and lets the service manager start it again.
//!
//! This is off unless asked for. A node is deployed by whatever manages its
//! host, and that usually pins a version on purpose; a binary that changed
//! itself under such a host would make the pin a lie. `kari-node update` is a
//! person asking, and `serve --auto-update` is an operator asking.

use anyhow::{anyhow, bail, Context, Result};
use std::io::Write;
use std::path::{Path, PathBuf};

/// Where releases come from. The same repository the app updates from.
const REPO: &str = "lightheaded/kari";

/// GitHub answers an API call with no user agent with 403.
const UA: &str = concat!("kari-node/", env!("CARGO_PKG_VERSION"));

/// The version this binary was built as.
pub fn current() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// The asset name suffix for this host, or `None` where no node is built.
///
/// The release workflow builds two. A Mac runs the desktop app, which updates
/// itself, and a node on a Mac is built by hand — so there is nothing to fetch
/// and this says so rather than guessing at a name that is not there.
pub fn target() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => Some("x86_64-unknown-linux-gnu"),
        ("windows", "x86_64") => Some("x86_64-pc-windows-msvc"),
        _ => None,
    }
}

/// The newest release, as GitHub reports it.
pub struct Release {
    /// The git tag, such as `v0.8.0`. The asset names carry it.
    pub tag: String,
    /// The tag without its `v`, which is what compares against `current()`.
    pub version: String,
}

/// What one update attempt did.
pub struct Outcome {
    pub from: String,
    pub to: String,
    /// False when this host already ran the newest release.
    pub replaced: bool,
}

fn client() -> Result<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .user_agent(UA)
        .timeout(std::time::Duration::from_secs(300))
        .build()
        .context("build the http client")
}

/// Ask GitHub for the newest published release.
///
/// `releases/latest` skips drafts and pre-releases, so a release still being
/// uploaded is never offered.
pub fn latest() -> Result<Release> {
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let r = client()?
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .send()
        .with_context(|| format!("ask {url}"))?;
    if !r.status().is_success() {
        bail!("{url} answered {}", r.status());
    }
    let body: serde_json::Value = r.json().context("read the release as JSON")?;
    let tag = body["tag_name"]
        .as_str()
        .ok_or_else(|| anyhow!("the newest release has no tag_name"))?
        .to_string();
    let version = tag.trim_start_matches('v').to_string();
    Ok(Release { tag, version })
}

/// Whether `candidate` is a release this node does not have.
///
/// A string compare would call 0.10.0 older than 0.9.0. Anything that does not
/// parse is treated as "not newer": a tag nobody can order is not a reason to
/// overwrite a working binary.
pub fn is_newer(candidate: &str, running: &str) -> bool {
    match (
        semver::Version::parse(candidate),
        semver::Version::parse(running),
    ) {
        (Ok(a), Ok(b)) => a > b,
        _ => false,
    }
}

/// Fetch the newest release and put it in place of this binary.
///
/// `Ok(Outcome { replaced: false, .. })` means there was nothing newer, which
/// is the ordinary answer and not a failure.
pub fn update() -> Result<Outcome> {
    let running = current().to_string();
    let release = latest()?;
    if !is_newer(&release.version, &running) {
        return Ok(Outcome {
            from: running,
            to: release.version,
            replaced: false,
        });
    }
    let target = target().ok_or_else(|| {
        anyhow!(
            "no node is built for {} {}; build it from source",
            std::env::consts::OS,
            std::env::consts::ARCH
        )
    })?;
    let exe = std::env::current_exe().context("find this binary")?;
    // A symlink points at the real file, and renaming over the link would
    // replace the link instead of the binary. `/nix/store` arrives this way.
    let exe = std::fs::canonicalize(&exe).unwrap_or(exe);
    let dir = exe
        .parent()
        .ok_or_else(|| anyhow!("{} has no directory", exe.display()))?
        .to_path_buf();

    let suffix = if cfg!(windows) { ".exe" } else { "" };
    let name = format!("kari-node-{}-{target}{suffix}", release.tag);
    let base = format!(
        "https://github.com/{REPO}/releases/download/{}",
        release.tag
    );

    let c = client()?;
    let bin = get(&c, &format!("{base}/{name}"))?;
    let sums = get(&c, &format!("{base}/{name}.sha256"))?;
    let want = String::from_utf8(sums)
        .context("read the checksum file")?
        .split_whitespace()
        .next()
        .ok_or_else(|| anyhow!("{name}.sha256 is empty"))?
        .to_lowercase();
    let got = sha256(&bin);
    if got != want {
        bail!("{name} does not match its checksum: got {got}, expected {want}");
    }

    let staged = dir.join(format!(".kari-node-{}.new", release.tag));
    write_executable(&staged, &bin).with_context(|| {
        format!(
            "write {}. A node cannot replace itself from a read-only \
             directory, such as the Nix store; update it the way its host \
             installs it.",
            staged.display()
        )
    })?;
    if let Err(e) = swap(&staged, &exe) {
        let _ = std::fs::remove_file(&staged);
        return Err(e);
    }
    Ok(Outcome {
        from: running,
        to: release.version,
        replaced: true,
    })
}

fn get(c: &reqwest::blocking::Client, url: &str) -> Result<Vec<u8>> {
    let r = c.get(url).send().with_context(|| format!("fetch {url}"))?;
    if !r.status().is_success() {
        bail!("{url} answered {}", r.status());
    }
    Ok(r.bytes().with_context(|| format!("read {url}"))?.to_vec())
}

fn sha256(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(bytes);
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// Write the staged binary, executable, and flushed to disk.
///
/// Beside the running binary on purpose: a rename across filesystems fails,
/// and a temporary directory is usually on another one.
fn write_executable(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut f = std::fs::File::create(path)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

/// Put the staged file where the running one is.
///
/// Unix replaces the directory entry and this process keeps the inode it was
/// started from. Windows refuses to overwrite a running image but does allow
/// renaming it away, so the old one is moved aside first and swept up by the
/// next start, which is the earliest moment nothing holds it open.
#[cfg(not(windows))]
fn swap(staged: &Path, exe: &Path) -> Result<()> {
    std::fs::rename(staged, exe).with_context(|| format!("replace {}", exe.display()))?;
    Ok(())
}

#[cfg(windows)]
fn swap(staged: &Path, exe: &Path) -> Result<()> {
    let old = aside(exe);
    let _ = std::fs::remove_file(&old);
    std::fs::rename(exe, &old).with_context(|| format!("move {} aside", exe.display()))?;
    if let Err(e) = std::fs::rename(staged, exe) {
        // Put the running binary back. Leaving none at all would turn a failed
        // update into a host with no node.
        let _ = std::fs::rename(&old, exe);
        return Err(e).with_context(|| format!("replace {}", exe.display()));
    }
    Ok(())
}

fn aside(exe: &Path) -> PathBuf {
    let mut s = exe.as_os_str().to_os_string();
    s.push(".old");
    PathBuf::from(s)
}

/// Remove the binary a previous update moved aside.
///
/// Windows only, and only ever best effort: on the first start after an update
/// nothing holds the old image any more, and if something still does, the next
/// start tries again.
pub fn sweep() {
    if !cfg!(windows) {
        return;
    }
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::fs::remove_file(aside(&exe));
    }
}
