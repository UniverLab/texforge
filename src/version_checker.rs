//! GitHub releases version checker for auto-update detection.
//!
//! Queries the GitHub API to detect new stable versions and compare with local version.

#![allow(dead_code)]

use crate::version::SemVer;
use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use std::io::Read;
use std::path::{Path, PathBuf};

/// GitHub API release response (minimal fields)
#[derive(Debug, Deserialize)]
struct GitHubRelease {
    tag_name: String,
    prerelease: bool,
    draft: bool,
}

/// Version check result
#[derive(Debug, Clone)]
pub struct VersionCheckResult {
    pub local_version: SemVer,
    pub latest_stable: Option<SemVer>,
    pub update_available: bool,
}

/// Check for newer stable versions on GitHub
pub fn check_for_updates(owner: &str, repo: &str) -> Result<VersionCheckResult> {
    let local = get_local_version()?;

    // Query GitHub API for releases
    let latest_stable = fetch_latest_stable_release(owner, repo)?;

    let update_available = if let Some(ref remote) = latest_stable {
        remote > &local
    } else {
        false
    };

    Ok(VersionCheckResult {
        local_version: local,
        latest_stable,
        update_available,
    })
}

/// Get the current texforge version (from Cargo.toml at compile time)
pub fn get_local_version() -> Result<SemVer> {
    let version_str = env!("CARGO_PKG_VERSION");
    SemVer::parse(version_str)
        .ok_or_else(|| anyhow!("Failed to parse local version: {}", version_str))
}

/// Fetch latest stable release from GitHub
/// Filters out pre-releases and drafts
fn fetch_latest_stable_release(owner: &str, repo: &str) -> Result<Option<SemVer>> {
    let url = format!("https://api.github.com/repos/{}/{}/releases", owner, repo);

    let client = reqwest::blocking::Client::new();
    let response = client
        .get(&url)
        .header("User-Agent", "texforge")
        .send()
        .context("Failed to query GitHub API")?;

    if !response.status().is_success() {
        return Err(anyhow!(
            "GitHub API returned status {}: {}",
            response.status(),
            response.text().unwrap_or_default()
        ));
    }

    let releases: Vec<GitHubRelease> = response
        .json()
        .context("Failed to parse GitHub releases JSON")?;

    // Find the latest stable version (skip pre-releases and drafts)
    for release in releases {
        if !release.draft && !release.prerelease {
            // Remove 'v' prefix if present
            let tag = release.tag_name.trim_start_matches('v');
            if let Some(version) = SemVer::parse(tag) {
                return Ok(Some(version));
            }
        }
    }

    Ok(None)
}

/// Get the download URL for a specific release
pub fn get_release_download_url(owner: &str, repo: &str, version: &SemVer) -> String {
    // The release workflow names its assets `<repo>-v<version>-<target>.<ext>`,
    // e.g. `texforge-v0.7.0-x86_64-unknown-linux-musl.tar.gz`. Three things
    // have to line up or the URL 404s: the `v` before the version, the FULL
    // target triple (not the bare architecture), and the archive extension.
    // Verified against the published assets of v0.7.0 on 2026-08-10.
    let (target, ext) = release_target();
    let filename = format!("{repo}-v{version}-{target}.{ext}");

    format!(
        "https://github.com/{}/{}/releases/download/v{}/{}",
        owner, repo, version, filename
    )
}

/// The rust target triple the release workflow builds for this platform, and
/// the archive extension it uses. Kept next to `get_architecture`/`get_os`
/// because those two answer a different question — what machine we are on —
/// and neither is enough to name a release asset on its own.
fn release_target() -> (&'static str, &'static str) {
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    return ("x86_64-unknown-linux-musl", "tar.gz");
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    return ("aarch64-unknown-linux-musl", "tar.gz");
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    return ("x86_64-apple-darwin", "tar.gz");
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    return ("aarch64-apple-darwin", "tar.gz");
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    return ("x86_64-pc-windows-msvc", "zip");
    #[cfg(not(any(
        all(target_os = "linux", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "aarch64"),
        all(target_os = "macos", target_arch = "x86_64"),
        all(target_os = "macos", target_arch = "aarch64"),
        all(target_os = "windows", target_arch = "x86_64"),
    )))]
    return ("unknown", "tar.gz");
}

