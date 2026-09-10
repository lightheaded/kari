//! What an autopilot run must not do.
//!
//! Autopilot can do the work and open a pull request. It must not release.
//! A run that autopilot started reaches this module before every Bash tool
//! call, and a release-shaped command is refused with a reason the session
//! can act on.
//!
//! The match is on intent, not on a substring. The command line is split the
//! way a shell splits it, each part is reduced to a program and its
//! arguments, and the program is judged by the base name of its path. So
//! `/opt/homebrew/bin/gh pr merge 34` and `gh pr merge 34` are the same
//! refusal, and a commit message that holds the words `gh release` is not a
//! refusal at all.
//!
//! What this module does NOT catch is written in the tests and in the pull
//! request that added it. A gate that claims to be complete, and is not, is
//! worse than a gate with a documented edge.

/// One reason to refuse a command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// The command merges a pull request, or moves the main branch.
    Merge,
    /// The command tags a version, or publishes a release.
    Release,
    /// The command pushes a container image to a registry.
    ImagePush,
    /// The command writes in a directory where a commit deploys.
    ProtectedPath,
}

impl Refusal {
    /// What the session is told. It says what happened, and what to do next,
    /// because the run continues after the refusal.
    pub fn reason(self) -> &'static str {
        match self {
            Refusal::Merge => {
                "kari stopped this command. Autopilot started this run, and an autopilot run \
                 cannot merge a pull request or move the main branch. Open a pull request and \
                 stop. A person merges it."
            }
            Refusal::Release => {
                "kari stopped this command. Autopilot started this run, and an autopilot run \
                 cannot tag a version or publish a release. Open a pull request and stop. \
                 A person releases it."
            }
            Refusal::ImagePush => {
                "kari stopped this command. Autopilot started this run, and an autopilot run \
                 cannot push an image to a registry. Open a pull request and stop. A person \
                 publishes the image."
            }
            Refusal::ProtectedPath => {
                "kari stopped this command. Autopilot started this run, and a commit in this \
                 directory deploys. Open a pull request and stop. A person applies it."
            }
        }
    }

    /// A short label for the board.
    pub fn label(self) -> &'static str {
        match self {
            Refusal::Merge => "merge",
            Refusal::Release => "release",
            Refusal::ImagePush => "image push",
            Refusal::ProtectedPath => "deploy path",
        }
    }
}

/// What the gate knows about the run that wants to use the command.
#[derive(Debug, Default, Clone)]
pub struct Run<'a> {
    /// The directory the tool call runs in, from the hook payload.
    pub cwd: Option<&'a str>,
    /// The branch that is checked out in `cwd`. The caller reads it only for
    /// a push with no refspec, because that push moves the current branch.
    pub branch: Option<&'a str>,
    /// Directories where a commit deploys. Empty unless the user names one,
    /// so this repository holds no private path.
    pub protected: &'a [String],
}

/// How deep a `sh -c` chain is followed. A command nests this far in practice
/// and never further, and the limit keeps a crafted string from looping.
const MAX_DEPTH: u8 = 4;

/// The reason to refuse `command`, or None when the run may use it.
pub fn refusal(command: &str, run: &Run) -> Option<Refusal> {
    refusal_at(command, run, 0)
}

fn refusal_at(command: &str, run: &Run, depth: u8) -> Option<Refusal> {
    if depth > MAX_DEPTH {
        return None;
    }
    segments(command)
        .iter()
        .filter_map(|s| {
            let w = words(s);
            if w.is_empty() {
                None
            } else {
                segment_refusal(&w, run, depth)
            }
        })
        .next()
}

