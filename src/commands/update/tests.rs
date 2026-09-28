use super::*;
use std::cell::RefCell;
use std::sync::OnceLock;

/// The compiled-in version, handed out as `&'static SemVer` so every test
/// can build `UpdateDeps` without juggling borrows.
fn current() -> &'static SemVer {
    static CURRENT: OnceLock<SemVer> = OnceLock::new();
    CURRENT.get_or_init(|| get_local_version().expect("CARGO_PKG_VERSION parses"))
}

/// A fake release tag guaranteed newer than this binary, whatever the
/// current version is. Hardcoding "the next version" here re-traps on
/// every version bump: the moment `Cargo.toml` catches up, the test that
/// used to prove "there is something newer" starts proving nothing.
fn fake_newer_tag() -> String {
    let mut parts = env!("CARGO_PKG_VERSION").split('.');
    let (major, minor, patch) = (
        parts.next().expect("semver major"),
        parts.next().expect("semver minor"),
        parts
            .next()
            .expect("semver patch")
            .parse::<u64>()
            .expect("numeric patch"),
    );
    format!("v{major}.{minor}.{}", patch + 1)
}

fn fake_newer_version() -> SemVer {
    SemVer::parse(&fake_newer_tag()).expect("fake tag parses")
}

fn release(tag: &str) -> GitHubRelease {
    GitHubRelease {
        tag_name: tag.to_string(),
        prerelease: false,
        draft: false,
    }
}

/// Offline downloader: always serves the staged archive, and a checksum
/// file only when the test provides one — otherwise it behaves like a
/// release that ships no `SHA256SUMS.txt`.
struct FakeDownloader {
    asset: Vec<u8>,
    checksums: Option<Vec<u8>>,
    calls: RefCell<Vec<String>>,
}

impl FakeDownloader {
    fn new(asset: Vec<u8>) -> Self {
        Self {
            asset,
            checksums: None,
            calls: RefCell::new(Vec::new()),
        }
    }

    fn called(&self) -> bool {
        !self.calls.borrow().is_empty()
    }
}

impl BinaryDownloader for FakeDownloader {
    fn download(&self, url: &str) -> Result<Vec<u8>> {
        self.calls.borrow_mut().push(url.to_string());
        if url.ends_with("SHA256SUMS.txt") {
            match &self.checksums {
                Some(bytes) => Ok(bytes.clone()),
                None => Err(anyhow!("404: release ships no checksum file")),
            }
        } else {
            Ok(self.asset.clone())
        }
    }
}

/// Read-only deps: the executable and target facts are placeholders, the
/// way `--check` runs in production.
fn check_deps<'a>(releases: Vec<GitHubRelease>, downloader: &'a FakeDownloader) -> UpdateDeps<'a> {
    UpdateDeps {
        current: current(),
        releases: Ok(releases),
        exe: Path::new("/tmp/texforge-update-test/texforge"),
        cargo_bin: Path::new("/tmp/texforge-update-test/not-cargo"),
        target: Ok(("x86_64-unknown-linux-musl", "tar.gz")),
        downloader,
        confirm: &|| true,
    }
}

/// A non-cargo-managed install: `texforge` holding old bytes, plus a
/// `not-cargo` directory the guard must ignore.
fn install_target(dir: &Path) -> (PathBuf, PathBuf) {
    let exe = dir.join("texforge");
    std::fs::write(&exe, b"old").unwrap();
    let cargo_bin = dir.join("not-cargo");
    std::fs::create_dir_all(&cargo_bin).unwrap();
    (exe, cargo_bin)
}

/// Deps pointed at a real, non-cargo-managed executable.
fn install_deps<'a>(
    exe: &'a Path,
    cargo_bin: &'a Path,
    downloader: &'a FakeDownloader,
    confirm: &'a dyn Fn() -> bool,
) -> UpdateDeps<'a> {
    UpdateDeps {
        current: current(),
        releases: Ok(vec![release(&fake_newer_tag())]),
        exe,
        cargo_bin,
        target: Ok(("x86_64-unknown-linux-musl", "tar.gz")),
        downloader,
        confirm,
    }
}

