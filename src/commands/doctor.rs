//! `texforge doctor` command implementation.
//!
//! Reports the verified state of everything texforge manages: the Tectonic
//! engine, its resource cache, fonts available to it, installed spell-check
//! dictionaries, and whether the current directory is a texforge project.
//! It diagnoses, and refreshes an outdated managed Tectonic copy like a
//! build does; it never repairs or cleans anything else.

use anyhow::Result;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::compiler;
use crate::compiler::tectonic_version::{is_older, query_tectonic_version};
use crate::compiler::TECTONIC_VERSION;
use crate::domain::project::Project;
use crate::linter;

/// Run every check and print a human-readable report.
///
/// Every other section is purely informational; Tectonic is the one
/// component a build cannot proceed without.
pub fn execute() -> Result<()> {
    println!("texforge doctor");

    compiler::refresh_managed_tectonic();
    let tectonic = compiler::locate_tectonic();
    let tectonic_version = tectonic.as_deref().and_then(query_tectonic_version);
    println!();
    println!("Tectonic");
    print!(
        "{}{}",
        format_tectonic_status(
            tectonic
                .as_deref()
                .map(|p| (p, tectonic_version.as_deref()))
        ),
        outdated_line(tectonic_version.as_deref(), TECTONIC_VERSION).unwrap_or_default()
    );

    let cache_dir = tectonic_cache_dir();
    println!();
    report_cache(cache_dir.as_deref());

    println!();
    report_fonts(cache_dir.as_deref());

    println!();
    report_dictionaries();

    println!();
    report_project();

    if tectonic.is_none() {
        anyhow::bail!("Tectonic is not present — builds cannot run");
    }
    Ok(())
}

/// Format the Tectonic status line(s). Separated from resolution so the
/// missing-binary case is testable without touching `PATH`.
fn format_tectonic_status(found: Option<(&Path, Option<&str>)>) -> String {
    match found {
        Some((path, version)) => format!(
            "  \u{2713} present: {}\n    version: {}\n",
            path.display(),
            version.unwrap_or("(could not determine)")
        ),
        None => "  \u{2717} not found\n".to_string(),
    }
}

/// Warn when the Tectonic that will run is older than the supported one.
/// Pure so the warning text is testable without touching `PATH`.
fn outdated_line(found: Option<&str>, pinned: &str) -> Option<String> {
    let version = found?;
    if is_older(version, pinned) {
        Some(format!(
            "    ! older than the supported {pinned}; run 'texforge build' to update the managed copy, or remove this one from PATH\n"
        ))
    } else {
        None
    }
}

/// Resolve Tectonic's resource cache directory: `$TECTONIC_CACHE_DIR` when
/// set (Tectonic itself honors this override), otherwise the platform cache
/// directory joined with `Tectonic` — matching Tectonic's own default.
fn tectonic_cache_dir() -> Option<PathBuf> {
    std::env::var_os("TECTONIC_CACHE_DIR")
        .map(PathBuf::from)
        .or_else(|| dirs::cache_dir().map(|d| d.join("Tectonic")))
}

/// Recursively total the size and file count of everything under `dir`.
/// A missing or empty directory reports `(0, 0)`.
fn dir_stats(dir: &Path) -> (u64, usize) {
    let mut size = 0u64;
    let mut count = 0usize;
    for entry in walkdir::WalkDir::new(dir)
        .into_iter()
        .filter_map(std::result::Result::ok)
    {
        if entry.file_type().is_file() {
            count += 1;
            if let Ok(meta) = entry.metadata() {
                size += meta.len();
            }
        }
    }
    (size, count)
}

/// Render a byte count as a human-readable size (e.g. `65.2 MB`).
fn format_size(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    format!("{size:.1} {}", UNITS[unit])
}

fn report_cache(cache_dir: Option<&Path>) {
    println!("Cache");
    let Some(dir) = cache_dir else {
        println!("  \u{2717} could not determine cache location (no home directory)");
        return;
    };
    println!("  location: {}", dir.display());
    if !dir.exists() {
        println!("  size: 0 B");
        println!("  entries: 0 (not created yet — run a build to populate it)");
        return;
    }
    let (size, count) = dir_stats(dir);
    println!("  size: {}", format_size(size));
    println!("  entries: {count}");
}

/// Font-program extensions Tectonic embeds glyph data from, as opposed to
/// `.tfm`/`.vf` files, which describe spacing but carry no glyph outlines.
const FONT_EXTENSIONS: &[&str] = &["otf", "ttf", "ttc", "otc", "pfb"];

/// Pull font filenames out of Tectonic bundle manifest text. Each manifest
/// line is `filename size hash`; this reports what the cache actually
/// recorded, not what a bundle might theoretically contain.
fn extract_font_names(manifest_content: &str) -> BTreeSet<String> {
    let mut fonts = BTreeSet::new();
    for line in manifest_content.lines() {
        let Some(name) = line.split_whitespace().next() else {
            continue;
        };
        let Some(ext) = name.rsplit('.').next() else {
            continue;
        };
        if FONT_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()) {
            fonts.insert(name.to_string());
        }
    }
    fonts
}

