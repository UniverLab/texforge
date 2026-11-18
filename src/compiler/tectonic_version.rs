//! Managed Tectonic version helpers: querying, comparing against the
//! pinned version, and refreshing an outdated managed copy.

use std::path::Path;
use std::process::Command;

/// Run `tectonic --version` and pull the version number out of its output.
pub(crate) fn query_tectonic_version(path: &Path) -> Option<String> {
    let output = Command::new(path).arg("--version").output().ok()?;
    if !output.status.success() {
        return None;
    }
    let raw = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    parse_version(&raw)
}

/// Pull the first `X.Y.Z`-shaped token out of arbitrary CLI output.
pub(crate) fn parse_version(raw: &str) -> Option<String> {
    raw.split(|c: char| !(c.is_ascii_digit() || c == '.'))
        .find(|tok| tok.matches('.').count() >= 2 && tok.starts_with(|c: char| c.is_ascii_digit()))
        .map(str::to_string)
}

/// True only when both versions parse and `found` is numerically older
/// than `pinned` (so `0.9.0` is older than `0.17.0`). Any parse failure
/// is false.
pub(crate) fn is_older(found: &str, pinned: &str) -> bool {
    match (
        crate::version::SemVer::parse(found),
        crate::version::SemVer::parse(pinned),
    ) {
        (Some(f), Some(p)) => f < p,
        _ => false,
    }
}

/// Outcome of attempting to refresh the managed Tectonic copy.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Refresh {
    Absent,
    Current,
    Updated { from: String },
    Failed { from: String, error: String },
}

/// Refresh the managed Tectonic copy when it is older than `pinned`.
///
/// A missing copy is `Absent` (`install` is not called); an unreadable
/// version or one no older than `pinned` is `Current`. An older copy
/// triggers `install` into a temporary file next to `managed`, renamed
/// over it only on success. A failure leaves `managed` untouched and
/// never stops the build.
pub(crate) fn refresh_managed_if_outdated(
    managed: &Path,
    pinned: &str,
    install: &dyn Fn(&Path) -> anyhow::Result<()>,
) -> Refresh {
    if !managed.exists() {
        return Refresh::Absent;
    }
    let Some(from) = query_tectonic_version(managed) else {
        return Refresh::Current;
    };
    if !is_older(&from, pinned) {
        return Refresh::Current;
    }
    eprintln!("Updating the managed Tectonic {from} to {pinned}...");
    let mut tmp_os = managed.as_os_str().to_owned();
    tmp_os.push(".new");
    let tmp = Path::new(&tmp_os).to_path_buf();
    match install(&tmp) {
        Ok(()) => match std::fs::rename(&tmp, managed) {
            Ok(()) => Refresh::Updated { from },
            Err(e) => {
                let _ = std::fs::remove_file(&tmp);
                refresh_failed(from, e.to_string())
            }
        },
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            refresh_failed(from, e.to_string())
        }
    }
}