/// Build a `.tar.gz` in memory containing `name` → `contents`.
fn tar_bytes(name: &str, contents: &[u8]) -> Vec<u8> {
    let mut tar_data = Vec::new();
    {
        let mut builder = tar::Builder::new(&mut tar_data);
        let mut header = tar::Header::new_gnu();
        header.set_size(contents.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        builder.append_data(&mut header, name, contents).unwrap();
        builder.finish().unwrap();
    }
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    std::io::Write::write_all(&mut encoder, &tar_data).unwrap();
    encoder.finish().unwrap()
}

/// Build a `.zip` in memory containing `name` → `contents`.
fn zip_bytes(name: &str, contents: &[u8]) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    writer.start_file(name, options).unwrap();
    std::io::Write::write_all(&mut writer, contents).unwrap();
    writer.finish().unwrap().into_inner()
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn entry_names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    names.sort();
    names
}

// ── Selection ───────────────────────────────────────────────

/// The fake tag must stay strictly newer than whatever `Cargo.toml`
/// says, forever — never hardcode the next version.
#[test]
fn fake_newer_tag_is_strictly_newer_than_the_compiled_in_version() {
    let fake = fake_newer_version();
    assert!(&fake > current(), "{fake} is not newer than {}", current());
    assert!(fake.is_stable());
}

#[test]
fn select_latest_stable_picks_the_newest_stable_release() {
    let current = SemVer::parse("1.0.0").unwrap();
    let releases = vec![
        release("v1.1.0"),
        release("v1.2.0"),
        GitHubRelease {
            tag_name: "v9.9.9-rc1".to_string(),
            prerelease: true,
            draft: false,
        },
        GitHubRelease {
            tag_name: "v9.9.9".to_string(),
            prerelease: false,
            draft: true,
        },
        release("not-a-version"),
    ];
    assert_eq!(
        select_latest_stable(&releases, &current),
        Some(SemVer::parse("1.2.0").unwrap())
    );
}

#[test]
fn select_latest_stable_ignores_releases_that_are_not_newer() {
    let current = SemVer::parse("1.2.0").unwrap();
    let releases = vec![release("v1.2.0"), release("v1.1.0")];
    assert_eq!(select_latest_stable(&releases, &current), None);
}

// ── Notice path (injected fetcher) ──────────────────────────

struct FakeFetcher {
    body: Result<String, String>,
}

impl ReleaseFetcher for FakeFetcher {
    fn get(&self, _url: &str) -> Result<String> {
        self.body.clone().map_err(|error| anyhow!(error))
    }
}

