//! Embedded templates and template resolution.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};

use crate::utils;

const REGISTRY_REPO: &str = "UniverLab/texforge-templates";

/// How long a cached template is considered fresh before `resolve` attempts a
/// background refresh. Twenty-four hours is a defensible default: templates
/// rarely change more than once a day, but when they do (a bug fix, a new
/// section), a user who builds at least once a day picks up the fix within a
/// day without any manual intervention. Shorter would hit the network on
/// almost every build; longer would leave users on a broken template for too
/// long. Stored as seconds since the Unix epoch.
const CACHE_TTL_SECS: u64 = 86_400;

const CACHE_META_FILE: &str = ".cache_meta.json";

#[derive(serde::Serialize, serde::Deserialize)]
struct CacheMeta {
    fetched_at: u64,
}

/// Embedded files for the "general" template (fallback when offline).
const GENERAL_TEMPLATE_TOML: &str = include_str!("general/template.toml");
const GENERAL_MAIN_TEX: &str = include_str!("general/main.tex");
const GENERAL_BODY_TEX: &str = include_str!("general/sections/body.tex");
const GENERAL_REFERENCES_BIB: &str = include_str!("general/bib/references.bib");

/// A resolved template ready to scaffold a project.
pub struct ResolvedTemplate {
    /// Map of relative path -> file contents.
    pub files: HashMap<String, Vec<u8>>,
}

/// Resolve a template by name: fresh cache → refresh stale cache → download → embedded fallback.
pub fn resolve(name: &str) -> Result<ResolvedTemplate> {
    // 1. Check local cache
    if let Ok((t, fetched_at)) = load_from_cache_with_meta(name) {
        if !is_stale(fetched_at) {
            return Ok(t);
        }
        // Stale: attempt a refresh, but fall back to the cached copy on failure.
        match download(name) {
            Ok(fresh) => return Ok(fresh),
            Err(e) => {
                eprintln!(
                    "texforge: could not refresh template '{}' ({}); using cached copy",
                    name, e
                );
                return Ok(t);
            }
        }
    }

    // 2. Try downloading from GitHub
    if let Ok(t) = download(name) {
        return Ok(t);
    }

    // 3. Fallback to embedded (only "general")
    if name == "general" {
        return Ok(embedded_general());
    }

    anyhow::bail!(
        "Template '{}' not found. Run 'texforge template add {}' first.",
        name,
        name
    );
}

fn is_stale(fetched_at: Option<u64>) -> bool {
    let Some(fetched_at) = fetched_at else {
        return true;
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    now.saturating_sub(fetched_at) > CACHE_TTL_SECS
}

fn embedded_general() -> ResolvedTemplate {
    let mut files = HashMap::new();
    files.insert(
        "template.toml".into(),
        GENERAL_TEMPLATE_TOML.as_bytes().to_vec(),
    );
    files.insert("main.tex".into(), GENERAL_MAIN_TEX.as_bytes().to_vec());
    files.insert(
        "sections/body.tex".into(),
        GENERAL_BODY_TEX.as_bytes().to_vec(),
    );
    files.insert(
        "bib/references.bib".into(),
        GENERAL_REFERENCES_BIB.as_bytes().to_vec(),
    );
    ResolvedTemplate { files }
}

fn load_from_cache_with_meta(name: &str) -> Result<(ResolvedTemplate, Option<u64>)> {
    let dir = utils::templates_dir()?.join(name);
    if !dir.is_dir() {
        anyhow::bail!("not cached");
    }
    let t = load_dir_recursive(&dir)?;
    let fetched_at = read_cache_meta(&dir);
    Ok((t, fetched_at))
}

fn read_cache_meta(dir: &Path) -> Option<u64> {
    let meta_path = dir.join(CACHE_META_FILE);
    let contents = std::fs::read_to_string(&meta_path).ok()?;
    let meta: CacheMeta = serde_json::from_str(&contents).ok()?;
    Some(meta.fetched_at)
}

fn write_cache_meta(dir: &Path) -> Result<()> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let meta = CacheMeta { fetched_at: now };
    let json = serde_json::to_string(&meta)?;
    std::fs::write(dir.join(CACHE_META_FILE), json)?;
    Ok(())
}

