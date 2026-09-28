//! `texforge update` — the explicit self-update command and the shared
//! release check.
//!
//! Port of canopy 3.0.1's `autoupdate` core (`run_update` /
//! `run_update_with` / `UpdateDeps`): the command always asks first —
//! default **NO** — never updates silently, and refuses to replace a
//! cargo-managed install. Every external fact (release list, archive bytes,
//! prompt answer, executable path, target triple) is injected through
//! [`UpdateDeps`], so the unit tests run with zero network access.
//!
//! The passive notice `texforge init` shows lives here too, so there is one
//! owner of the release check. That path never downloads by itself and stays
//! quiet when the network is unavailable; only the explicit command fails
//! loudly, because silence there would read as "you are up to date".

use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use serde::Deserialize;

use crate::version::SemVer;

/// The GitHub repository whose stable releases contain texforge binaries.
pub const GITHUB_OWNER: &str = "UniverLab";
pub const GITHUB_REPO: &str = "texforge";

/// The exact remediation printed when the running binary lives below
/// `~/.cargo/bin`: cargo owns that file, so texforge refuses to replace it.
pub const CARGO_INSTALL_HINT: &str = "cargo install --force texforge";

/// Total request timeout for the release-list lookup. An explicit command
/// must fail loudly, but it must never hang either.
const RELEASE_TIMEOUT: Duration = Duration::from_secs(15);

/// Total request timeout for the archive download. A release binary is a few
/// MiB, so a slow link must not be mistaken for a dead one.
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(120);

/// The binary's name inside release archives for this platform: the Windows
/// asset is a `.zip` carrying `texforge.exe`, every other target a `.tar.gz`
/// carrying `texforge`.
#[cfg(windows)]
const BIN_NAME: &str = "texforge.exe";
#[cfg(not(windows))]
const BIN_NAME: &str = "texforge";

/// Daily-once throttle for the passive `init` notice. `texforge update` never
/// reads or writes this file: the explicit command always checks.
const CHECK_INTERVAL_SECS: u64 = 24 * 60 * 60;
const LAST_CHECK_FILE: &str = "last_update_check.txt";

// ── Release and transport seams ──────────────────────────────────

/// The release fields needed to select a stable, published binary.
#[derive(Deserialize, Debug, Clone, PartialEq)]
pub struct GitHubRelease {
    pub tag_name: String,
    #[serde(default)]
    pub prerelease: bool,
    #[serde(default)]
    pub draft: bool,
}

/// Injectable release-JSON lookup used by the update core and the notice.
pub trait ReleaseFetcher {
    fn get(&self, url: &str) -> Result<String>;
}

/// Production release lookup. All network and HTTP-status handling lives
/// behind [`ReleaseFetcher`] so unit tests can use a deterministic fake.
pub struct RealFetcher;

impl ReleaseFetcher for RealFetcher {
    fn get(&self, url: &str) -> Result<String> {
        let client = http_client(RELEASE_TIMEOUT)?;
        let response = client
            .get(url)
            .send()
            .context("failed to fetch GitHub releases")?;
        let status = response.status();
        if !status.is_success() {
            bail!("GitHub releases request failed: HTTP {status}");
        }
        response
            .text()
            .context("failed to read GitHub releases response")
    }
}

/// Injectable binary downloader. The archive is decoded only after this seam
/// returns, keeping the updater tests entirely offline.
pub trait BinaryDownloader {
    fn download(&self, url: &str) -> Result<Vec<u8>>;
}

/// Production binary downloader.
pub struct RealDownloader;

impl BinaryDownloader for RealDownloader {
    fn download(&self, url: &str) -> Result<Vec<u8>> {
        let client = http_client(DOWNLOAD_TIMEOUT)?;
        let response = client
            .get(url)
            .send()
            .with_context(|| format!("failed to download {url}"))?;
        let status = response.status();
        if !status.is_success() {
            bail!("download failed: HTTP {status}");
        }
        let bytes = response
            .bytes()
            .context("failed to read the downloaded archive")?;
        Ok(bytes.to_vec())
    }
}

fn http_client(timeout: Duration) -> Result<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .connect_timeout(timeout)
        .timeout(timeout)
        // The GitHub API rejects requests without a `User-Agent`.
        .user_agent("texforge")
        .build()
        .context("failed to build the HTTP client")
}