fn releases_body(entries: &[(&str, bool, bool)]) -> String {
    let items: Vec<String> = entries
        .iter()
        .map(|(tag, prerelease, draft)| {
            format!(r#"{{"tag_name":"{tag}","prerelease":{prerelease},"draft":{draft}}}"#)
        })
        .collect();
    format!("[{}]", items.join(","))
}

#[test]
fn notice_finds_a_newer_release_through_the_fetcher() {
    let fetcher = FakeFetcher {
        body: Ok(releases_body(&[(&fake_newer_tag(), false, false)])),
    };
    assert_eq!(
        fetch_latest_stable_with(&fetcher),
        Some(fake_newer_version())
    );
}

#[test]
fn notice_ignores_drafts_prereleases_and_old_versions() {
    let current_tag = format!("v{}", current());
    let fetcher = FakeFetcher {
        body: Ok(releases_body(&[
            ("v9.9.9", false, true),
            ("v9.9.9", true, false),
            (current_tag.as_str(), false, false),
        ])),
    };
    assert_eq!(fetch_latest_stable_with(&fetcher), None);
}

#[test]
fn notice_stays_silent_when_the_lookup_fails() {
    let fetcher = FakeFetcher {
        body: Err("network is down".to_string()),
    };
    assert_eq!(fetch_latest_stable_with(&fetcher), None);
}

// ── --check exit codes ──────────────────────────────────────

#[test]
fn check_exits_1_when_a_newer_stable_release_exists() {
    let downloader = FakeDownloader::new(Vec::new());
    let deps = check_deps(vec![release(&fake_newer_tag())], &downloader);
    assert_eq!(run_update_with(true, false, &deps).unwrap(), 1);
    assert!(!downloader.called(), "--check must not download");
}

#[test]
fn check_exits_0_when_already_current() {
    let downloader = FakeDownloader::new(Vec::new());
    let deps = check_deps(vec![release(&format!("v{}", current()))], &downloader);
    assert_eq!(run_update_with(true, false, &deps).unwrap(), 0);
    assert!(!downloader.called());
}

#[test]
fn prereleases_and_junk_tags_never_win() {
    let downloader = FakeDownloader::new(Vec::new());
    let newer = fake_newer_tag();
    let deps = check_deps(
        vec![
            GitHubRelease {
                tag_name: format!("{newer}-rc1"),
                prerelease: false,
                draft: false,
            },
            GitHubRelease {
                tag_name: format!("{newer}-beta.1"),
                prerelease: true,
                draft: false,
            },
            release("not-a-version"),
            release(&format!("v{}", current())),
        ],
        &downloader,
    );
    assert_eq!(run_update_with(true, false, &deps).unwrap(), 0);
}

#[test]
fn drafts_never_win() {
    let downloader = FakeDownloader::new(Vec::new());
    let deps = check_deps(
        vec![GitHubRelease {
            tag_name: fake_newer_tag(),
            prerelease: false,
            draft: true,
        }],
        &downloader,
    );
    assert_eq!(run_update_with(true, false, &deps).unwrap(), 0);
}

// ── Errors ──────────────────────────────────────────────────

#[test]
fn release_lookup_errors_surface_loudly() {
    let downloader = FakeDownloader::new(Vec::new());
    let deps = UpdateDeps {
        current: current(),
        releases: Err("network is down".to_string()),
        exe: Path::new("/tmp/texforge-update-test/texforge"),
        cargo_bin: Path::new("/tmp/texforge-update-test/not-cargo"),
        target: Ok(("x86_64-unknown-linux-musl", "tar.gz")),
        downloader: &downloader,
        confirm: &|| true,
    };
    let error = run_update_with(true, false, &deps).unwrap_err();
    assert!(error.to_string().contains("network is down"));
    let error = run_update_with(false, true, &deps).unwrap_err();
    assert!(error.to_string().contains("network is down"));
    assert!(!downloader.called());
}

#[test]
fn unsupported_target_fails_before_downloading() {
    let downloader = FakeDownloader::new(Vec::new());
    let deps = UpdateDeps {
        current: current(),
        releases: Ok(vec![release(&fake_newer_tag())]),
        exe: Path::new("/tmp/texforge-update-test/texforge"),
        cargo_bin: Path::new("/tmp/texforge-update-test/not-cargo"),
        target: Err("unsupported target: x86_64-windows".to_string()),
        downloader: &downloader,
        confirm: &|| true,
    };
    let error = run_update_with(false, true, &deps).unwrap_err();
    assert!(error.to_string().contains("x86_64-windows"));
    assert!(!downloader.called());
}

// ── Consent and the cargo guard ─────────────────────────────

#[test]
fn cargo_installed_binary_is_refused_without_downloading() {
    let dir = tempfile::tempdir().unwrap();
    let cargo_bin = dir.path().join("bin");
    std::fs::create_dir_all(&cargo_bin).unwrap();
    let exe = cargo_bin.join("texforge");
    std::fs::write(&exe, b"cargo-managed").unwrap();

    let downloader = FakeDownloader::new(Vec::new());
    let deps = UpdateDeps {
        current: current(),
        releases: Ok(vec![release(&fake_newer_tag())]),
        exe: exe.as_path(),
        cargo_bin: cargo_bin.as_path(),
        target: Ok(("x86_64-unknown-linux-musl", "tar.gz")),
        downloader: &downloader,
        confirm: &|| panic!("the cargo guard must run before the prompt"),
    };
    assert_eq!(run_update_with(false, true, &deps).unwrap(), 0);
    assert!(!downloader.called());
    assert_eq!(std::fs::read(&exe).unwrap(), b"cargo-managed");
}

#[test]
fn declined_prompt_leaves_everything_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let (exe, cargo_bin) = install_target(dir.path());
    let downloader = FakeDownloader::new(tar_bytes(BIN_NAME, b"new"));
    let deps = install_deps(&exe, &cargo_bin, &downloader, &|| false);

    assert_eq!(run_update_with(false, false, &deps).unwrap(), 0);
    assert!(!downloader.called(), "a declined prompt must not download");
    assert_eq!(std::fs::read(&exe).unwrap(), b"old");
}

