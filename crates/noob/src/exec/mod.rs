//! Run one child process to completion and collect its merged output.
//!
//! Shared by every tool that shells out (`bash`, `websearch`). The care here
//! is all about not leaving anything behind: the child gets its own session so
//! a timeout or Ctrl-C kills the whole tree, stdout and stderr share one pipe
//! so interleaving matches a terminal, the reader runs on its own thread so a
//! fast producer cannot deadlock against the wait, and the leader is polled
//! with WNOWAIT so its pgid cannot be recycled under a later group kill.
//!
//! The caller owns the argv and the summary; this owns the process lifecycle.

use std::time::Duration;

mod buffer;
#[cfg(unix)]
mod unix;
#[cfg(unix)]
pub(crate) use unix::*;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub(crate) use windows::*;

/// What one finished command left behind. `body` is the merged, truncated
/// output, already carrying any note about escaped background processes.
pub(crate) struct Run {
    pub code: i32,
    pub body: String,
    pub elapsed: Duration,
}

/// Cancellation and timeout are errors, not exit codes: both mean the output
/// is partial and the caller must say so rather than report a verdict.
pub(crate) enum RunError {
    Canceled { body: String, elapsed: Duration },
    TimedOut { body: String, timeout_s: u64 },
    Spawn(String),
}

