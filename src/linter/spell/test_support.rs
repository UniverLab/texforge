//! Test-only helpers shared by the spell-check test modules.
//!
//! Every test that mutates `HOME` or the dictionary environment must hold
//! [`crate::test_sync::ENV_LOCK`]: the environment is process-global while
//! `cargo test` runs threads in parallel, and two overlapping tests would
//! swap `HOME` out from under each other.

use std::fs;
use std::path::PathBuf;

use tempfile::TempDir;

// --- TE11: Hunspell backend via `spellbook`, hand-written fixture pair ---

/// Absolute paths to the minimal, hand-written fixture pair committed
/// under `tests/fixtures/hunspell/`. Deliberately not a copy of a real
/// dictionary: a handful of stems and exactly one affix rule.
pub(super) fn hunspell_fixture_paths() -> (PathBuf, PathBuf) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    (
        root.join("tests/fixtures/hunspell/mini.dic"),
        root.join("tests/fixtures/hunspell/mini.aff"),
    )
}

// --- TF-spell: accent macro resolution ---

pub(super) fn run_with_home<F, R>(spanish_words: &str, english_words: &str, f: F) -> R
where
    F: FnOnce() -> R,
{
    let _lock = crate::test_sync::ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let home = TempDir::new().unwrap();
    let dicts_dir = home.path().join(".texforge").join("dicts");
    fs::create_dir_all(&dicts_dir).unwrap();
    fs::write(dicts_dir.join("spanish.txt"), spanish_words).unwrap();
    fs::write(dicts_dir.join("english.txt"), english_words).unwrap();

    let orig_home = std::env::var("HOME").ok();
    let orig_nex = std::env::var("NEXTEST_RUN_ID").ok();
    std::env::set_var("HOME", home.path());
    std::env::set_var("NEXTEST_RUN_ID", "tf-spell-accent");

    let result = f();

    std::env::remove_var("NEXTEST_RUN_ID");
    if let Some(v) = orig_nex {
        std::env::set_var("NEXTEST_RUN_ID", v);
    }
    match orig_home {
        Some(v) => std::env::set_var("HOME", v),
        None => std::env::remove_var("HOME"),
    }
    result
}
