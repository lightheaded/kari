//! Child processes that must not open a window, and children that must not
//! count as kari to the privacy checks of macOS (see `disclaim`).
//!
//! The desktop app is a GUI process and has no console of its own. On Windows a
//! child started from such a process is given a brand new console window unless
//! it is told otherwise — and kari shells out on a timer, to `curl` for the
//! usage endpoint and to `claude` for the job list. Without this a console
//! blinks on the user's desktop every couple of minutes.
//!
//! The node never showed it, which is why it went unnoticed until the app ran
//! on Windows: a console program's children inherit the console it already has,
//! so there is nothing to create.
//!
//! `launcher::open_in_terminal_argv` is the one place that wants a window, and
//! it asks for CREATE_NEW_CONSOLE instead of coming through here.

use std::process::Command;

/// Run this child without a console window of its own. A no-op off Windows,
/// where a child inherits the parent's terminal and creates nothing.
pub fn quiet(cmd: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        /// CREATE_NO_WINDOW.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// Make this child answer for itself to macOS privacy checks, and not kari.
///
/// macOS charges a privacy request (TCC) to the *responsible* process of the
/// requester, and a child inherits the responsible process of its parent. So
/// a `claude --bg` from kari makes kari responsible for the Claude daemon that
/// call starts, and for every session the daemon starts after that — also the
/// sessions the user starts with `claude agents` from a terminal, long after
/// kari has quit. Those sessions start the MCP servers of the user, and when
/// the 1Password one reads its group container, the dialog says that kari
/// "would like to access data from other apps".
///
/// With the flag the child is responsible for itself, the same as a daemon
/// that a terminal starts. A dialog then names the `claude` binary and not
/// kari.
///
/// Call this last, after every argument and variable is set: it copies them
/// now. The call has no effect on anything but macOS, and it leaves the child
/// as it is when the flag is not in the system library.
pub fn disclaim(cmd: &mut Command) -> &mut Command {
    #[cfg(target_os = "macos")]
    macos::disclaim(cmd);
    cmd
}

#[cfg(target_os = "macos")]
mod macos {
    use std::ffi::{CString, OsStr, OsString};
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    type SetDisclaim =
        unsafe extern "C" fn(*mut libc::posix_spawnattr_t, libc::c_int) -> libc::c_int;

    /// The private libSystem call that Chromium and LLDB use for the same job.
    /// It is looked up at run time, so a macOS without it only loses the flag.
    fn set_disclaim() -> Option<SetDisclaim> {
        let name = c"responsibility_spawnattrs_setdisclaim";
        let f = unsafe { libc::dlsym(libc::RTLD_DEFAULT, name.as_ptr()) };
        (!f.is_null()).then(|| unsafe { std::mem::transmute::<*mut libc::c_void, SetDisclaim>(f) })
    }

    /// Everything the forked child needs, made before the fork. The child only
    /// calls `posix_spawnp`: it must not allocate, because another thread of
    /// kari can hold the allocator lock at the moment of the fork.
    struct Exec {
        attr: libc::posix_spawnattr_t,
        program: CString,
        _args: Vec<CString>,
        argv: Vec<*mut libc::c_char>,
        _env: Vec<CString>,
        envp: Vec<*mut libc::c_char>,
    }

    // The pointers go into the forked child only, which is single-threaded.
    unsafe impl Send for Exec {}
    unsafe impl Sync for Exec {}

    impl Drop for Exec {
        fn drop(&mut self) {
            unsafe { libc::posix_spawnattr_destroy(&mut self.attr) };
        }
    }

    fn cstr(s: &OsStr) -> Option<CString> {
        CString::new(s.as_bytes()).ok()
    }

    fn nul_terminated(v: &[CString]) -> Vec<*mut libc::c_char> {
        v.iter()
            .map(|s| s.as_ptr() as *mut libc::c_char)
            .chain(std::iter::once(std::ptr::null_mut()))
            .collect()
    }

    /// The environment the child gets: kari's own, changed as the command says.
    /// `Command` sets it only after the `pre_exec` hooks run, so the hook must
    /// pass it itself.
    fn child_env(cmd: &Command) -> Vec<(OsString, OsString)> {
        let mut env: Vec<(OsString, OsString)> = std::env::vars_os().collect();
        for (k, v) in cmd.get_envs() {
            env.retain(|(ek, _)| ek != k);
            if let Some(v) = v {
                env.push((k.to_owned(), v.to_owned()));
            }
        }
        env
    }

    fn prepare(cmd: &Command, set_disclaim: SetDisclaim) -> Option<Exec> {
        let program = cstr(cmd.get_program())?;
        let args: Vec<CString> = std::iter::once(Some(program.clone()))
            .chain(cmd.get_args().map(cstr))
            .collect::<Option<_>>()?;
        let env: Vec<CString> = child_env(cmd)
            .into_iter()
            .map(|(k, v)| {
                let mut kv = k;
                kv.push("=");
                kv.push(v);
                cstr(&kv)
            })
            .collect::<Option<_>>()?;
        let mut attr: libc::posix_spawnattr_t = std::ptr::null_mut();
        if unsafe { libc::posix_spawnattr_init(&mut attr) } != 0 {
            return None;
        }
        let mut exec = Exec {
            attr,
            program,
            argv: nul_terminated(&args),
            _args: args,
            envp: nul_terminated(&env),
            _env: env,
        };
        // SETEXEC turns the spawn into an exec of the forked child, so stdio,
        // the working directory and the process id stay what `Command` made.
        let ok = unsafe {
            libc::posix_spawnattr_setflags(&mut exec.attr, libc::POSIX_SPAWN_SETEXEC as _) == 0
                && set_disclaim(&mut exec.attr, 1) == 0
        };
        ok.then_some(exec)
    }

    pub(super) fn disclaim(cmd: &mut Command) {
        let Some(set_disclaim) = set_disclaim() else {
            return;
        };
        let Some(exec) = prepare(cmd, set_disclaim) else {
            return;
        };
        let hook = move || {
            let mut pid: libc::pid_t = 0;
            // Returns only on failure. On success the child is `program` now.
            let rc = unsafe {
                libc::posix_spawnp(
                    &mut pid,
                    exec.program.as_ptr(),
                    std::ptr::null(),
                    &exec.attr,
                    exec.argv.as_ptr(),
                    exec.envp.as_ptr(),
                )
            };
            Err(std::io::Error::from_raw_os_error(rc))
        };
        unsafe { cmd.pre_exec(hook) };
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use std::process::Stdio;

    type ResponsibleFor = unsafe extern "C" fn(libc::pid_t) -> libc::pid_t;

    fn responsible_for(pid: u32) -> libc::pid_t {
        let name = c"responsibility_get_pid_responsible_for_pid";
        let f = unsafe { libc::dlsym(libc::RTLD_DEFAULT, name.as_ptr()) };
        assert!(!f.is_null());
        let f = unsafe { std::mem::transmute::<*mut libc::c_void, ResponsibleFor>(f) };
        unsafe { f(pid as libc::pid_t) }
    }

    #[test]
    fn a_disclaimed_child_is_responsible_for_itself() {
        let mut plain = Command::new("/bin/sleep");
        plain.arg("2");
        let mut child = plain.spawn().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(200));
        assert_ne!(responsible_for(child.id()), child.id() as libc::pid_t);
        child.kill().unwrap();
        child.wait().unwrap();

        let mut cmd = Command::new("/bin/sleep");
        cmd.arg("2");
        disclaim(&mut cmd);
        let mut child = cmd.spawn().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(200));
        assert_eq!(responsible_for(child.id()), child.id() as libc::pid_t);
        child.kill().unwrap();
        child.wait().unwrap();
    }

    #[test]
    fn a_disclaimed_child_keeps_args_env_cwd_and_stdio() {
        let dir = std::env::temp_dir();
        let mut cmd = Command::new("/bin/sh");
        cmd.args([
            "-c",
            "printf '%s|%s|%s|%s' \"$1\" \"$KARI_T\" \"${HOME:+home}\" \"$PWD\"",
            "sh",
            "a b",
        ])
        .current_dir(&dir)
        .env("KARI_T", "yes")
        .env_remove("HOME")
        .stdout(Stdio::piped());
        disclaim(&mut cmd);
        let out = cmd.output().unwrap();
        assert!(out.status.success());
        let dir = dir.canonicalize().unwrap();
        assert_eq!(
            String::from_utf8(out.stdout).unwrap(),
            format!("a b|yes||{}", dir.display())
        );
    }

    #[test]
    fn a_missing_program_is_still_an_error() {
        let mut cmd = Command::new("/nonexistent/kari-test");
        disclaim(&mut cmd);
        assert!(cmd.output().is_err());
    }
}