fn get_architecture() -> &'static str {
    #[cfg(target_arch = "x86_64")]
    return "x86_64";
    #[cfg(target_arch = "aarch64")]
    return "aarch64";
    #[cfg(target_arch = "arm")]
    return "arm";
}

fn get_os() -> &'static str {
    #[cfg(target_os = "linux")]
    {
        "linux"
    }
    #[cfg(target_os = "macos")]
    {
        "macos"
    }
    #[cfg(target_os = "windows")]
    {
        "windows"
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        "unknown"
    }
}

// ── Cargo-managed install detection ────────────────────────────────
//
// A self-updater that writes to a hardcoded directory creates a second
// binary whenever the user installed with `cargo install` instead — and
// which copy actually runs then depends on PATH order. Refusing to touch a
// cargo-managed binary avoids that: cargo keeps its own metadata about what
// it manages at that path, and overwriting the file behind its back leaves
// cargo believing it still owns a binary it no longer produced.

/// The environment inputs used to resolve where `cargo install` places
/// binaries. Kept as a struct (rather than reading `std::env` directly)
/// so the precedence rules can be tested without mutating process-global
/// environment variables.
struct CargoRootEnv {
    install_root: Option<String>,
    cargo_home: Option<String>,
    home: Option<PathBuf>,
}

/// `CARGO_INSTALL_ROOT` wins over `CARGO_HOME`, which wins over `~/.cargo` —
/// the same precedence cargo itself uses to decide where `cargo install`
/// places binaries.
fn resolve_cargo_bin_dir(env: &CargoRootEnv) -> Option<PathBuf> {
    if let Some(root) = &env.install_root {
        return Some(PathBuf::from(root).join("bin"));
    }
    if let Some(home) = &env.cargo_home {
        return Some(PathBuf::from(home).join("bin"));
    }
    env.home.as_ref().map(|h| h.join(".cargo").join("bin"))
}

/// True if `exe_path` resolves inside the cargo install root.
///
/// Both sides are canonicalised before comparing, since `exe_path` (from
/// `std::env::current_exe`) can be a symlink. A path that fails to
/// canonicalise — missing, broken symlink, permission denied — is treated
/// as "not cargo": a false positive here blocks a legitimate update, which
/// is worse than the false negative of overwriting the same file the user
/// was already running.
fn is_cargo_managed(exe_path: &Path, env: &CargoRootEnv) -> bool {
    let Some(cargo_bin) = resolve_cargo_bin_dir(env) else {
        return false;
    };
    let Ok(canon_exe) = exe_path.canonicalize() else {
        return false;
    };
    let Ok(canon_cargo_bin) = cargo_bin.canonicalize() else {
        return false;
    };
    canon_exe.starts_with(&canon_cargo_bin)
}

/// True if the currently running binary was installed with `cargo install`.
pub fn current_exe_is_cargo_managed(exe_path: &Path) -> bool {
    let env = CargoRootEnv {
        install_root: std::env::var("CARGO_INSTALL_ROOT")
            .ok()
            .filter(|s| !s.is_empty()),
        cargo_home: std::env::var("CARGO_HOME").ok().filter(|s| !s.is_empty()),
        home: dirs::home_dir(),
    };
    is_cargo_managed(exe_path, &env)
}

// ── Download + install ──────────────────────────────────────────────

/// Extract the binary named `bin_name` from a `.tar.gz` archive read from
/// `reader`, writing it to `dest`. Rejects a missing or empty entry so a
/// truncated or corrupt download never reaches the replace step.
fn extract_binary_from_tar_gz<R: Read>(reader: R, bin_name: &str, dest: &Path) -> Result<()> {
    let decoder = flate2::read::GzDecoder::new(reader);
    let mut archive = tar::Archive::new(decoder);

    for entry in archive.entries().context("failed to read archive")? {
        let mut entry = entry.context("failed to read archive entry")?;
        let path = entry.path().context("failed to read archive entry path")?;
        if path.file_name().is_some_and(|n| n == bin_name) {
            entry
                .unpack(dest)
                .context("failed to extract binary from archive")?;
            let size = std::fs::metadata(dest)
                .context("failed to read extracted binary metadata")?
                .len();
            if size == 0 {
                anyhow::bail!("extracted binary '{bin_name}' is empty");
            }
            return Ok(());
        }
    }

    anyhow::bail!("binary '{bin_name}' not found in downloaded archive")
}