// ── Installing ──────────────────────────────────────────────

#[test]
fn yes_replaces_the_running_binary_and_leaves_no_temp_files() {
    let dir = tempfile::tempdir().unwrap();
    let (exe, cargo_bin) = install_target(dir.path());
    let downloader = FakeDownloader::new(tar_bytes(BIN_NAME, b"new"));
    let deps = install_deps(&exe, &cargo_bin, &downloader, &|| {
        panic!("--yes skips the prompt")
    });

    assert_eq!(run_update_with(false, true, &deps).unwrap(), 0);
    assert!(downloader.called());
    assert_eq!(std::fs::read(&exe).unwrap(), b"new");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&exe).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o755);
    }

    let expected = vec![
        cargo_bin.file_name().unwrap().to_string_lossy().to_string(),
        exe.file_name().unwrap().to_string_lossy().to_string(),
    ];
    assert_eq!(entry_names(dir.path()), expected);
}

#[test]
fn zip_assets_install_like_tarballs() {
    let dir = tempfile::tempdir().unwrap();
    let (exe, cargo_bin) = install_target(dir.path());
    let downloader = FakeDownloader::new(zip_bytes(BIN_NAME, b"new"));
    let mut deps = install_deps(&exe, &cargo_bin, &downloader, &|| {
        panic!("--yes skips the prompt")
    });
    deps.target = Ok(("x86_64-pc-windows-msvc", "zip"));

    assert_eq!(run_update_with(false, true, &deps).unwrap(), 0);
    assert_eq!(std::fs::read(&exe).unwrap(), b"new");
}

#[test]
fn archive_without_the_binary_changes_nothing_and_exits_1() {
    let dir = tempfile::tempdir().unwrap();
    let (exe, cargo_bin) = install_target(dir.path());
    let downloader = FakeDownloader::new(tar_bytes("readme.txt", b"not a binary"));
    let deps = install_deps(&exe, &cargo_bin, &downloader, &|| {
        panic!("--yes skips the prompt")
    });

    assert_eq!(run_update_with(false, true, &deps).unwrap(), 1);
    assert_eq!(std::fs::read(&exe).unwrap(), b"old");
}

// ── Checksums ───────────────────────────────────────────────

fn checksums_for(asset_bytes: &[u8]) -> String {
    let asset = asset_name(
        GITHUB_REPO,
        &fake_newer_version(),
        "x86_64-unknown-linux-musl",
        "tar.gz",
    );
    format!("{}  {asset}\n", sha256_hex(asset_bytes))
}

#[test]
fn checksum_is_skipped_when_the_release_ships_none() {
    let dir = tempfile::tempdir().unwrap();
    let (exe, cargo_bin) = install_target(dir.path());
    let downloader = FakeDownloader::new(tar_bytes(BIN_NAME, b"new"));
    let deps = install_deps(&exe, &cargo_bin, &downloader, &|| {
        panic!("--yes skips the prompt")
    });

    assert_eq!(run_update_with(false, true, &deps).unwrap(), 0);
    assert_eq!(std::fs::read(&exe).unwrap(), b"new");
}

#[test]
fn checksum_line_for_another_asset_skips_verification() {
    let dir = tempfile::tempdir().unwrap();
    let (exe, cargo_bin) = install_target(dir.path());
    let asset_bytes = tar_bytes(BIN_NAME, b"new");
    let mut downloader = FakeDownloader::new(asset_bytes);
    downloader.checksums =
        Some(format!("{}  other-asset.tar.gz\n", sha256_hex(b"whatever")).into_bytes());
    let deps = install_deps(&exe, &cargo_bin, &downloader, &|| {
        panic!("--yes skips the prompt")
    });

    assert_eq!(run_update_with(false, true, &deps).unwrap(), 0);
    assert_eq!(std::fs::read(&exe).unwrap(), b"new");
}

