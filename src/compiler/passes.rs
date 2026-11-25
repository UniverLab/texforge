// Workaround for tectonic#1412 (fix proposed in #1413): remove this module and call tectonic_command once when the pinned tectonic release carries that fix.

//! One Tectonic process per TeX pass.
//!
//! Tectonic runs every pass of a build in a single process, and its
//! glyph-box cache outlives `XeTeX`'s per-pass font numbering: a font that
//! gets a different number in a later pass inherits the cached box of
//! whatever font held that number before, stretching lines (upstream
//! issue tectonic#1412, fix proposed in #1413). Running each pass as its
//! own process gives every pass a fresh cache, so the boxes are right.
//!
//! The intermediate passes write `.xdv` instead of a PDF and stop once
//! the auxiliary files (`.aux`, `.toc`, `.bbl`, ...) settle; one final
//! pass then writes the PDF. The rest of [`crate::compiler::compile`] sees
//! only that final pass, exactly as if one Tectonic call had run.

use std::path::Path;
use std::process::{Command, Output};

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};

/// Intermediate (`.xdv`) passes before the final PDF pass.
const MAX_PASSES: usize = 6;
/// Intermediate passes always run, even when the first two agree at once.
const MIN_PASSES: usize = 2;

/// Auxiliary-file extensions fingerprinted between passes. Files with any
/// other extension (sources, the PDF, the `.xdv`) never settle a build.
const AUX_EXTENSIONS: &[&str] = &[
    "aux", "toc", "lof", "lot", "out", "bbl", "nav", "snm", "brf", "glo", "ind", "idx",
];

/// Whether the loop may stop after an intermediate pass.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Stop {
    /// Run another intermediate pass.
    Continue,
    /// The auxiliary files settled: run the final PDF pass.
    Converged,
    /// The cap was reached before the files settled: warn, then run the
    /// final PDF pass.
    CapReached,
}

/// Run each TeX pass in its own Tectonic process and return the final
/// pass's output: its exit status, stdout and stderr are what the rest of
/// `compile` parses, so early passes never leak warnings into the report.
/// A failing intermediate pass stops the loop at once and its output is
/// returned as is.
pub(super) fn run_passes(
    tectonic: &Path,
    root: &Path,
    entry: &str,
    epoch: Option<u64>,
) -> Result<Output> {
    if std::env::var_os("TEXFORGE_SINGLE_PROCESS").is_some() {
        return single_process(tectonic, root, entry, epoch);
    }
    let mut previous: Option<[u8; 32]> = None;
    for done in 1..=MAX_PASSES {
        let output = pass_cmd(tectonic, root, entry, epoch, true)
            .output()
            .with_context(|| format!("Failed to run tectonic at {}", tectonic.display()))?;
        if done == 1 && !output.status.success() && fallback_needed(&output) {
            eprintln!("{}", fallback_warning());
            return single_process(tectonic, root, entry, epoch);
        }
        if !output.status.success() {
            return Ok(output);
        }
        let current = fingerprint(root);
        match should_stop(done, previous, current) {
            Stop::Continue => previous = Some(current),
            Stop::Converged => break,
            Stop::CapReached => {
                eprintln!(
                    "warning: the auxiliary files were still changing after 6 passes \
                     (references, table of contents or bibliography may be out of date)"
                );
                break;
            }
        }
    }
    pass_cmd(tectonic, root, entry, epoch, false)
        .output()
        .with_context(|| format!("Failed to run tectonic at {}", tectonic.display()))
}

/// The old single-process build: one Tectonic call for every pass.
fn single_process(tectonic: &Path, root: &Path, entry: &str, epoch: Option<u64>) -> Result<Output> {
    super::tectonic_command(tectonic, root, entry, epoch)
        .output()
        .with_context(|| format!("Failed to run tectonic at {}", tectonic.display()))
}

/// One pass of the loop: like [`super::tectonic_command`] plus
/// `--keep-intermediates --reruns 0`, writing `.xdv` for an intermediate
/// pass and the PDF for the final one.
fn pass_cmd(
    tectonic: &Path,
    root: &Path,
    entry: &str,
    epoch: Option<u64>,
    intermediate: bool,
) -> Command {
    let mut cmd = super::tectonic_command(tectonic, root, entry, epoch);
    cmd.arg("--keep-intermediates").arg("--reruns").arg("0");
    if intermediate {
        cmd.arg("--outfmt").arg("xdv");
    }
    cmd
}

/// Decide whether the loop may stop after `done` intermediate passes:
/// `previous` is the fingerprint after the pass before, `current` after
/// this one. Equal hashes settle the loop once at least [`MIN_PASSES`]
/// ran; pass [`MAX_PASSES`] still changing hits the cap instead.
fn should_stop(done: usize, previous: Option<[u8; 32]>, current: [u8; 32]) -> Stop {
    if previous == Some(current) && done >= MIN_PASSES {
        return Stop::Converged;
    }
    if done >= MAX_PASSES {
        return Stop::CapReached;
    }
    Stop::Continue
}