/// Move `new_bin` into `current_exe`'s place. Renames when possible (fast,
/// same-filesystem, and keeps the source's file mode); falls back to copy
/// for the cross-filesystem case.
fn replace_binary(new_bin: &Path, current_exe: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(new_bin, std::fs::Permissions::from_mode(0o755))
            .context("failed to mark downloaded binary as executable")?;
    }

    if std::fs::rename(new_bin, current_exe).is_err() {
        std::fs::copy(new_bin, current_exe).with_context(|| {
            format!(
                "failed to replace binary at {} (check that you have write permission to this location)",
                current_exe.display()
            )
        })?;
    }

    Ok(())
}

/// Extract the binary named `bin_name` from the archive read from `reader`
/// and atomically replace `current_exe` with it.
///
/// The archive is extracted to a temporary file beside `current_exe` (same
/// directory, so the common case is a same-filesystem rename) and verified
/// before anything is replaced. On any failure the temporary file is
/// removed and `current_exe` is left untouched.
fn replace_from_reader<R: Read>(reader: R, bin_name: &str, current_exe: &Path) -> Result<()> {
    let parent = current_exe.parent().unwrap_or_else(|| Path::new("."));
    let tmp = tempfile::Builder::new()
        .prefix(".texforge-update-")
        .tempfile_in(parent)
        .context("failed to create temporary file next to the running binary")?
        .into_temp_path();

    extract_binary_from_tar_gz(reader, bin_name, &tmp)?;
    replace_binary(&tmp, current_exe)?;

    Ok(())
}