#[test]
fn matching_checksum_passes() {
    let dir = tempfile::tempdir().unwrap();
    let (exe, cargo_bin) = install_target(dir.path());
    let asset_bytes = tar_bytes(BIN_NAME, b"new");
    let mut downloader = FakeDownloader::new(asset_bytes);
    downloader.checksums = Some(checksums_for(&downloader.asset).into_bytes());
    let deps = install_deps(&exe, &cargo_bin, &downloader, &|| {
        panic!("--yes skips the prompt")
    });

    assert_eq!(run_update_with(false, true, &deps).unwrap(), 0);
    assert_eq!(std::fs::read(&exe).unwrap(), b"new");
}

#[test]
fn checksum_mismatch_is_fatal_and_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let (exe, cargo_bin) = install_target(dir.path());
    let asset_bytes = tar_bytes(BIN_NAME, b"new");
    let mut downloader = FakeDownloader::new(asset_bytes);
    let asset = asset_name(
        GITHUB_REPO,
        &fake_newer_version(),
        "x86_64-unknown-linux-musl",
        "tar.gz",
    );
    downloader.checksums =
        Some(format!("{}  {asset}\n", sha256_hex(b"something else")).into_bytes());
    let deps = install_deps(&exe, &cargo_bin, &downloader, &|| {
        panic!("--yes skips the prompt")
    });

    let error = run_update_with(false, true, &deps).unwrap_err();
    assert!(error.to_string().contains("SHA256 mismatch"));
    assert_eq!(std::fs::read(&exe).unwrap(), b"old");
}

#[test]
fn checksum_binary_mode_marker_still_verifies() {
    // `sha256sum -b` prefixes the file name with `*` ("hash *asset").
    // The verifier must strip it: a matching digest passes, and a
    // mismatched digest under `*` is still fatal rather than skipped.
    let dir = tempfile::tempdir().unwrap();
    let (exe, cargo_bin) = install_target(dir.path());
    let asset_bytes = tar_bytes(BIN_NAME, b"new");
    let mut downloader = FakeDownloader::new(asset_bytes);
    let asset = asset_name(
        GITHUB_REPO,
        &fake_newer_version(),
        "x86_64-unknown-linux-musl",
        "tar.gz",
    );
    downloader.checksums =
        Some(format!("{} *{asset}\n", sha256_hex(&downloader.asset)).into_bytes());
    let deps = install_deps(&exe, &cargo_bin, &downloader, &|| {
        panic!("--yes skips the prompt")
    });

    assert_eq!(run_update_with(false, true, &deps).unwrap(), 0);
    assert_eq!(std::fs::read(&exe).unwrap(), b"new");
}

// ── Extraction ──────────────────────────────────────────────

#[test]
fn tar_gz_without_the_binary_reports_not_found() {
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("staged");
    let bytes = tar_bytes("readme.txt", b"not a binary");
    assert!(!extract_binary_from_tar_gz(bytes.as_slice(), BIN_NAME, &dest).unwrap());
    assert!(!dest.exists());
}

#[test]
fn tar_gz_with_an_empty_binary_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("staged");
    let bytes = tar_bytes(BIN_NAME, b"");
    let error = extract_binary_from_tar_gz(bytes.as_slice(), BIN_NAME, &dest).unwrap_err();
    assert!(error.to_string().contains("empty"));
}

#[test]
fn zip_without_the_binary_reports_not_found() {
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("staged");
    let bytes = zip_bytes("readme.txt", b"not a binary");
    assert!(!extract_binary_from_zip(&bytes, BIN_NAME, &dest).unwrap());
    assert!(!dest.exists());
}

// ── Replacement ─────────────────────────────────────────────