/// Split a command line where a shell starts a new command: `;`, a newline,
/// `&&`, `||`, `|`, `&`, and the brackets of a subshell or a command
/// substitution. Quoted text is left whole, so `git commit -m "a; b"` is one
/// segment.
fn segments(command: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut it = command.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '\\' if quote != Some('\'') => {
                cur.push(c);
                if let Some(n) = it.next() {
                    cur.push(n);
                }
            }
            '\'' | '"' => {
                match quote {
                    Some(q) if q == c => quote = None,
                    None => quote = Some(c),
                    _ => {}
                }
                cur.push(c);
            }
            _ if quote.is_some() => cur.push(c),
            ';' | '\n' | '&' | '|' => {
                // A doubled operator is one separator, not two.
                if (c == '&' || c == '|') && it.peek() == Some(&c) {
                    it.next();
                }
                out.push(std::mem::take(&mut cur));
            }
            // `$(`, a subshell and a backtick all start a command of their own.
            '(' | ')' | '`' | '{' | '}' => out.push(std::mem::take(&mut cur)),
            _ => cur.push(c),
        }
    }
    out.push(cur);
    out.retain(|s| !s.trim().is_empty());
    out
}

/// Split one segment into words the way a shell does, and drop the quote
/// characters. `-m 'a b'` becomes `-m` and `a b`.
fn words(segment: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut started = false;
    let mut quote: Option<char> = None;
    let mut it = segment.chars();
    while let Some(c) = it.next() {
        match c {
            '\\' if quote != Some('\'') => {
                if let Some(n) = it.next() {
                    cur.push(n);
                    started = true;
                }
            }
            '\'' | '"' => {
                match quote {
                    Some(q) if q == c => quote = None,
                    None => quote = Some(c),
                    _ => cur.push(c),
                }
                started = true;
            }
            c if c.is_whitespace() && quote.is_none() => {
                if started {
                    out.push(std::mem::take(&mut cur));
                    started = false;
                }
            }
            _ => {
                cur.push(c);
                started = true;
            }
        }
    }
    if started {
        out.push(cur);
    }
    out
}

/// Words that stand in front of the real program and change nothing the gate
/// cares about. `env A=b gh ...` is a `gh` command.
const PREFIXES: &[&str] = &[
    "env", "command", "builtin", "exec", "nohup", "time", "sudo", "doas", "nice", "stdbuf",
];

/// The program of a segment, reduced to the base name of its path, and the
/// index it sits at. `/usr/bin/gh` and `gh.exe` both answer `gh`.
fn program(words: &[String]) -> Option<(String, usize)> {
    let mut i = 0;
    while i < words.len() {
        let w = words[i].as_str();
        if PREFIXES.contains(&w) || is_assignment(w) {
            i += 1;
            continue;
        }
        break;
    }
    let w = words.get(i)?;
    let base = w.rsplit(['/', '\\']).next().unwrap_or(w);
    let base = base.strip_suffix(".exe").unwrap_or(base);
    if base.is_empty() {
        return None;
    }
    Some((base.to_ascii_lowercase(), i))
}

/// `NAME=value`, the shape that stands in front of a command.
fn is_assignment(w: &str) -> bool {
    match w.split_once('=') {
        Some((k, _)) => {
            !k.is_empty()
                && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                && !k.starts_with(|c: char| c.is_ascii_digit())
        }
        None => false,
    }
}

fn segment_refusal(words: &[String], run: &Run, depth: u8) -> Option<Refusal> {
    let (prog, at) = program(words)?;
    let rest = &words[at + 1..];
    match prog.as_str() {
        // A wrapper carries the real command in the word after `-c`.
        "sh" | "bash" | "zsh" | "dash" | "ksh" | "fish" => {
            let inner = rest.iter().position(|w| w == "-c")?;
            refusal_at(rest.get(inner + 1)?, run, depth + 1)
        }
        // A runner carries the real program in its own arguments. The estate
        // pushes an image with `nix run nixpkgs#skopeo -- copy`, so a gate
        // that reads only the first word sees `nix` and lets it through.
        "nix" => nix_refusal(rest, run, depth),
        "npx" | "bunx" | "uvx" | "pipx" => {
            let at = rest.iter().position(|w| !w.starts_with('-'))?;
            let mut inner = vec![package_program(&rest[at])];
            inner.extend(rest[at + 1..].iter().cloned());
            segment_refusal(&inner, run, depth + 1)
        }
        "gh" => gh_refusal(rest),
        "git" => git_refusal(rest, run),
        "skopeo" => first_word(rest)
            .filter(|s| *s == "copy" || *s == "sync")
            .map(|_| Refusal::ImagePush),
        "podman" | "docker" | "nerdctl" | "crane" | "regctl" => first_word(rest)
            .filter(|s| *s == "push")
            .map(|_| Refusal::ImagePush),
        _ => None,
    }
}