fn fonts_from_cache(cache_dir: &Path) -> BTreeSet<String> {
    let manifests_dir = cache_dir.join("manifests");
    let mut fonts = BTreeSet::new();
    let Ok(entries) = std::fs::read_dir(&manifests_dir) else {
        return fonts;
    };
    for entry in entries.filter_map(std::result::Result::ok) {
        if let Ok(content) = std::fs::read_to_string(entry.path()) {
            fonts.extend(extract_font_names(&content));
        }
    }
    fonts
}

fn report_fonts(cache_dir: Option<&Path>) {
    println!("Fonts");
    let Some(dir) = cache_dir else {
        println!("  \u{2717} could not determine cache location (no home directory)");
        return;
    };
    let fonts = fonts_from_cache(dir);
    if fonts.is_empty() {
        println!("  0 fonts available (no cached bundle manifest — build once to populate)");
        return;
    }
    println!("  {} font file(s) available", fonts.len());
    const SHOWN: usize = 20;
    for font in fonts.iter().take(SHOWN) {
        println!("    {font}");
    }
    if fonts.len() > SHOWN {
        println!("    ... and {} more", fonts.len() - SHOWN);
    }
}

fn report_dictionaries() {
    println!("Dictionaries");
    let dicts = linter::installed_dictionaries();
    if dicts.is_empty() {
        println!("  none installed");
        return;
    }
    for dict in &dicts {
        match dict {
            linter::InstalledDictionary::Wordlist { lang, path } => {
                match std::fs::read_to_string(path) {
                    Ok(content) => {
                        let words = content.lines().filter(|l| !l.trim().is_empty()).count();
                        println!("  {lang}: {} ({words} words)", path.display());
                    }
                    Err(e) => println!("  {lang}: {} (unreadable: {e})", path.display()),
                }
            }
            linter::InstalledDictionary::Hunspell {
                lang,
                dic_path,
                aff_path,
            } => {
                println!(
                    "  {lang}: hunspell dictionary ({}, {})",
                    dic_path.display(),
                    aff_path.display()
                );
            }
        }
    }
}