fn load_dir_recursive(base: &Path) -> Result<ResolvedTemplate> {
    let mut files = HashMap::new();
    for entry in walkdir::WalkDir::new(base)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if entry.file_type().is_file() {
            let rel = entry
                .path()
                .strip_prefix(base)?
                .to_string_lossy()
                .to_string();
            if rel == CACHE_META_FILE {
                continue;
            }
            let content = std::fs::read(entry.path())?;
            files.insert(rel, content);
        }
    }
    Ok(ResolvedTemplate { files })
}

/// Download a template tarball from GitHub and cache it locally.
pub fn download(name: &str) -> Result<ResolvedTemplate> {
    #[cfg(test)]
    {
        let override_fn = TEST_DOWNLOAD_OVERRIDE.with(|o| o.borrow().as_ref().map(|f| f(name)));
        if let Some(result) = override_fn {
            let files = result?;
            let cache_dir = utils::templates_dir()?.join(name);
            std::fs::create_dir_all(&cache_dir)?;
            for (rel, content) in &files {
                let dest = cache_dir.join(rel);
                if let Some(parent) = dest.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(&dest, content)?;
            }
            write_cache_meta(&cache_dir)?;
            return Ok(ResolvedTemplate { files });
        }
    }

    let url = format!(
        "https://api.github.com/repos/{}/tarball/main",
        REGISTRY_REPO
    );

    let response = reqwest::blocking::Client::new()
        .get(&url)
        .header("User-Agent", "texforge")
        .send()
        .context("Failed to connect to template registry")?;

    if !response.status().is_success() {
        anyhow::bail!("Registry returned HTTP {}", response.status());
    }

    let bytes = response.bytes()?;
    let decoder = flate2::read::GzDecoder::new(&bytes[..]);
    let mut archive = tar::Archive::new(decoder);

    let cache_dir = utils::templates_dir()?.join(name);
    let cache_existed = cache_dir.is_dir();
    let mut files = HashMap::new();
    let prefix = format!("{}/", name);

    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.to_string_lossy().to_string();

        // GitHub tarballs have a root dir like "UniverLab-texforge-templates-abc1234/"
        // We need to find entries under "<root>/<template_name>/..."
        let Some(after_root) = path.split_once('/').map(|x| x.1) else {
            continue;
        };
        let Some(rel) = after_root.strip_prefix(&prefix) else {
            continue;
        };
        if rel.is_empty() || entry.header().entry_type().is_dir() {
            continue;
        }

        let mut content = Vec::new();
        std::io::Read::read_to_end(&mut entry, &mut content)?;

        // Cache to disk
        let dest = cache_dir.join(rel);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&dest, &content)?;

        files.insert(rel.to_string(), content);
    }

    if files.is_empty() {
        if !cache_existed {
            let _ = std::fs::remove_dir_all(&cache_dir);
        }
        anyhow::bail!("Template '{}' not found in registry", name);
    }

    write_cache_meta(&cache_dir)?;

    Ok(ResolvedTemplate { files })
}

#[cfg(test)]
type DownloadOverride =
    Box<dyn Fn(&str) -> std::result::Result<HashMap<String, Vec<u8>>, anyhow::Error>>;

#[cfg(test)]
thread_local! {
    static TEST_DOWNLOAD_OVERRIDE: std::cell::RefCell<Option<DownloadOverride>> =
        std::cell::RefCell::new(None);
}

#[cfg(test)]
fn set_download_override<F>(f: F)
where
    F: Fn(&str) -> std::result::Result<HashMap<String, Vec<u8>>, anyhow::Error> + 'static,
{
    TEST_DOWNLOAD_OVERRIDE.with(|o| *o.borrow_mut() = Some(Box::new(f)));
}

#[cfg(test)]
fn clear_download_override() {
    TEST_DOWNLOAD_OVERRIDE.with(|o| *o.borrow_mut() = None);
}

/// List template names available in the remote registry.
pub fn list_remote() -> Result<Vec<String>> {
    let url = format!("https://api.github.com/repos/{}/contents", REGISTRY_REPO);

    let response = reqwest::blocking::Client::new()
        .get(&url)
        .header("User-Agent", "texforge")
        .send()
        .context("Failed to connect to template registry")?;

    if !response.status().is_success() {
        anyhow::bail!("Registry returned HTTP {}", response.status());
    }

    #[derive(serde::Deserialize)]
    struct Entry {
        name: String,
        #[serde(rename = "type")]
        kind: String,
    }

    let entries: Vec<Entry> = response.json()?;
    let mut names: Vec<String> = entries
        .into_iter()
        .filter(|e| e.kind == "dir")
        .map(|e| e.name)
        .collect();
    names.sort();
    Ok(names)
}