/// Download the release archive for the running platform, verify it, and
/// atomically replace `current_exe` with it.
///
/// Callers are expected to check `current_exe_is_cargo_managed` first —
/// this function always downloads and writes when called.
pub fn download_and_replace(
    owner: &str,
    repo: &str,
    version: &SemVer,
    current_exe: &Path,
) -> Result<()> {
    let url = get_release_download_url(owner, repo, version);

    let response = reqwest::blocking::Client::new()
        .get(&url)
        .header("User-Agent", repo)
        .send()
        .context("failed to download update")?;

    if !response.status().is_success() {
        anyhow::bail!("download failed: HTTP {}", response.status());
    }

    replace_from_reader(response, repo, current_exe)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_local_version() {
        let version = get_local_version().unwrap();
        assert!(version.major > 0 || version.minor > 0 || version.patch > 0);
    }

    #[test]
    fn test_get_release_download_url() {
        let version = SemVer::parse("1.2.3").unwrap();
        let url = get_release_download_url("UniverLab", "texforge", &version);
        assert!(url.contains("github.com"));
        assert!(url.contains("UniverLab"));
        assert!(url.contains("texforge"));
        assert!(url.contains("1.2.3"));
    }

    #[test]
    fn test_get_architecture() {
        let arch = get_architecture();
        assert!(!arch.is_empty());
        assert!(arch == "x86_64" || arch == "aarch64" || arch == "arm");
    }

    #[test]
    fn test_get_os() {
        let os = get_os();
        assert!(!os.is_empty());
        assert!(os == "linux" || os == "macos" || os == "windows" || os == "unknown");
    }

    #[test]
    fn test_version_check_result_struct() {
        let local = SemVer::parse("1.0.0").unwrap();
        let latest = SemVer::parse("2.0.0").unwrap();
        let result = VersionCheckResult {
            local_version: local.clone(),
            latest_stable: Some(latest.clone()),
            update_available: true,
        };
        assert_eq!(result.local_version, local);
        assert_eq!(result.latest_stable, Some(latest));
        assert!(result.update_available);
    }

    #[test]
    fn test_version_check_no_update() {
        let local = SemVer::parse("2.0.0").unwrap();
        let latest = SemVer::parse("1.0.0").unwrap();
        let result = VersionCheckResult {
            local_version: local,
            latest_stable: Some(latest),
            update_available: false,
        };
        assert!(!result.update_available);
    }

    #[test]
    fn test_version_check_no_latest() {
        let local = SemVer::parse("1.0.0").unwrap();
        let result = VersionCheckResult {
            local_version: local,
            latest_stable: None,
            update_available: false,
        };
        assert!(!result.update_available);
        assert!(result.latest_stable.is_none());
    }

    #[test]
    fn test_get_release_download_url_contains_arch() {
        let version = SemVer::parse("1.0.0").unwrap();
        let url = get_release_download_url("owner", "repo", &version);
        let arch = get_architecture();
        assert!(url.contains(arch));
    }

    /// The asset name has to match what the release workflow publishes,
    /// byte for byte, or the printed URL 404s. It did, until 2026-08-10.
    #[test]
    fn download_url_matches_the_published_asset_name() {
        let version = SemVer::parse("0.7.0").unwrap();
        let url = get_release_download_url("UniverLab", "texforge", &version);
        let (target, ext) = release_target();

        assert!(
            url.ends_with(&format!("/texforge-v0.7.0-{target}.{ext}")),
            "asset name drifted from the release workflow: {url}"
        );
        assert!(
            url.contains("/releases/download/v0.7.0/"),
            "tag path lost its v prefix: {url}"
        );
    }

    #[test]
    fn test_get_release_download_url_format() {
        let version = SemVer::parse("2.5.1").unwrap();
        let url = get_release_download_url("UniverLab", "texforge", &version);
        assert!(url.starts_with("https://github.com/"));
        assert!(url.contains("v2.5.1"));
        assert!(url.contains("texforge"));
    }

    #[test]
    fn test_get_local_version_is_stable() {
        let version = get_local_version().unwrap();
        // CARGO_PKG_VERSION should be a stable release (no prerelease)
        assert!(version.is_stable());
    }

    #[test]
    fn test_get_architecture_nonempty() {
        let arch = get_architecture();
        assert!(!arch.is_empty());
    }

    #[test]
    fn test_get_os_nonempty() {
        let os = get_os();
        assert!(!os.is_empty());
    }

    #[test]
    fn test_version_check_result_debug() {
        let local = SemVer::parse("1.0.0").unwrap();
        let result = VersionCheckResult {
            local_version: local,
            latest_stable: None,
            update_available: false,
        };
        let debug = format!("{:?}", result);
        assert!(debug.contains("VersionCheckResult"));
    }

    #[test]
    fn test_version_check_result_clone() {
        let local = SemVer::parse("1.0.0").unwrap();
        let result = VersionCheckResult {
            local_version: local,
            latest_stable: None,
            update_available: false,
        };
        let cloned = result.clone();
        assert_eq!(result.local_version, cloned.local_version);
        assert_eq!(result.update_available, cloned.update_available);
    }

    // ── Cargo-managed install detection ──────────────────────────

    #[test]
    fn install_root_env_wins_over_cargo_home_and_default_home() {
        let env = CargoRootEnv {
            install_root: Some("/install-root".to_string()),
            cargo_home: Some("/cargo-home".to_string()),
            home: Some(PathBuf::from("/home/user")),
        };
        assert_eq!(
            resolve_cargo_bin_dir(&env),
            Some(PathBuf::from("/install-root/bin"))
        );
    }

    #[test]
    fn cargo_home_env_wins_over_default_home() {
        let env = CargoRootEnv {
            install_root: None,
            cargo_home: Some("/cargo-home".to_string()),
            home: Some(PathBuf::from("/home/user")),
        };
        assert_eq!(
            resolve_cargo_bin_dir(&env),
            Some(PathBuf::from("/cargo-home/bin"))
        );
    }

    #[test]
    fn default_home_cargo_dir_used_when_no_env_vars_set() {
        let env = CargoRootEnv {
            install_root: None,
            cargo_home: None,
            home: Some(PathBuf::from("/home/user")),
        };
        assert_eq!(
            resolve_cargo_bin_dir(&env),
            Some(PathBuf::from("/home/user/.cargo/bin"))
        );
    }

    #[test]
    fn path_inside_cargo_root_is_detected_as_cargo_managed() {
        let tmp = tempfile::tempdir().unwrap();
        let cargo_root = tmp.path().join("cargo-home");
        let bin_dir = cargo_root.join("bin");
        std::fs::create_dir_all(&bin_dir).unwrap();
        let exe = bin_dir.join("texforge");
        std::fs::write(&exe, b"fake").unwrap();

        let env = CargoRootEnv {
            install_root: None,
            cargo_home: Some(cargo_root.to_string_lossy().to_string()),
            home: None,
        };
        assert!(is_cargo_managed(&exe, &env));
    }

    #[test]
    fn path_outside_cargo_root_is_not_cargo_managed() {
        let tmp = tempfile::tempdir().unwrap();
        let cargo_root = tmp.path().join("cargo-home");
        std::fs::create_dir_all(cargo_root.join("bin")).unwrap();
        let elsewhere = tmp.path().join("elsewhere");
        std::fs::create_dir_all(&elsewhere).unwrap();
        let exe = elsewhere.join("texforge");
        std::fs::write(&exe, b"fake").unwrap();

        let env = CargoRootEnv {
            install_root: None,
            cargo_home: Some(cargo_root.to_string_lossy().to_string()),
            home: None,
        };
        assert!(!is_cargo_managed(&exe, &env));
    }

    #[test]
    fn uncanonicalizable_path_is_treated_as_not_cargo_managed() {
        let env = CargoRootEnv {
            install_root: None,
            cargo_home: Some("/nonexistent-cargo-home-xyz-texforge-test".to_string()),
            home: None,
        };
        let missing_exe = Path::new("/nonexistent-path-xyz-texforge-test/texforge");
        assert!(!is_cargo_managed(missing_exe, &env));
    }

    // ── Download + install ───────────────────────────────────────

    /// Build a `.tar.gz` archive in memory containing the given entries.
    fn build_tar_gz(entries: &[(&str, &[u8])]) -> Vec<u8> {
        use std::io::Write;

        let mut tar_data = Vec::new();
        {
            let mut builder = tar::Builder::new(&mut tar_data);
            for (name, content) in entries {
                let mut header = tar::Header::new_gnu();
                header.set_size(content.len() as u64);
                header.set_mode(0o755);
                header.set_cksum();
                builder.append_data(&mut header, name, *content).unwrap();
            }
            builder.finish().unwrap();
        }

        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(&tar_data).unwrap();
        encoder.finish().unwrap()
    }

    #[test]
    fn valid_archive_replaces_binary_and_sets_executable() {
        let tmp = tempfile::tempdir().unwrap();
        let current = tmp.path().join("texforge");
        std::fs::write(&current, b"old binary content").unwrap();

        let archive = build_tar_gz(&[("texforge", b"new binary content")]);
        replace_from_reader(std::io::Cursor::new(archive), "texforge", &current).unwrap();

        assert_eq!(std::fs::read(&current).unwrap(), b"new binary content");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&current).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o755);
        }
    }

    #[test]
    fn missing_binary_entry_leaves_original_untouched() {
        let tmp = tempfile::tempdir().unwrap();
        let current = tmp.path().join("texforge");
        std::fs::write(&current, b"original content").unwrap();

        let archive = build_tar_gz(&[("readme.txt", b"not a binary")]);
        let result = replace_from_reader(std::io::Cursor::new(archive), "texforge", &current);

        assert!(result.is_err());
        assert_eq!(std::fs::read(&current).unwrap(), b"original content");
    }

    #[test]
    fn empty_binary_entry_is_rejected_and_original_untouched() {
        let tmp = tempfile::tempdir().unwrap();
        let current = tmp.path().join("texforge");
        std::fs::write(&current, b"original content").unwrap();

        let archive = build_tar_gz(&[("texforge", b"")]);
        let result = replace_from_reader(std::io::Cursor::new(archive), "texforge", &current);

        assert!(result.is_err());
        assert_eq!(std::fs::read(&current).unwrap(), b"original content");
    }

    #[test]
    fn temp_file_does_not_survive_a_failed_install() {
        let tmp = tempfile::tempdir().unwrap();
        let current = tmp.path().join("texforge");
        std::fs::write(&current, b"original content").unwrap();

        let archive = build_tar_gz(&[("readme.txt", b"not a binary")]);
        let result = replace_from_reader(std::io::Cursor::new(archive), "texforge", &current);
        assert!(result.is_err());

        let leftovers: Vec<_> = std::fs::read_dir(tmp.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with(".texforge-update-")
            })
            .collect();
        assert!(leftovers.is_empty(), "leftover temp files: {leftovers:?}");
    }

    #[test]
    fn replace_binary_in_tmp_dir_is_executable() {
        let tmp = tempfile::tempdir().unwrap();
        let new_bin = tmp.path().join("new-bin");
        std::fs::write(&new_bin, b"new content").unwrap();
        let current = tmp.path().join("current-bin");
        std::fs::write(&current, b"old content").unwrap();

        replace_binary(&new_bin, &current).unwrap();

        assert_eq!(std::fs::read(&current).unwrap(), b"new content");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&current).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o755);
        }
    }
}
