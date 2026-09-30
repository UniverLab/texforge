//! Process-global locks for tests that mutate process-global state.
//!
//! `cargo test` and nextest run tests on many threads inside one process.
//! A test that calls `set_current_dir` must hold [`CWD_LOCK`] for the whole
//! swap span: otherwise a parallel test observes a working directory that
//! is not its own — or its directory is deleted out from under it and even
//! `current_dir()` fails.

use std::sync::Mutex;

/// Held for the duration of every `set_current_dir` swap in tests.
pub static CWD_LOCK: Mutex<()> = Mutex::new(());
