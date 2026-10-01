//! Process-global locks and guards for tests that mutate process-global state.
//!
//! `cargo test` and nextest run tests on many threads inside one process.
//! A test that calls `set_current_dir` must hold [`CWD_LOCK`] for the whole
//! swap span: otherwise a parallel test observes a working directory that
//! is not its own — or its directory is deleted out from under it and even
//! `current_dir()` fails. The same holds for the process environment, guarded
//! by [`ENV_LOCK`] and snapshotted with [`EnvGuard`].

use std::ffi::OsString;
use std::sync::Mutex;

/// Held for the duration of every `set_current_dir` swap in tests.
pub static CWD_LOCK: Mutex<()> = Mutex::new(());

/// Held for the duration of every test that mutates `HOME` or
/// `XDG_CONFIG_HOME`. `config_file_path` and `dirs::home_dir` read both,
/// so an unguarded swap sends another test's config read or write to the
/// wrong directory — up to the developer's real `~/.texforge`. A poisoned
/// lock (a test already failed) is still usable: the failure itself is
/// reported by the panicking test.
pub static ENV_LOCK: Mutex<()> = Mutex::new(());

/// Snapshots the given environment variables and restores them on drop —
/// including while a panicking test's stack unwinds, which restore code
/// placed after the closure call would never reach.
pub(crate) struct EnvGuard {
    saved: Vec<(&'static str, Option<OsString>)>,
}

impl EnvGuard {
    /// Records the current value of every variable in `vars` (including
    /// "unset") so `Drop` can put the environment back exactly as it was.
    pub(crate) fn capture(vars: &[&'static str]) -> Self {
        Self {
            saved: vars
                .iter()
                .map(|&name| (name, std::env::var_os(name)))
                .collect(),
        }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for (name, value) in std::mem::take(&mut self.saved) {
            match value {
                Some(value) => std::env::set_var(name, value),
                None => std::env::remove_var(name),
            }
        }
    }
}