/// The program a package reference names. `nixpkgs#skopeo` is `skopeo`, and
/// `github:o/r#pkgs.skopeo` is `skopeo` as well.
fn package_program(reference: &str) -> String {
    reference
        .rsplit(['#', '/', '.', ':'])
        .next()
        .unwrap_or(reference)
        .to_string()
}

fn nix_refusal(rest: &[String], run: &Run, depth: u8) -> Option<Refusal> {
    let sub = first_word(rest)?;
    match sub {
        // `nix run <ref> [--] <args...>`
        "run" => {
            let at = rest.iter().position(|w| w.as_str() == "run")?;
            let refpos = rest
                .iter()
                .enumerate()
                .skip(at + 1)
                .find(|(_, w)| !w.starts_with('-'))
                .map(|(i, _)| i)?;
            let args: Vec<String> = match rest.iter().position(|w| w == "--") {
                Some(i) => rest[i + 1..].to_vec(),
                None => rest[refpos + 1..].to_vec(),
            };
            let mut inner = vec![package_program(&rest[refpos])];
            inner.extend(args);
            segment_refusal(&inner, run, depth + 1)
        }
        // `nix shell <ref> -c <cmd> <args...>`
        "shell" | "develop" => {
            let c = rest.iter().position(|w| w == "-c" || w == "--command")?;
            let inner = rest[c + 1..].to_vec();
            if inner.is_empty() {
                None
            } else {
                segment_refusal(&inner, run, depth + 1)
            }
        }
        _ => None,
    }
}

/// The first word that is not a flag.
fn first_word(rest: &[String]) -> Option<&str> {
    rest.iter()
        .find(|w| !w.starts_with('-'))
        .map(|s| s.as_str())
}

fn gh_refusal(rest: &[String]) -> Option<Refusal> {
    let subs: Vec<&str> = rest
        .iter()
        .filter(|w| !w.starts_with('-'))
        .map(|s| s.as_str())
        .collect();
    match *subs.first()? {
        // Every release subcommand, on purpose. A run that must not release
        // has no need to read the release list either, and a narrow match
        // here is the kind of gap that lets a write through.
        "release" => Some(Refusal::Release),
        "pr" if subs.get(1) == Some(&"merge") => Some(Refusal::Merge),
        _ => None,
    }
}

fn git_refusal(rest: &[String], run: &Run) -> Option<Refusal> {
    // Global options stand before the subcommand. `-C` moves the directory,
    // which decides whether the command writes in a protected path.
    let mut i = 0;
    let mut dir: Option<&str> = None;
    while i < rest.len() {
        match rest[i].as_str() {
            "-C" => {
                dir = rest.get(i + 1).map(|s| s.as_str());
                i += 2;
            }
            "-c" | "--git-dir" | "--work-tree" | "--namespace" => i += 2,
            w if w.starts_with('-') => i += 1,
            _ => break,
        }
    }
    let sub = rest.get(i)?.as_str();
    let args = &rest[i + 1..];

    // A commit in a deploy directory is the deploy, whatever it says.
    let where_it_runs = dir.or(run.cwd);
    if in_protected(where_it_runs, run.protected)
        && matches!(
            sub,
            "commit" | "push" | "merge" | "am" | "cherry-pick" | "revert" | "rebase" | "tag"
        )
    {
        return Some(Refusal::ProtectedPath);
    }

    match sub {
        "tag" => {
            let lists = args.iter().any(|a| a == "-l" || a == "--list");
            let writes = args.iter().any(|a| {
                matches!(
                    a.as_str(),
                    "-a" | "-s"
                        | "-f"
                        | "-d"
                        | "-m"
                        | "--annotate"
                        | "--sign"
                        | "--force"
                        | "--delete"
                        | "--message"
                )
            });
            let names = args.iter().any(|a| !a.starts_with('-'));
            // Bare `git tag` prints the list, so it is a read.
            if lists && !writes {
                None
            } else if names || writes {
                Some(Refusal::Release)
            } else {
                None
            }
        }
        "push" => {
            let flags: Vec<&str> = args
                .iter()
                .filter(|a| a.starts_with('-'))
                .map(|s| s.as_str())
                .collect();
            let refs: Vec<&str> = args
                .iter()
                .filter(|a| !a.starts_with('-'))
                .map(|s| s.as_str())
                .collect();
            if flags
                .iter()
                .any(|f| *f == "--tags" || *f == "--follow-tags")
            {
                return Some(Refusal::Release);
            }
            if refs.iter().skip(1).any(|r| is_tag_ref(r)) {
                return Some(Refusal::Release);
            }
            if refs.iter().skip(1).any(|r| is_main_ref(r)) {
                return Some(Refusal::Merge);
            }
            // With no refspec the current branch goes up, so the branch that
            // is checked out decides.
            if refs.len() <= 1 && run.branch.is_some_and(is_main_ref) {
                return Some(Refusal::Merge);
            }
            None
        }
        _ => None,
    }
}

