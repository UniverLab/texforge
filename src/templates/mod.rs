//! Embedded templates and template resolution.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::utils;

const REGISTRY_REPO: &str = "UniverLab/texforge-templates";

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

/// Resolve a template by name: local cache → download → embedded fallback.
pub fn resolve(name: &str) -> Result<ResolvedTemplate> {
    // 1. Check local cache
    if let Ok(t) = load_from_cache(name) {
        return Ok(t);
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

fn load_from_cache(name: &str) -> Result<ResolvedTemplate> {
    let dir = utils::templates_dir()?.join(name);
    if !dir.is_dir() {
        anyhow::bail!("not cached");
    }
    load_dir_recursive(&dir)
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
            let content = std::fs::read(entry.path())?;
            files.insert(rel, content);
        }
    }
    Ok(ResolvedTemplate { files })
}

/// Download a template tarball from GitHub and cache it locally.
pub fn download(name: &str) -> Result<ResolvedTemplate> {
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
        // Clean up empty cache dir
        let _ = std::fs::remove_dir_all(&cache_dir);
        anyhow::bail!("Template '{}' not found in registry", name);
    }

    Ok(ResolvedTemplate { files })
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

#[cfg(test)]
mod tests {
    use super::*;

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
        let t = resolve("general").unwrap();
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
}