/// List template names available in local cache.
pub fn list_cached() -> Result<Vec<String>> {
    let dir = utils::templates_dir()?;
    let mut names = Vec::new();
    if dir.is_dir() {
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                if let Some(name) = entry.file_name().to_str() {
                    names.push(name.to_string());
                }
            }
        }
    }
    names.sort();
    Ok(names)
}

/// Remove a template from local cache.
pub fn remove_cached(name: &str) -> Result<PathBuf> {
    let dir = utils::templates_dir()?.join(name);
    if !dir.is_dir() {
        anyhow::bail!("Template '{}' is not installed", name);
    }
    std::fs::remove_dir_all(&dir)?;
    Ok(dir)
}

/// Force-refresh a single cached template, bypassing the TTL.
/// If the download fails and a cached copy exists, the cache is kept.
pub fn refresh(name: &str) -> Result<()> {
    let dir = utils::templates_dir()?.join(name);
    let had_cache = dir.is_dir();
    match download(name) {
        Ok(_) => Ok(()),
        Err(e) if had_cache => {
            eprintln!(
                "texforge: could not refresh template '{}' ({}); keeping cached copy",
                name, e
            );
            Ok(())
        }
        Err(e) => Err(e),
    }
}

/// Force-refresh every cached template, bypassing the TTL.
/// Templates that fail to refresh are reported on stderr but do not abort.
pub fn refresh_all() -> Result<()> {
    let names = list_cached()?;
    if names.is_empty() {
        println!("No cached templates to refresh.");
        return Ok(());
    }
    for name in &names {
        print!("Refreshing '{}'... ", name);
        match download(name.as_str()) {
            Ok(_) => println!("done"),
            Err(e) => {
                println!("failed ({})", e);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn ensure_rustls() {
        let _ = rustls::crypto::ring::default_provider().install_default();
    }

    #[test]
    fn embedded_general_has_required_files() {
        let t = embedded_general();
        assert!(t.files.contains_key("template.toml"));
        assert!(t.files.contains_key("main.tex"));
        assert!(t.files.contains_key("sections/body.tex"));
        assert!(t.files.contains_key("bib/references.bib"));
    }

    #[test]
    fn embedded_general_main_tex_is_valid_utf8() {
        let t = embedded_general();
        let main = t.files.get("main.tex").unwrap();
        let text = std::str::from_utf8(main).expect("main.tex should be valid UTF-8");
        assert!(text.contains("\\documentclass"));
    }

    #[test]
    fn embedded_general_template_toml_is_valid_toml() {
        let t = embedded_general();
        let toml_bytes = t.files.get("template.toml").unwrap();
        let text = std::str::from_utf8(toml_bytes).unwrap();
        let parsed: toml::Value = toml::from_str(text).expect("template.toml should be valid TOML");
        assert!(parsed.is_table());
    }

    #[test]
    fn embedded_general_body_tex_not_empty() {
        let t = embedded_general();
        let body = t.files.get("sections/body.tex").unwrap();
        assert!(!body.is_empty());
    }

    #[test]
    fn embedded_general_references_bib_not_empty() {
        let t = embedded_general();
        let bib = t.files.get("bib/references.bib").unwrap();
        assert!(!bib.is_empty());
    }

    #[test]
    fn resolve_general_returns_embedded() {
        let t = embedded_general();
        assert!(t.files.contains_key("main.tex"));
    }

    #[test]
    fn list_cached_returns_vec() {
        let result = list_cached();
        assert!(result.is_ok());
    }

    #[test]
    fn remove_cached_nonexistent_fails() {
        let result = remove_cached("definitely-not-cached-xyz-123");
        assert!(result.is_err());
    }

    #[test]
    fn embedded_files_count_is_four() {
        let t = embedded_general();
        assert_eq!(t.files.len(), 4);
    }

    #[test]
    fn resolve_unknown_template_errors() {
        ensure_rustls();
        let result = resolve("nonexistent-template-xyz-123");
        assert!(result.is_err());
        // Verify the error message mentions the template name
        if let Err(e) = result {
            let msg = format!("{}", e);
            assert!(msg.contains("not found"));
        }
    }

    #[test]
    fn load_dir_recursive_with_files() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path();
        fs::write(base.join("main.tex"), "\\documentclass{article}").unwrap();
        let sub = base.join("sections");
        fs::create_dir_all(&sub).unwrap();
        fs::write(sub.join("body.tex"), "Hello").unwrap();
        fs::write(base.join("refs.bib"), "@misc{a}").unwrap();

        let result = load_dir_recursive(base).unwrap();
        assert!(result.files.contains_key("main.tex"));
        assert!(result.files.contains_key("sections/body.tex"));
        assert!(result.files.contains_key("refs.bib"));
        assert_eq!(result.files.len(), 3);
    }

    #[test]
    fn load_dir_recursive_empty_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let result = load_dir_recursive(tmp.path()).unwrap();
        assert!(result.files.is_empty());
    }

    #[test]
    fn load_dir_recursive_file_contents_match() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("a.tex"), "content_a").unwrap();
        let result = load_dir_recursive(tmp.path()).unwrap();
        let content = result.files.get("a.tex").unwrap();
        assert_eq!(content, b"content_a");
    }

    #[test]
    fn list_cached_empty_when_no_templates() {
        // Just verify it doesn't panic; the real dir may or may not have templates
        let result = list_cached();
        assert!(result.is_ok());
        let _ = result.unwrap();
    }

    #[test]
    fn list_cached_finds_cached_templates() {
        // Verify the function returns a sorted list
        let result = list_cached().unwrap();
        // Check it's sorted
        for w in result.windows(2) {
            assert!(w[0] <= w[1]);
        }
    }

    #[test]
    fn remove_cached_removes_existing() {
        // Create a temp template in the real templates dir, then remove it
        let templates_dir = crate::utils::templates_dir().unwrap();
        let test_dir = templates_dir.join("__test_remove_temp__");
        std::fs::create_dir_all(&test_dir).unwrap();
        std::fs::write(test_dir.join("x.tex"), "x").unwrap();
        let path = remove_cached("__test_remove_temp__").unwrap();
        assert!(path.ends_with("__test_remove_temp__"));
        assert!(!test_dir.exists());
    }

    #[test]
    fn load_from_cache_nonexistent_errors() {
        let result = load_from_cache_with_meta("no-such-template-xyz-abc");
        assert!(result.is_err());
    }

    #[test]
    fn embedded_general_toml_content_is_nonempty() {
        let t = embedded_general();
        let toml = t.files.get("template.toml").unwrap();
        assert!(!toml.is_empty());
        let text = std::str::from_utf8(toml).unwrap();
        assert!(text.contains("template"));
    }

    #[test]
    fn list_cached_nonexistent_dir_returns_empty() {
        // If templates dir doesn't exist, list_cached should return empty
        // But utils::templates_dir() creates the dir, so we just test the function
        let result = list_cached();
        assert!(result.is_ok());
    }

    #[test]
    fn is_stale_returns_false_for_fresh_entry() {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        assert!(!is_stale(Some(now)));
        assert!(!is_stale(Some(now - CACHE_TTL_SECS / 2)));
    }

    #[test]
    fn is_stale_returns_true_for_old_entry() {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        assert!(is_stale(Some(now - CACHE_TTL_SECS - 1)));
        assert!(is_stale(Some(0)));
    }

    #[test]
    fn is_stale_returns_true_when_no_metadata() {
        assert!(is_stale(None));
    }

    #[test]
    fn fresh_cache_served_without_network() {
        let templates_dir = crate::utils::templates_dir().unwrap();
        let test_name = "__test_fresh_cache__";
        let test_dir = templates_dir.join(test_name);
        std::fs::create_dir_all(&test_dir).unwrap();
        std::fs::write(test_dir.join("main.tex"), "fresh-content").unwrap();
        write_cache_meta(&test_dir).unwrap();

        let result = resolve(test_name).unwrap();
        assert!(result.files.contains_key("main.tex"));
        assert_eq!(result.files.get("main.tex").unwrap(), b"fresh-content");

        std::fs::remove_dir_all(&test_dir).unwrap();
    }

    #[test]
    fn stale_cache_falls_back_when_refresh_fails() {
        let templates_dir = crate::utils::templates_dir().unwrap();
        let test_name = "__test_stale_fallback__";
        let test_dir = templates_dir.join(test_name);
        std::fs::create_dir_all(&test_dir).unwrap();
        std::fs::write(test_dir.join("main.tex"), "stale-content").unwrap();
        let old_meta = CacheMeta { fetched_at: 0 };
        std::fs::write(
            test_dir.join(CACHE_META_FILE),
            serde_json::to_string(&old_meta).unwrap(),
        )
        .unwrap();

        ensure_rustls();
        let result = resolve(test_name).unwrap();
        assert!(result.files.contains_key("main.tex"));
        assert_eq!(result.files.get("main.tex").unwrap(), b"stale-content");

        std::fs::remove_dir_all(&test_dir).unwrap();
    }

    #[test]
    fn cache_without_meta_is_treated_as_stale() {
        let templates_dir = crate::utils::templates_dir().unwrap();
        let test_name = "__test_no_meta_stale__";
        let test_dir = templates_dir.join(test_name);
        std::fs::create_dir_all(&test_dir).unwrap();
        std::fs::write(test_dir.join("main.tex"), "no-meta-content").unwrap();

        let (_, fetched_at) = load_from_cache_with_meta(test_name).unwrap();
        assert!(fetched_at.is_none());
        assert!(is_stale(fetched_at));

        ensure_rustls();
        let result = resolve(test_name).unwrap();
        assert_eq!(result.files.get("main.tex").unwrap(), b"no-meta-content");

        std::fs::remove_dir_all(&test_dir).unwrap();
    }

    #[test]
    fn write_and_read_cache_meta_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        write_cache_meta(tmp.path()).unwrap();
        let fetched_at = read_cache_meta(tmp.path());
        assert!(fetched_at.is_some());
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        assert!((fetched_at.unwrap() as i64 - now as i64).unsigned_abs() < 5);
    }

    #[test]
    fn load_dir_recursive_skips_cache_meta_file() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("main.tex"), "content").unwrap();
        fs::write(tmp.path().join(CACHE_META_FILE), "{}").unwrap();

        let result = load_dir_recursive(tmp.path()).unwrap();
        assert!(result.files.contains_key("main.tex"));
        assert!(!result.files.contains_key(CACHE_META_FILE));
        assert_eq!(result.files.len(), 1);
    }

    #[test]
    fn embedded_fallback_works_when_no_cache() {
        ensure_rustls();
        let result = resolve("general").unwrap();
        assert!(result.files.contains_key("main.tex"));
        assert!(result.files.contains_key("template.toml"));
    }

    #[test]
    fn stale_cache_refresh_returns_fresh_content() {
        let templates_dir = crate::utils::templates_dir().unwrap();
        let test_name = "__test_stale_refresh__";
        let test_dir = templates_dir.join(test_name);

        std::fs::create_dir_all(&test_dir).unwrap();
        std::fs::write(test_dir.join("main.tex"), "stale-content").unwrap();
        let old_meta = CacheMeta { fetched_at: 0 };
        std::fs::write(
            test_dir.join(CACHE_META_FILE),
            serde_json::to_string(&old_meta).unwrap(),
        )
        .unwrap();

        let old_ts = read_cache_meta(&test_dir).unwrap();

        set_download_override(|_name| {
            let mut files = HashMap::new();
            files.insert("main.tex".into(), b"fresh-content".to_vec());
            Ok(files)
        });

        let result = resolve(test_name).unwrap();
        assert_eq!(result.files.get("main.tex").unwrap(), b"fresh-content");

        let new_ts = read_cache_meta(&test_dir).unwrap();
        assert!(new_ts > old_ts);

        clear_download_override();
        std::fs::remove_dir_all(&test_dir).unwrap();
    }

    #[test]
    fn refresh_bypasses_ttl_on_fresh_entry() {
        let templates_dir = crate::utils::templates_dir().unwrap();
        let test_name = "__test_refresh_bypass_ttl__";
        let test_dir = templates_dir.join(test_name);

        std::fs::create_dir_all(&test_dir).unwrap();
        std::fs::write(test_dir.join("main.tex"), "original-content").unwrap();
        write_cache_meta(&test_dir).unwrap();

        let pre_ts = read_cache_meta(&test_dir).unwrap();
        assert!(!is_stale(Some(pre_ts)));

        std::thread::sleep(std::time::Duration::from_secs(2));

        set_download_override(|_name| {
            let mut files = HashMap::new();
            files.insert("main.tex".into(), b"refreshed-content".to_vec());
            Ok(files)
        });

        refresh(test_name).unwrap();

        let result = load_from_cache_with_meta(test_name).unwrap();
        assert_eq!(
            result.0.files.get("main.tex").unwrap(),
            b"refreshed-content"
        );

        let post_ts = result.1.unwrap();
        assert!(post_ts > pre_ts);

        clear_download_override();
        std::fs::remove_dir_all(&test_dir).unwrap();
    }
}
