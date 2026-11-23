//! Local template folders for `texforge new -t <path>`.
//!
//! A `-t` value is a path when it starts with `./`, `../`, `/`, `~/` or is
//! `.`/`..` (one trailing `/` is allowed), or when a directory of that exact
//! name exists in the current directory and contains a `template.toml`.
//! Otherwise it is a template name resolved as before. `~` expands through
//! `dirs::home_dir()`, the same `HOME`-based helper the rest of the crate
//! uses, so tests can point it at a temporary directory.
//!
//! A local template is used in place: nothing is copied into
//! `~/.texforge/templates/` and no registry request is made. The folder is
//! read with the same checks `template add` runs on a local folder
//! (`template.toml` present, manifest `id` usable, `main.tex` present,
//! unsafe entries skipped), and the recorded template name is the manifest
//! `id`, never the path.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::manifest::TemplateManifest;
use crate::templates::{is_plain_name, is_safe_relative};

/// Strip one trailing `/`, keeping `/` itself intact.
fn strip_one_trailing_slash(raw: &str) -> &str {
    if raw.len() > 1 && raw.ends_with('/') {
        &raw[..raw.len() - 1]
    } else {
        raw
    }
}

/// True when the `-t` value names a local template folder rather than an
/// installed or registry template.
pub(crate) fn is_local_arg(raw: &str) -> bool {
    let stripped = strip_one_trailing_slash(raw);
    if stripped == "." || stripped == ".." {
        return true;
    }
    if stripped.starts_with("./")
        || stripped.starts_with("../")
        || stripped.starts_with('/')
        || stripped.starts_with("~/")
    {
        return true;
    }
    let path = Path::new(stripped);
    path.is_dir() && path.join("template.toml").is_file()
}

/// Resolve the `-t` path value to a directory. `~/` expands through the
/// `HOME`-based helper; every other form is used as given.
pub(crate) fn resolve_dir(raw: &str) -> PathBuf {
    let stripped = strip_one_trailing_slash(raw);
    if stripped == "~" {
        if let Some(home) = dirs::home_dir() {
            return home;
        }
    } else if let Some(rest) = stripped.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(stripped)
}

/// Read a local template folder into memory, returning the files keyed by
/// their path relative to the folder and the manifest `id` the project
/// records as its template name.
///
/// Fails naming what is missing — a missing `template.toml`, an unreadable
/// manifest or unusable `id`, a missing `main.tex` — so the caller can fail
/// before creating the project directory. Entries that would escape the
/// folder are skipped, exactly as `template add` skips them.
pub(crate) fn load_local(dir: &Path) -> Result<(HashMap<String, Vec<u8>>, String)> {
    let manifest_path = dir.join("template.toml");
    if !manifest_path.is_file() {
        anyhow::bail!(
            "No template.toml in '{}'; a template needs a manifest",
            dir.display()
        );
    }
    let manifest = TemplateManifest::from_file(&manifest_path).with_context(|| {
        format!(
            "Failed to read template manifest '{}'",
            manifest_path.display()
        )
    })?;
    if !is_plain_name(&manifest.id) {
        anyhow::bail!(
            "Template id '{}' in '{}' is not a plain name (ASCII letters, digits, '-' and '_' only)",
            manifest.id,
            manifest_path.display()
        );
    }
    if !dir.join("main.tex").is_file() {
        anyhow::bail!("No main.tex in '{}'", dir.display());
    }
    let mut files = HashMap::new();
    for entry in walkdir::WalkDir::new(dir)
        .into_iter()
        .filter_map(|entry| entry.ok())
    {
        let rel = entry
            .path()
            .strip_prefix(dir)
            .with_context(|| format!("Failed to relativize '{}'", entry.path().display()))?
            .to_string_lossy()
            .to_string();
        if rel.is_empty() || !is_safe_relative(&rel) {
            continue;
        }
        if entry.file_type().is_file() {
            let content = std::fs::read(entry.path())
                .with_context(|| format!("Failed to read '{}'", entry.path().display()))?;
            files.insert(rel, content);
        }
    }
    Ok((files, manifest.id))
}