/// Dependencies for the hermetic update command path. The real command uses
/// the same flow with production I/O; tests provide every fallible external
/// fact here and never contact GitHub.
pub struct UpdateDeps<'a> {
    pub current: &'a SemVer,
    pub releases: Result<Vec<GitHubRelease>, String>,
    pub exe: &'a Path,
    pub cargo_bin: &'a Path,
    pub target: Result<(&'a str, &'a str), String>,
    pub downloader: &'a dyn BinaryDownloader,
    pub confirm: &'a dyn Fn() -> bool,
}

// ── Public update entry points ───────────────────────────────────

/// Check for and, after consent, install the latest stable release.
///
/// The returned integer is the process exit code: `0` means no update was
/// installed (already current, a declined prompt, or a cargo-managed
/// binary), and `1` is reserved for an available update in `--check` mode or
/// an archive without the texforge binary. Network and API errors surface as
/// `Err` — the explicit command fails loudly, unlike the silent notice path.
pub fn run_update(check: bool, yes: bool) -> Result<i32> {
    let current = get_local_version()?;
    let releases = fetch_releases_with(&RealFetcher)?;

    match select_latest_stable(&releases, &current) {
        // An actual install needs the executable and target facts. Resolve
        // them only now, so `--check` never touches a local path and keeps
        // working on platforms without release assets.
        Some(latest) if !check => {
            let exe =
                std::env::current_exe().context("failed to locate the texforge executable")?;
            let cargo_bin = cargo_bin_dir();
            let target = resolve_target()?;
            let deps = UpdateDeps {
                current: &current,
                releases: Ok(releases),
                exe: &exe,
                cargo_bin: &cargo_bin,
                target: Ok(target),
                downloader: &RealDownloader,
                confirm: &|| {
                    inquire::Confirm::new(&format!("Update to {latest}?"))
                        .with_default(false)
                        .prompt()
                        .unwrap_or(false)
                },
            };
            run_update_with(false, yes, &deps)
        }
        // No newer release, or a read-only `--check`: hand the network
        // result to the same hermetic core, which owns all user-visible
        // output, the cargo guard, and consent. The executable facts below
        // are placeholders: neither path reaches them (`--check` returns
        // before the cargo guard; "up to date" returns before the prompt),
        // so they must never name a real location.
        _ => {
            let deps = UpdateDeps {
                current: &current,
                releases: Ok(releases),
                exe: Path::new(""),
                cargo_bin: Path::new(""),
                target: Ok(release_target()),
                downloader: &RealDownloader,
                confirm: &|| false,
            };
            run_update_with(check, yes, &deps)
        }
    }
}

/// Hermetic update flow used by unit tests and embedders. It has no
/// network or filesystem setup step; callers provide those facts through
/// [`UpdateDeps`].
pub fn run_update_with(check: bool, yes: bool, deps: &UpdateDeps<'_>) -> Result<i32> {
    run_update_core(check, yes, deps)
}

/// The hermetic core: every user-visible line, the exit-code contract, the
/// cargo guard, and consent live here so `--check`, the explicit command,
/// and the unit tests all exercise the same flow.
fn run_update_core(check: bool, yes: bool, deps: &UpdateDeps<'_>) -> Result<i32> {
    let releases = deps
        .releases
        .as_ref()
        .map_err(|error| anyhow!("release lookup failed: {error}"))?;
    let current = deps.current;

    let Some(latest) = select_latest_stable(releases, current) else {
        println!("texforge {current} is up to date");
        return Ok(0);
    };
    println!("texforge {current} → {latest}");

    // `--check` ends here: exit 1 = update available, 0 = already current.
    // Nothing below this line — cargo guard, target, prompt, download — may
    // run in read-only mode.
    if check {
        return Ok(1);
    }

    if is_cargo_installed(deps.exe, deps.cargo_bin) {
        // cargo owns that file: a refusal with guidance, not a failure.
        println!("installed with cargo — run: {CARGO_INSTALL_HINT}");
        return Ok(0);
    }

    let target = deps
        .target
        .as_ref()
        .map_err(|error| anyhow!("target resolution failed: {error}"))?;
    let (target, ext) = *target;

    if !yes && !(deps.confirm)() {
        println!("Aborted.");
        return Ok(0);
    }

    let staging = tempfile::tempdir().context("failed to create update staging directory")?;
    let staged = staging.path().join("texforge-new");
    if !download_and_extract_with(deps.downloader, &latest, target, ext, &staged)? {
        eprintln!("  ✗ Binary not found in archive");
        return Ok(1);
    }

    replace_binary(&staged, deps.exe)?;
    println!("✓ updated to {latest}");
    Ok(0)
}

/// Read-only release lookup for the `texforge init` notice: `None` when
/// offline, when the API fails, or when no newer stable release exists. It
/// never downloads and never fails the command.
pub fn fetch_latest_stable_silent() -> Option<SemVer> {
    fetch_latest_stable_with(&RealFetcher)
}