#[test]
fn replace_binary_swaps_the_target_and_sets_executable_mode() {
    let dir = tempfile::tempdir().unwrap();
    let staged = dir.path().join("staged");
    let target = dir.path().join("texforge");
    std::fs::write(&staged, b"new").unwrap();
    std::fs::write(&target, b"old").unwrap();

    replace_binary(&staged, &target).unwrap();
    assert_eq!(std::fs::read(&target).unwrap(), b"new");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&target).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o755);
    }
}

#[test]
fn a_failed_replace_leaves_the_binary_and_no_temp_files() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("texforge");
    std::fs::write(&target, b"old").unwrap();

    let missing = dir.path().join("never-staged");
    assert!(replace_binary(&missing, &target).is_err());
    assert_eq!(std::fs::read(&target).unwrap(), b"old");
    assert_eq!(entry_names(dir.path()), vec!["texforge".to_string()]);
}

// ── Asset URLs ──────────────────────────────────────────────

/// The asset name has to match what the release workflow publishes,
/// byte for byte, or the printed URL 404s. It did, until 2026-08-10.
#[test]
fn download_url_matches_the_published_asset_name() {
    let version = SemVer::parse("0.7.0").unwrap();
    let (target, ext) = release_target();
    let url = get_release_download_url(GITHUB_OWNER, GITHUB_REPO, &version, target, ext);

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
fn download_url_keeps_owner_repo_and_version() {
    let version = SemVer::parse("2.5.1").unwrap();
    let url = get_release_download_url(
        GITHUB_OWNER,
        GITHUB_REPO,
        &version,
        "x86_64-unknown-linux-musl",
        "tar.gz",
    );
    assert!(url.starts_with("https://github.com/UniverLab/texforge/"));
    assert!(url.contains("/releases/download/v2.5.1/"));
}

// ── Cargo-managed install detection ─────────────────────────

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
    let bin_dir = tmp.path().join("cargo-home").join("bin");
    std::fs::create_dir_all(&bin_dir).unwrap();
    let exe = bin_dir.join("texforge");
    std::fs::write(&exe, b"fake").unwrap();

    assert!(is_cargo_installed(&exe, &bin_dir));
}

#[test]
fn path_outside_cargo_root_is_not_cargo_managed() {
    let tmp = tempfile::tempdir().unwrap();
    let cargo_bin = tmp.path().join("cargo-home").join("bin");
    std::fs::create_dir_all(&cargo_bin).unwrap();
    let elsewhere = tmp.path().join("elsewhere");
    std::fs::create_dir_all(&elsewhere).unwrap();
    let exe = elsewhere.join("texforge");
    std::fs::write(&exe, b"fake").unwrap();

    assert!(!is_cargo_installed(&exe, &cargo_bin));
}

#[test]
fn uncanonicalizable_path_is_treated_as_not_cargo_managed() {
    let missing_exe = Path::new("/nonexistent-path-xyz-texforge-test/texforge");
    let missing_cargo_bin = Path::new("/nonexistent-cargo-home-xyz-texforge-test/bin");
    assert!(!is_cargo_installed(missing_exe, missing_cargo_bin));
}

// ── Notice throttle ─────────────────────────────────────────

#[test]
fn the_notice_is_due_without_a_stamp_or_after_a_day() {
    let dir = tempfile::tempdir().unwrap();
    let stamp = dir.path().join(LAST_CHECK_FILE);
    let now = 1_800_000_000u64;

    assert!(check_is_due(&stamp, now), "a missing stamp is due");

    std::fs::write(&stamp, now.to_string()).unwrap();
    assert!(!check_is_due(&stamp, now), "just checked");
    assert!(!check_is_due(&stamp, now + CHECK_INTERVAL_SECS - 1));
    assert!(check_is_due(&stamp, now + CHECK_INTERVAL_SECS));

    std::fs::write(&stamp, "not-a-timestamp").unwrap();
    assert!(check_is_due(&stamp, now), "a garbage stamp is due");
}

#[test]
fn a_local_version_is_parsed_and_stable() {
    let version = get_local_version().unwrap();
    assert!(version.is_stable());
    assert_eq!(version.to_string(), env!("CARGO_PKG_VERSION"));
}
