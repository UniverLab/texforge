//! GitHub releases version checker for auto-update detection.
//!
//! Queries the GitHub API to detect new stable versions and compare with local version.

#![allow(dead_code)]

use crate::version::SemVer;
use anyhow::{anyhow, Context, Result};
use serde::Deserialize;

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
    let arch = get_architecture();
    let _os = get_os();
    let filename = format!("{}-{}-{}", repo, version, arch);

    format!(
        "https://github.com/{}/{}/releases/download/v{}/{}",
        owner, repo, version, filename
    )
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
}