/// Injectable form of [`fetch_latest_stable_silent`], so the "fail silently
/// on notice paths" contract is testable without a network.
pub fn fetch_latest_stable_with(fetcher: &dyn ReleaseFetcher) -> Option<SemVer> {
    let current = get_local_version().ok()?;
    let releases = fetch_releases_with(fetcher).ok()?;
    select_latest_stable(&releases, &current)
}

/// Download `latest` and atomically replace the running binary. Called by
/// the `init` notice only after the user said yes; it re-checks the cargo
/// guard because that guard must hold no matter which path asked.
///
/// Returns `Ok(false)` when the binary is cargo-managed (guidance printed,
/// nothing touched) and `Ok(true)` after a successful replacement.
pub fn install_after_confirm(latest: &SemVer) -> Result<bool> {
    let exe = std::env::current_exe().context("failed to resolve the running binary's path")?;
    let cargo_bin = cargo_bin_dir();
    if is_cargo_installed(&exe, &cargo_bin) {
        println!("\n  texforge was installed with cargo.");
        println!("  Run: {CARGO_INSTALL_HINT}\n");
        return Ok(false);
    }

    let (target, ext) = resolve_target()?;
    let staging = tempfile::tempdir().context("failed to create update staging directory")?;
    let staged = staging.path().join("texforge-new");
    if !download_and_extract_with(&RealDownloader, latest, target, ext, &staged)? {
        bail!("binary '{BIN_NAME}' not found in the downloaded archive");
    }
    replace_binary(&staged, &exe)?;
    Ok(true)
}

// ── Version helpers ──────────────────────────────────────────────

/// The version this binary was compiled from.
pub fn get_local_version() -> Result<SemVer> {
    let version_str = env!("CARGO_PKG_VERSION");
    SemVer::parse(version_str)
        .ok_or_else(|| anyhow!("failed to parse local version: {version_str}"))
}

/// The newest published stable release strictly newer than `current`.
/// Drafts, prereleases, and non-semver tags never win.
pub fn select_latest_stable(releases: &[GitHubRelease], current: &SemVer) -> Option<SemVer> {
    releases
        .iter()
        .filter(|release| !release.draft && !release.prerelease)
        .filter_map(|release| SemVer::parse(&release.tag_name))
        .filter(SemVer::is_stable)
        .filter(|version| version > current)
        .max()
}

// ── Release lookup ───────────────────────────────────────────────

/// Fetch the full release list (not `/releases/latest`) so drafts,
/// prereleases, and non-semver tags can be filtered in code.
fn fetch_releases_with(fetcher: &dyn ReleaseFetcher) -> Result<Vec<GitHubRelease>> {
    let url = format!("https://api.github.com/repos/{GITHUB_OWNER}/{GITHUB_REPO}/releases");
    let body = fetcher.get(&url)?;
    serde_json::from_str(&body).context("failed to parse releases JSON")
}

// ── Target and installation-path helpers ─────────────────────────

