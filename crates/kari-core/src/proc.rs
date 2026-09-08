//! Child processes that must not open a window.
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
