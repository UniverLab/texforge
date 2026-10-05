//! Test-only support helpers and synchronization primitives.
//!
//! Compiled only under `#[cfg(test)]` via the crate-root declaration in
//! `src/main.rs`; production binaries never include this module.

use std::fs::{File, OpenOptions};

/// Machine-wide exclusive lock for tests that invoke Tectonic.
///
/// Tectonic downloads its support bundle into a shared cache. When that cache
/// is cold, concurrent compiles race on the same files and one may read a
/// half-written file (for example `Bad \patterns` from `hyph-cu.tex`). An
/// in-process `Mutex` is not enough: `cargo nextest` runs each test in its own
/// process and parallel `cargo-mutants` jobs are separate processes too. This
/// opens a lock file in the system temp directory and takes an OS-level
/// exclusive lock, serialising the critical section across threads and
/// processes.
///
/// The lock is held until the returned [`File`] is dropped, so bind it to a
/// named local (`let _tectonic = ...`) for the whole test — never `let _ =`,
/// which drops it immediately.
pub(crate) fn tectonic_lock() -> File {
    let path = std::env::temp_dir().join("texforge-tectonic-tests.lock");
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .unwrap_or_else(|e| panic!("failed to open tectonic test lock {}: {e}", path.display()));
    file.lock().unwrap_or_else(|e| {
        panic!(
            "failed to acquire tectonic test lock {}: {e}",
            path.display()
        )
    });
    file
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::{Duration, Instant};

    /// One lock-holding interval, recorded by the thread that held the lock.
    #[derive(Clone, Copy, Debug)]
    struct Span {
        _thread: usize,
        enter: Instant,
        exit: Instant,
    }

    /// Takes the machine-wide lock, holds it briefly, and records the interval
    /// *while still holding it* so the recorded exit can never be observed as
    /// overlapping another thread's enter.
    fn hold_lock(thread: usize, spans: &Mutex<Vec<Span>>) {
        let guard = tectonic_lock();
        let enter = Instant::now();
        thread::sleep(Duration::from_millis(50));
        let exit = Instant::now();
        drop(guard);
        spans.lock().unwrap().push(Span {
            _thread: thread,
            enter,
            exit,
        });
    }

    #[test]
    fn tectonic_lock_serializes_concurrent_threads() {
        let spans = Arc::new(Mutex::new(Vec::new()));
        let first = {
            let spans = Arc::clone(&spans);
            thread::spawn(move || hold_lock(0, &spans))
        };
        let second = {
            let spans = Arc::clone(&spans);
            thread::spawn(move || hold_lock(1, &spans))
        };
        first.join().unwrap();
        second.join().unwrap();

        let recorded = spans.lock().unwrap();
        assert_eq!(recorded.len(), 2, "each locker records one span");
        let (a, b) = (recorded[0], recorded[1]);
        assert!(
            a.exit <= b.enter || b.exit <= a.enter,
            "critical sections overlapped: {a:?} and {b:?}",
        );
    }
}