/// The rust target triple the release workflow builds for this platform, and
/// the archive extension it uses. The `unknown` fallback is what an
/// unsupported platform gets from [`release_target`]; [`resolve_target`]
/// refuses it before anything is downloaded.
pub fn release_target() -> (&'static str, &'static str) {
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

/// Same as [`release_target`], but refuses platforms the release workflow
/// does not publish for instead of naming an asset that would 404.
pub fn resolve_target() -> Result<(&'static str, &'static str)> {
    let (target, ext) = release_target();
    if target == "unknown" {
        bail!(
            "unsupported target: {}-{} (no release asset published for it)",
            std::env::consts::ARCH,
            std::env::consts::OS
        );
    }
    Ok((target, ext))
}

/// The exact release asset file name, byte-critical: `{repo}-v{version}-{target}.{ext}`.
pub fn asset_name(repo: &str, version: &SemVer, target: &str, ext: &str) -> String {
    format!("{repo}-v{version}-{target}.{ext}")
}

/// The download URL for a release asset. Three things have to line up or the
/// URL 404s: the `v` before the version, the FULL target triple (not the
/// bare architecture), and the archive extension. Verified against the
/// published assets of v0.7.0 on 2026-08-10.
pub fn get_release_download_url(
    owner: &str,
    repo: &str,
    version: &SemVer,
    target: &str,
    ext: &str,
) -> String {
    let asset = asset_name(repo, version, target, ext);
    format!("https://github.com/{owner}/{repo}/releases/download/v{version}/{asset}")
}

// ── Cargo-managed install detection ──────────────────────────────
//
// A self-updater that writes to a hardcoded directory creates a second
// binary whenever the user installed with `cargo install` instead — and
// which copy actually runs then depends on PATH order. Refusing to touch a
// cargo-managed binary avoids that: cargo keeps its own metadata about what
// it manages at that path, and overwriting the file behind its back leaves
// cargo believing it still owns a binary it no longer produced.

/// The environment inputs used to resolve where `cargo install` places
/// binaries. Kept as a struct (rather than reading `std::env` directly) so
/// the precedence rules can be tested without mutating process-global
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

/// The cargo bin directory selected by the environment, falling back to the
/// conventional `$HOME/.cargo/bin` location.
pub fn cargo_bin_dir() -> PathBuf {
    resolve_cargo_bin_dir(&CargoRootEnv {
        install_root: std::env::var("CARGO_INSTALL_ROOT")
            .ok()
            .filter(|s| !s.is_empty()),
        cargo_home: std::env::var("CARGO_HOME").ok().filter(|s| !s.is_empty()),
        home: dirs::home_dir(),
    })
    .unwrap_or_else(|| PathBuf::from(".cargo").join("bin"))
}

/// True if `exe` lives below the cargo bin directory, i.e. `cargo install`
/// produced it and cargo — not texforge — owns that file.
///
/// The executable side must canonicalise (it comes from `current_exe`, so it
/// exists); a path that fails to canonicalise — missing, broken symlink,
/// permission denied — is treated as "not cargo": a false positive here
/// blocks a legitimate update, which is worse than the false negative of
/// overwriting the file the user was already running. The cargo side falls
/// back to its raw path so non-existent directories still compare sanely.
pub fn is_cargo_installed(exe: &Path, cargo_bin: &Path) -> bool {
    let Ok(exe) = exe.canonicalize() else {
        return false;
    };
    let cargo_bin = cargo_bin
        .canonicalize()
        .unwrap_or_else(|_| cargo_bin.to_path_buf());
    exe.starts_with(&cargo_bin)
}

// ── Daily-once notice throttle ───────────────────────────────────

/// True when the passive `init` notice has no stamp, an unreadable one, or
/// one older than [`CHECK_INTERVAL_SECS`].
fn check_is_due(stamp: &Path, now: u64) -> bool {
    let Ok(content) = std::fs::read_to_string(stamp) else {
        return true;
    };
    let Ok(last) = content.trim().parse::<u64>() else {
        return true;
    };
    now.saturating_sub(last) >= CHECK_INTERVAL_SECS
}

/// Whether the passive `init` notice should run. Checked at most once a day;
/// `texforge update` never consults this.
pub fn should_check() -> bool {
    let Ok(dir) = crate::utils::data_dir() else {
        return true;
    };
    let Ok(now) = now_secs() else {
        return true;
    };
    check_is_due(&dir.join(LAST_CHECK_FILE), now)
}

/// Stamp the notice attempt. Recorded even when the lookup fails so a
/// disconnected machine does not retry on every `texforge init`.
pub fn record_check() {
    let Ok(dir) = crate::utils::data_dir() else {
        return;
    };
    let Ok(now) = now_secs() else {
        return;
    };
    let _ = std::fs::write(dir.join(LAST_CHECK_FILE), now.to_string());
}

fn now_secs() -> Result<u64> {
    Ok(std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .context("system clock error")?
        .as_secs())
}

// ── Download, verification, extraction, and atomic replacement ───

/// Download the release asset for `version`/`target`, verify it when the
/// release ships checksums, and unpack the binary into `output`.
/// Returns `Ok(false)` when the archive holds no such entry.
pub fn download_and_extract_with(
    downloader: &dyn BinaryDownloader,
    version: &SemVer,
    target: &str,
    ext: &str,
    output: &Path,
) -> Result<bool> {
    let asset = asset_name(GITHUB_REPO, version, target, ext);
    let url = get_release_download_url(GITHUB_OWNER, GITHUB_REPO, version, target, ext);
    let bytes = downloader.download(&url)?;
    verify_checksum_if_present(downloader, version, &asset, &bytes)?;

    match ext {
        "zip" => extract_binary_from_zip(&bytes, BIN_NAME, output),
        _ => extract_binary_from_tar_gz(bytes.as_slice(), BIN_NAME, output),
    }
}

/// Verify the archive against the release's `SHA256SUMS.txt` when the
/// release ships one. A missing checksum file (older releases) or a missing
/// line for this asset skips verification — deliberately more lenient than
/// `scripts/install.sh`, which errors on a missing line; a present-but-
/// mismatched digest is fatal.
fn verify_checksum_if_present(
    downloader: &dyn BinaryDownloader,
    version: &SemVer,
    asset: &str,
    bytes: &[u8],
) -> Result<()> {
    let url = format!(
        "https://github.com/{GITHUB_OWNER}/{GITHUB_REPO}/releases/download/v{version}/SHA256SUMS.txt"
    );
    let Ok(sums) = downloader.download(&url) else {
        return Ok(());
    };

    let text = String::from_utf8_lossy(&sums);
    // `sha256sum -b` (binary mode) prefixes the file name with `*`; plain
    // `sha256sum` separates with two spaces. Accept both, like the asset
    // lookup in `scripts/install.sh` should.
    let Some(line) = text.lines().find(|line| {
        line.split_whitespace()
            .nth(1)
            .is_some_and(|name| name.trim_start_matches('*') == asset)
    }) else {
        return Ok(());
    };
    let expected = line.split_whitespace().next().unwrap_or("");
    if expected.is_empty() {
        return Ok(());
    }

    use sha2::{Digest, Sha256};
    let actual: String = Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    if !actual.eq_ignore_ascii_case(expected) {
        bail!("SHA256 mismatch for {asset}: expected {expected}, got {actual}");
    }
    Ok(())
}

/// Extract the binary named `bin_name` from a `.tar.gz` archive read from
/// `reader`, writing it to `dest`. Returns `Ok(false)` when the archive has
/// no such entry; a zero-length entry is fatal, so a truncated or corrupt
/// download never reaches the replace step.
fn extract_binary_from_tar_gz<R: Read>(reader: R, bin_name: &str, dest: &Path) -> Result<bool> {
    let decoder = flate2::read::GzDecoder::new(reader);
    let mut archive = tar::Archive::new(decoder);

    for entry in archive.entries().context("failed to read archive")? {
        let mut entry = entry.context("failed to read archive entry")?;
        let path = entry.path().context("failed to read archive entry path")?;
        if path.file_name().is_some_and(|name| name == bin_name) {
            entry
                .unpack(dest)
                .context("failed to extract binary from archive")?;
            let size = std::fs::metadata(dest)
                .context("failed to read extracted binary metadata")?
                .len();
            if size == 0 {
                bail!("extracted binary '{bin_name}' is empty");
            }
            return Ok(true);
        }
    }
    Ok(false)
}

/// Extract the binary named `bin_name` from a `.zip` archive (the Windows
/// release asset), writing it to `dest`. Same contract as the tar.gz form.
fn extract_binary_from_zip(bytes: &[u8], bin_name: &str, dest: &Path) -> Result<bool> {
    let mut archive =
        zip::ZipArchive::new(std::io::Cursor::new(bytes)).context("failed to read zip archive")?;

    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .context("failed to read zip archive entry")?;
        let name = entry.name();
        let leaf = name.rsplit('/').next().unwrap_or(name);
        if leaf != bin_name {
            continue;
        }
        let mut file =
            std::fs::File::create(dest).context("failed to create the extracted binary")?;
        std::io::copy(&mut entry, &mut file).context("failed to extract binary from archive")?;
        let size = file
            .metadata()
            .context("failed to read extracted binary metadata")?
            .len();
        if size == 0 {
            bail!("extracted binary '{bin_name}' is empty");
        }
        return Ok(true);
    }
    Ok(false)
}

/// Replace `current_exe` from a staged file using a same-directory temp file
/// plus a rename, so the running binary is swapped atomically and never
/// written in place. Falls back to a copy where a rename over the target is
/// not possible (Windows, exotic mounts).
pub fn replace_binary(staged: &Path, current_exe: &Path) -> Result<()> {
    let parent = current_exe
        .parent()
        .context("cannot determine the texforge executable directory")?;
    let temporary = tempfile::NamedTempFile::new_in(parent)
        .context("failed to create an adjacent update file")?;
    std::fs::copy(staged, temporary.path()).context("failed to stage the new binary")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(temporary.path(), std::fs::Permissions::from_mode(0o755))
            .context("failed to set the executable permission on the staged binary")?;
    }
    temporary
        .as_file()
        .sync_all()
        .context("failed to flush the staged binary")?;

    if std::fs::rename(temporary.path(), current_exe).is_ok() {
        return Ok(());
    }

    // The NamedTempFile stays alive until this function returns, so the
    // staged bytes are still available after a failed rename.
    std::fs::copy(temporary.path(), current_exe)
        .context("failed to replace binary (copy fallback)")?;
    Ok(())
}

#[cfg(test)]
mod tests;