fn report_project() {
    println!("Project");
    let cwd = match std::env::current_dir() {
        Ok(d) => d,
        Err(e) => {
            println!("  \u{2717} could not determine current directory: {e}");
            return;
        }
    };
    let config_path = cwd.join("project.toml");
    if !config_path.exists() {
        println!(
            "  \u{2717} not a texforge project (no project.toml in {})",
            cwd.display()
        );
        return;
    }
    match Project::load() {
        Ok(project) => {
            println!("  \u{2713} {}", config_path.display());
            println!("    title: {}", project.config.document.title);
            println!("    entry: {}", project.config.build.entry);
        }
        Err(e) => {
            println!(
                "  \u{2717} {} exists but failed to parse: {e}",
                config_path.display()
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    // --- Tectonic status formatting (covers the missing-binary case) ---

    #[test]
    fn format_tectonic_status_missing_binary() {
        let out = format_tectonic_status(None);
        assert!(out.contains("not found"));
        assert!(!out.contains("present"));
    }

    #[test]
    fn format_tectonic_status_present_with_version() {
        let path = PathBuf::from("/usr/local/bin/tectonic");
        let out = format_tectonic_status(Some((&path, Some("0.15.0"))));
        assert!(out.contains("present: /usr/local/bin/tectonic"));
        assert!(out.contains("version: 0.15.0"));
    }

    #[test]
    fn format_tectonic_status_present_unknown_version() {
        let path = PathBuf::from("/usr/local/bin/tectonic");
        let out = format_tectonic_status(Some((&path, None)));
        assert!(out.contains("could not determine"));
    }

    // --- Outdated-Tectonic warning (pure: no PATH access) ---

    #[test]
    fn outdated_line_warns_for_older_version() {
        assert_eq!(
            outdated_line(Some("0.15.0"), "0.17.0"),
            Some(
                "    ! older than the supported 0.17.0; run 'texforge build' to update the managed copy, or remove this one from PATH\n"
                    .to_string()
            )
        );
    }

    #[test]
    fn outdated_line_none_for_current_version() {
        assert_eq!(outdated_line(Some("0.17.0"), "0.17.0"), None);
    }

    #[test]
    fn outdated_line_none_when_version_unknown() {
        assert_eq!(outdated_line(None, "0.17.0"), None);
    }

    // --- Cache stats (covers the empty-cache case) ---

    #[test]
    fn dir_stats_missing_dir_is_zero() {
        let tmp = tempfile::tempdir().unwrap();
        let missing = tmp.path().join("does-not-exist");
        assert_eq!(dir_stats(&missing), (0, 0));
    }

    #[test]
    fn dir_stats_empty_dir_is_zero() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(dir_stats(tmp.path()), (0, 0));
    }

    #[test]
    fn dir_stats_counts_nested_files() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("a"), b"1234").unwrap();
        let sub = tmp.path().join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(sub.join("b"), b"12345678").unwrap();
        assert_eq!(dir_stats(tmp.path()), (12, 2));
    }

    #[test]
    fn report_cache_empty_cache_dir_does_not_panic() {
        let tmp = tempfile::tempdir().unwrap();
        report_cache(Some(tmp.path()));
    }

    #[test]
    fn report_cache_missing_dir_does_not_panic() {
        let tmp = tempfile::tempdir().unwrap();
        let missing = tmp.path().join("not-created-yet");
        report_cache(Some(&missing));
    }

    #[test]
    fn report_cache_none_does_not_panic() {
        report_cache(None);
    }

    #[test]
    fn format_size_bytes() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(512), "512 B");
    }

    #[test]
    fn format_size_kilobytes() {
        assert_eq!(format_size(2048), "2.0 KB");
    }

    #[test]
    fn format_size_megabytes() {
        assert_eq!(format_size(65 * 1024 * 1024), "65.0 MB");
    }

    // --- Font extraction from bundle manifests ---

    #[test]
    fn extract_font_names_filters_by_extension() {
        let manifest = "article.cls 20144 abc\nlmroman12-regular.otf 110400 def\nec-lmr12.tfm 12092 ghi\ncmr10.pfb 5000 jkl\n";
        let fonts = extract_font_names(manifest);
        assert_eq!(fonts.len(), 2);
        assert!(fonts.contains("lmroman12-regular.otf"));
        assert!(fonts.contains("cmr10.pfb"));
        assert!(!fonts.contains("article.cls"));
        assert!(!fonts.contains("ec-lmr12.tfm"));
    }

    #[test]
    fn extract_font_names_empty_manifest() {
        assert!(extract_font_names("").is_empty());
    }

    #[test]
    fn extract_font_names_deduplicates() {
        let manifest = "font.otf 100 aaa\nfont.otf 100 bbb\n";
        assert_eq!(extract_font_names(manifest).len(), 1);
    }

    #[test]
    fn fonts_from_cache_missing_manifests_dir_is_empty() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(fonts_from_cache(tmp.path()).is_empty());
    }

    #[test]
    fn fonts_from_cache_reads_all_manifest_files() {
        let tmp = tempfile::tempdir().unwrap();
        let manifests = tmp.path().join("manifests");
        std::fs::create_dir_all(&manifests).unwrap();
        std::fs::write(manifests.join("a.txt"), "one.otf 1 x\n").unwrap();
        std::fs::write(manifests.join("b.txt"), "two.pfb 1 y\n").unwrap();
        let fonts = fonts_from_cache(tmp.path());
        assert_eq!(fonts.len(), 2);
        assert!(fonts.contains("one.otf"));
        assert!(fonts.contains("two.pfb"));
    }

    // --- Dictionary reporting ---

    #[test]
    fn report_dictionaries_runs_without_panicking() {
        report_dictionaries();
    }

    // --- Project detection ---

    #[test]
    fn report_project_no_project_toml() {
        let tmp = tempfile::tempdir().unwrap();
        let _cwd = crate::test_sync::CWD_LOCK.lock().unwrap();
        let orig = std::env::current_dir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();
        report_project();
        std::env::set_current_dir(&orig).unwrap();
    }

    #[test]
    fn report_project_valid_project_toml() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("project.toml"),
            "[document]\ntitle = \"T\"\nauthor = \"A\"\ntemplate = \"general\"\n\n[build]\nentry = \"main.tex\"\n",
        )
        .unwrap();
        let _cwd = crate::test_sync::CWD_LOCK.lock().unwrap();
        let orig = std::env::current_dir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();
        report_project();
        std::env::set_current_dir(&orig).unwrap();
    }

    #[test]
    fn report_project_invalid_project_toml() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("project.toml"), "not valid {{{ toml").unwrap();
        let _cwd = crate::test_sync::CWD_LOCK.lock().unwrap();
        let orig = std::env::current_dir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();
        report_project();
        std::env::set_current_dir(&orig).unwrap();
    }

    // --- Cache directory resolution respects TECTONIC_CACHE_DIR ---

    #[test]
    fn tectonic_cache_dir_honors_env_override() {
        let orig = std::env::var_os("TECTONIC_CACHE_DIR");
        std::env::set_var("TECTONIC_CACHE_DIR", "/tmp/custom-tectonic-cache");
        let dir = tectonic_cache_dir();
        match orig {
            Some(v) => std::env::set_var("TECTONIC_CACHE_DIR", v),
            None => std::env::remove_var("TECTONIC_CACHE_DIR"),
        }
        assert_eq!(dir, Some(PathBuf::from("/tmp/custom-tectonic-cache")));
    }
}