/// Record a failed refresh: warn on stderr and report `Failed`.
/// The managed copy is left untouched; a failure never stops the build.
fn refresh_failed(from: String, error: String) -> Refresh {
    eprintln!("warning: could not update the managed Tectonic ({error}); continuing with {from}");
    Refresh::Failed { from, error }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_version_extracts_simple_semver() {
        assert_eq!(parse_version("tectonic 0.15.0\n"), Some("0.15.0".into()));
    }

    #[test]
    fn parse_version_handles_concatenated_banner_output() {
        // Tectonic's actual `--version` output has no separator between the
        // clap-generated line and its own banner.
        assert_eq!(
            parse_version("tectonic 0.15.0Tectonic 0.15.0"),
            Some("0.15.0".into())
        );
    }

    #[test]
    fn parse_version_none_when_absent() {
        assert_eq!(parse_version("no version info here"), None);
    }

    #[test]
    fn is_older_older_patch() {
        assert!(is_older("0.15.0", "0.17.0"));
    }

    #[test]
    fn is_older_same_version_is_not_older() {
        assert!(!is_older("0.17.0", "0.17.0"));
    }

    #[test]
    fn is_older_newer_patch_is_not_older() {
        assert!(!is_older("0.17.1", "0.17.0"));
    }

    #[test]
    fn is_older_numeric_minor_comparison() {
        assert!(is_older("0.9.0", "0.17.0"));
    }

    #[test]
    fn is_older_newer_major_is_not_older() {
        assert!(!is_older("1.0.0", "0.17.0"));
    }

    #[test]
    fn is_older_garbage_is_not_older() {
        assert!(!is_older("garbage", "0.17.0"));
    }

    #[cfg(unix)]
    fn write_fake_managed(dir: &Path, version: &str) -> std::path::PathBuf {
        let managed = dir.join("tectonic");
        std::fs::write(&managed, format!("#!/bin/sh\necho tectonic {version}\n")).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&managed, std::fs::Permissions::from_mode(0o755)).unwrap();
        managed
    }

    #[cfg(unix)]
    fn tmp_path_for(managed: &Path) -> std::path::PathBuf {
        let mut tmp_os = managed.as_os_str().to_owned();
        tmp_os.push(".new");
        std::path::PathBuf::from(tmp_os)
    }

    #[cfg(unix)]
    #[test]
    fn refresh_replaces_outdated_copy() {
        let tmp = tempfile::tempdir().unwrap();
        let managed = write_fake_managed(tmp.path(), "0.15.0");
        let result = refresh_managed_if_outdated(&managed, "0.17.0", &|dest| {
            std::fs::write(dest, b"new-bytes").unwrap();
            Ok(())
        });
        assert_eq!(
            result,
            Refresh::Updated {
                from: "0.15.0".to_string()
            }
        );
        assert_eq!(std::fs::read(&managed).unwrap(), b"new-bytes");
        assert!(!tmp_path_for(&managed).exists());
    }

    #[cfg(unix)]
    #[test]
    fn refresh_failure_keeps_original() {
        let tmp = tempfile::tempdir().unwrap();
        let managed = write_fake_managed(tmp.path(), "0.15.0");
        let before = std::fs::read(&managed).unwrap();
        let result = refresh_managed_if_outdated(&managed, "0.17.0", &|dest| {
            std::fs::write(dest, b"partial").unwrap();
            anyhow::bail!("boom")
        });
        match &result {
            Refresh::Failed { from, .. } => assert_eq!(from, "0.15.0"),
            other => panic!("expected Failed, got {other:?}"),
        }
        assert_eq!(std::fs::read(&managed).unwrap(), before);
        assert!(!tmp_path_for(&managed).exists());
    }

    #[cfg(unix)]
    #[test]
    fn refresh_rename_failure_removes_temp_and_reports_failed() {
        let tmp = tempfile::tempdir().unwrap();
        let managed = write_fake_managed(tmp.path(), "0.15.0");
        let result = refresh_managed_if_outdated(&managed, "0.17.0", &|dest| {
            std::fs::write(dest, b"new-bytes").unwrap();
            // Force the rename to fail: a file cannot be renamed over a directory.
            std::fs::remove_file(&managed).unwrap();
            std::fs::create_dir(&managed).unwrap();
            Ok(())
        });
        match &result {
            Refresh::Failed { from, .. } => assert_eq!(from, "0.15.0"),
            other => panic!("expected Failed, got {other:?}"),
        }
        assert!(managed.is_dir());
        assert!(!tmp_path_for(&managed).exists());
    }

    #[cfg(unix)]
    #[test]
    fn refresh_current_version_skips_installer() {
        let tmp = tempfile::tempdir().unwrap();
        let managed = write_fake_managed(tmp.path(), "0.17.0");
        let called = std::cell::Cell::new(false);
        let result = refresh_managed_if_outdated(&managed, "0.17.0", &|_dest| {
            called.set(true);
            Ok(())
        });
        assert_eq!(result, Refresh::Current);
        assert!(!called.get());
    }

    #[cfg(unix)]
    #[test]
    fn refresh_missing_file_skips_installer() {
        let tmp = tempfile::tempdir().unwrap();
        let managed = tmp.path().join("tectonic");
        let called = std::cell::Cell::new(false);
        let result = refresh_managed_if_outdated(&managed, "0.17.0", &|_dest| {
            called.set(true);
            Ok(())
        });
        assert_eq!(result, Refresh::Absent);
        assert!(!called.get());
    }
}