/// The destination half of a refspec. `HEAD:main` points at `main`.
fn destination(r: &str) -> &str {
    let r = r.strip_prefix('+').unwrap_or(r);
    match r.split_once(':') {
        Some((_, dst)) => dst,
        None => r,
    }
}

fn is_main_ref(r: &str) -> bool {
    let d = destination(r);
    let d = d.strip_prefix("refs/heads/").unwrap_or(d);
    d == "main" || d == "master"
}

/// A tag refspec, or a name that reads as a version tag.
fn is_tag_ref(r: &str) -> bool {
    let d = destination(r);
    if d.starts_with("refs/tags/") {
        return true;
    }
    let rest = match d.strip_prefix('v') {
        Some(rest) => rest,
        None => return false,
    };
    rest.starts_with(|c: char| c.is_ascii_digit())
}

/// True when `path` is one of the protected roots, or sits inside one.
fn in_protected(path: Option<&str>, protected: &[String]) -> bool {
    let Some(path) = path else { return false };
    let path = path.trim_end_matches('/');
    protected.iter().any(|root| {
        let root = root.trim().trim_end_matches('/');
        !root.is_empty()
            && (path == root
                || path
                    .strip_prefix(root)
                    .is_some_and(|rest| rest.starts_with('/')))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain() -> Run<'static> {
        Run::default()
    }

    /// Everything the gate must refuse.
    #[test]
    fn refuses_a_merge() {
        for c in [
            "gh pr merge 34",
            "gh pr merge --squash 34",
            "/opt/homebrew/bin/gh pr merge 34",
            "cd /tmp && gh pr merge 34",
            "sh -c 'gh pr merge 34'",
            "GH_TOKEN=x gh pr merge 34",
        ] {
            assert_eq!(refusal(c, &plain()), Some(Refusal::Merge), "{c}");
        }
    }

    #[test]
    fn refuses_a_release() {
        for c in [
            "gh release create v1.0.0",
            "gh release edit v0.11.2 --draft=false",
            "git tag -s v1.0.0 -m x",
            "git tag v1.0.0",
            "git push --tags",
            "git push origin v0.11.2",
            "git push origin refs/tags/v1",
            "bash -c \"git tag -a v2 -m two\"",
        ] {
            assert_eq!(refusal(c, &plain()), Some(Refusal::Release), "{c}");
        }
    }

    #[test]
    fn refuses_an_image_push() {
        for c in [
            "skopeo copy docker-archive:/tmp/i docker://reg.example/x:1",
            "nix run nixpkgs#skopeo -- copy docker-archive:/tmp/i docker://reg.example/x:1",
            "docker push reg.example/x:1",
            "podman push reg.example/x:1",
            "nix shell nixpkgs#skopeo -c skopeo copy a docker://b",
            "npx -y crane push reg.example/x:1",
        ] {
            assert_eq!(refusal(c, &plain()), Some(Refusal::ImagePush), "{c}");
        }
    }

    #[test]
    fn refuses_a_push_that_moves_main() {
        assert_eq!(
            refusal("git push origin main", &plain()),
            Some(Refusal::Merge)
        );
        assert_eq!(
            refusal("git push origin HEAD:main", &plain()),
            Some(Refusal::Merge)
        );
        // No refspec: the branch that is checked out decides.
        let on_main = Run {
            branch: Some("main"),
            ..Run::default()
        };
        assert_eq!(refusal("git push", &on_main), Some(Refusal::Merge));
    }

    #[test]
    fn refuses_a_commit_where_a_commit_deploys() {
        let roots = vec!["/x/infra".to_string()];
        let inside = Run {
            cwd: Some("/x/infra/clusters/production"),
            protected: &roots,
            ..Run::default()
        };
        assert_eq!(
            refusal("git commit -m 'move the image'", &inside),
            Some(Refusal::ProtectedPath)
        );
        // The directory can also arrive as an option.
        let outside = Run {
            cwd: Some("/home/you/dev/kari"),
            protected: &roots,
            ..Run::default()
        };
        assert_eq!(
            refusal("git -C /x/infra commit -m x", &outside),
            Some(Refusal::ProtectedPath)
        );
    }

    /// Everything the gate must allow. A gate that stops the work is useless.
    #[test]
    fn allows_the_work() {
        for c in [
            "cargo test",
            "cargo clippy --workspace",
            "npm run build",
            "git status",
            "git diff main",
            "git log --oneline -5",
            "git add -A",
            "git commit -m 'fix the send path'",
            "git commit -m 'gh release create is named here'",
            "git push -u origin my-branch",
            "git push",
            "gh pr create --fill",
            "gh pr view 34",
            "gh pr list",
            "gh issue create -t x",
            "git tag",
            "git tag -l",
            "git tag --list 'v0.11.*'",
            "docker build -t x .",
            "skopeo inspect docker://reg.example/x:1",
            "nix run nixpkgs#skopeo -- inspect docker://reg.example/x:1",
            "nix build github:o/r/v1#image -o /tmp/i",
            "rg 'gh release' --files-with-matches",
        ] {
            assert_eq!(refusal(c, &plain()), None, "{c}");
        }
    }

    /// A push with no refspec on a feature branch is the normal way to open a
    /// pull request, so it must pass.
    #[test]
    fn allows_a_push_on_a_feature_branch() {
        let run = Run {
            branch: Some("my-branch"),
            ..Run::default()
        };
        assert_eq!(refusal("git push", &run), None);
        assert_eq!(refusal("git push origin my-branch", &run), None);
    }

    /// A commit outside every protected root is ordinary work.
    #[test]
    fn allows_a_commit_outside_the_deploy_path() {
        let roots = vec!["/x/infra".to_string()];
        let run = Run {
            cwd: Some("/home/you/dev/kari"),
            protected: &roots,
            ..Run::default()
        };
        assert_eq!(refusal("git commit -m x", &run), None);
        // A directory whose name only starts the same must not match.
        let near = Run {
            cwd: Some("/x/infrastructure-notes"),
            protected: &roots,
            ..Run::default()
        };
        assert_eq!(refusal("git commit -m x", &near), None);
    }

    /// Text that names a command, inside a quoted argument, is data.
    #[test]
    fn a_quoted_argument_is_not_a_command() {
        assert_eq!(
            refusal("git commit -m 'do not run gh pr merge here'", &plain()),
            None
        );
        assert_eq!(refusal("echo 'git tag v1.0.0'", &plain()), None);
    }

    /// A separator inside quotes does not start a new command.
    #[test]
    fn a_separator_inside_quotes_is_text() {
        assert_eq!(
            refusal("git commit -m 'first; then gh release create v1'", &plain()),
            None
        );
    }

    #[test]
    fn a_deep_wrapper_chain_stops() {
        // Five levels is past the limit, so this is not caught. The test
        // records the edge rather than pretending it does not exist.
        let deep = "sh -c 'sh -c \"sh -c 1\"'";
        assert_eq!(refusal(deep, &plain()), None);
    }

    #[test]
    fn the_program_is_read_from_its_base_name() {
        assert_eq!(
            program(&words("/usr/local/bin/git status")),
            Some(("git".into(), 0))
        );
        assert_eq!(
            program(&words("env A=1 B=2 gh pr merge")),
            Some(("gh".into(), 3))
        );
        assert_eq!(program(&words("A=1 ./gh pr merge")), Some(("gh".into(), 1)));
    }
}