/// The SHA-256 of the concatenated bytes of every file directly in `root`
/// with an auxiliary extension, taken in sorted file-name order.
/// Unreadable files and a missing directory hash as absent.
fn fingerprint(root: &Path) -> [u8; 32] {
    let mut names: Vec<std::ffi::OsString> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(root) {
        for entry in entries.flatten() {
            let is_aux = entry.path().is_file()
                && entry
                    .path()
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .is_some_and(|ext| AUX_EXTENSIONS.contains(&ext));
            if is_aux {
                names.push(entry.file_name());
            }
        }
    }
    names.sort();
    let mut hasher = Sha256::new();
    for name in &names {
        if let Ok(bytes) = std::fs::read(root.join(name)) {
            hasher.update(&bytes);
        }
    }
    hasher.finalize().into()
}

/// Whether a failed first pass means this Tectonic knows neither `--reruns`
/// nor `--outfmt` and the build must fall back to a single process.
fn fallback_needed(output: &Output) -> bool {
    let stderr = String::from_utf8_lossy(&output.stderr);
    stderr.contains("unexpected argument") || stderr.contains("unrecognized")
}

/// The one warning line printed before a fallback build.
fn fallback_warning() -> &'static str {
    "warning: this tectonic does not support one process per pass (--reruns/--outfmt); \
     building in a single process, so line heights can be wrong in documents that need \
     more than one pass (tectonic#1412)"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_file(root: &Path, name: &str, content: &[u8]) {
        std::fs::write(root.join(name), content).unwrap();
    }

    #[test]
    fn fingerprint_is_stable_for_the_same_files() {
        let dir = tempfile::tempdir().unwrap();
        write_file(dir.path(), "main.aux", b"\\relax\n");
        write_file(dir.path(), "main.toc", b"contents\n");
        assert_eq!(fingerprint(dir.path()), fingerprint(dir.path()));
    }

    #[test]
    fn fingerprint_changes_when_an_aux_file_changes() {
        let dir = tempfile::tempdir().unwrap();
        write_file(dir.path(), "main.aux", b"\\relax\n");
        let before = fingerprint(dir.path());
        write_file(dir.path(), "main.aux", b"\\relax\n\\newlabel{x}{{1}{2}}\n");
        assert_ne!(before, fingerprint(dir.path()));
    }

    #[test]
    fn fingerprint_ignores_source_and_output_files() {
        let dir = tempfile::tempdir().unwrap();
        write_file(dir.path(), "main.aux", b"\\relax\n");
        let before = fingerprint(dir.path());
        write_file(dir.path(), "main.tex", b"\\documentclass{article}\n");
        write_file(dir.path(), "main.pdf", b"%PDF-1.4 fake\n");
        write_file(dir.path(), "main.xdv", b"fake xdv\n");
        write_file(dir.path(), "main.log", b"fake log\n");
        assert_eq!(before, fingerprint(dir.path()));
    }

    #[test]
    fn fingerprint_reads_files_in_sorted_name_order() {
        let dir = tempfile::tempdir().unwrap();
        write_file(dir.path(), "b.aux", b"bee\n");
        write_file(dir.path(), "a.aux", b"ay\n");
        let mut expected = Sha256::new();
        expected.update(b"ay\n");
        expected.update(b"bee\n");
        let expected: [u8; 32] = expected.finalize().into();
        assert_eq!(fingerprint(dir.path()), expected);
    }

    #[test]
    fn should_stop_needs_two_passes_before_converging() {
        let hash = [7u8; 32];
        assert_eq!(should_stop(1, None, hash), Stop::Continue);
        assert_eq!(should_stop(1, Some(hash), hash), Stop::Continue);
    }

    #[test]
    fn should_stop_converges_on_equal_hashes_from_pass_two() {
        let hash = [7u8; 32];
        assert_eq!(should_stop(2, Some(hash), hash), Stop::Converged);
        assert_eq!(should_stop(4, Some(hash), hash), Stop::Converged);
    }

    #[test]
    fn should_stop_continues_while_hashes_change() {
        assert_eq!(should_stop(2, Some([7u8; 32]), [8u8; 32]), Stop::Continue);
        assert_eq!(should_stop(5, Some([7u8; 32]), [8u8; 32]), Stop::Continue);
    }

    #[test]
    fn should_stop_caps_at_six_passes_but_converges_when_settled() {
        assert_eq!(should_stop(6, Some([7u8; 32]), [8u8; 32]), Stop::CapReached);
        assert_eq!(should_stop(6, Some([7u8; 32]), [7u8; 32]), Stop::Converged);
    }

    #[test]
    fn fallback_warning_is_the_single_documented_line() {
        assert_eq!(
            fallback_warning(),
            "warning: this tectonic does not support one process per pass (--reruns/--outfmt); \
             building in a single process, so line heights can be wrong in documents that need \
             more than one pass (tectonic#1412)"
        );
    }

    #[cfg(unix)]
    #[test]
    fn fallback_triggers_on_unknown_flags_only() {
        let failing = |stderr: &[u8]| Output {
            status: std::os::unix::process::ExitStatusExt::from_raw(2 << 8),
            stdout: Vec::new(),
            stderr: stderr.to_vec(),
        };
        assert!(fallback_needed(&failing(
            b"error: unexpected argument '--reruns' found"
        )));
        assert!(fallback_needed(&failing(b"unrecognized option '--outfmt'")));
        assert!(!fallback_needed(&failing(b"! Undefined control sequence.")));
    }

    /// With a Tectonic that knows neither `--reruns` nor `--outfmt`, the
    /// first intermediate pass fails and `run_passes` runs the old single
    /// call exactly once instead. Unix only: the double is a shell script.
    #[cfg(unix)]
    #[test]
    fn old_tectonic_falls_back_to_one_single_call() {
        use std::os::unix::fs::PermissionsExt;

        let _env = crate::test_sync::ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _guard = crate::test_sync::EnvGuard::capture(&["TEXFORGE_SINGLE_PROCESS"]);
        std::env::remove_var("TEXFORGE_SINGLE_PROCESS");

        let dir = tempfile::tempdir().unwrap();
        let count = dir.path().join("calls");
        std::fs::write(&count, b"").unwrap();
        let fake = dir.path().join("tectonic");
        std::fs::write(
            &fake,
            format!(
                "#!/bin/sh\n\
                 echo \"$@\" >> \"{}\"\n\
                 case \"$*\" in\n\
                 *--reruns*)\n\
                 echo \"error: unexpected argument '--reruns' found\" >&2\n\
                 exit 2 ;;\n\
                 esac\n\
                 out=\"${{1%.*}}.pdf\"\n\
                 : > \"$out\"\n\
                 exit 0\n",
                count.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();

        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("main.tex"), b"fake source\n").unwrap();
        let output = run_passes(&fake, root.path(), "main.tex", None).unwrap();

        assert!(output.status.success(), "the fallback call must succeed");
        let calls = std::fs::read_to_string(&count).unwrap();
        let calls: Vec<&str> = calls.lines().collect();
        assert_eq!(
            calls.len(),
            2,
            "one intermediate plus one fallback: {calls:?}"
        );
        assert!(
            calls[0].contains("--reruns"),
            "the first call is the intermediate one: {calls:?}"
        );
        assert!(
            !calls[1].contains("--reruns") && !calls[1].contains("--outfmt"),
            "the fallback is the old single call: {calls:?}"
        );
        assert!(
            root.path().join("main.pdf").exists(),
            "the fallback writes the PDF like the old call"
        );
    }

    /// With `TEXFORGE_SINGLE_PROCESS` set, `run_passes` skips the loop and
    /// runs the old single call exactly once: one call means neither the
    /// cap warning nor the fallback warning could have been printed.
    /// Unix only: the double is a shell script.
    #[cfg(unix)]
    #[test]
    fn single_process_env_skips_the_loop() {
        use std::os::unix::fs::PermissionsExt;

        let _env = crate::test_sync::ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _guard = crate::test_sync::EnvGuard::capture(&["TEXFORGE_SINGLE_PROCESS"]);
        std::env::set_var("TEXFORGE_SINGLE_PROCESS", "1");

        let dir = tempfile::tempdir().unwrap();
        let count = dir.path().join("calls");
        std::fs::write(&count, b"").unwrap();
        let fake = dir.path().join("tectonic");
        std::fs::write(
            &fake,
            format!(
                "#!/bin/sh\n\
                 echo \"$@\" >> \"{}\"\n\
                 out=\"${{1%.*}}.pdf\"\n\
                 : > \"$out\"\n\
                 exit 0\n",
                count.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();

        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("main.tex"), b"fake source\n").unwrap();
        let output = run_passes(&fake, root.path(), "main.tex", None).unwrap();

        assert!(output.status.success(), "the single call must succeed");
        let calls = std::fs::read_to_string(&count).unwrap();
        let calls: Vec<&str> = calls.lines().collect();
        assert_eq!(
            calls.len(),
            1,
            "the old single call runs once, with no loop: {calls:?}"
        );
        assert!(
            !calls[0].contains("--reruns") && !calls[0].contains("--outfmt"),
            "no intermediate flags: {calls:?}"
        );
    }
}
