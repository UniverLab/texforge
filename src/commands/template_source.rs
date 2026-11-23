//! `texforge template source` subcommands: add, list and remove the extra
//! template registries stored as `[[template_sources]]` tables in the user
//! config file (the official registry is always present and never stored).

use anyhow::{Context, Result};

use crate::templates::load_sources;
use crate::templates::sources::{
    authed_get, is_valid_name, is_valid_repo, TemplateSource, OFFICIAL_NAME, OFFICIAL_REF,
};

/// Accept `owner/name`, `https://github.com/owner/name` and
/// `https://github.com/owner/name.git`; everything becomes `owner/name`.
pub(crate) fn normalize_repo(input: &str) -> String {
    let trimmed = input.trim();
    let path = trimmed
        .strip_prefix("https://github.com/")
        .unwrap_or(trimmed);
    path.strip_suffix(".git").unwrap_or(path).to_string()
}

/// Escape a value for a double-quoted TOML basic string.
fn escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Map the reachability probe's HTTP status to a result. Success is any 2xx;
/// a 404 or 403 names both ways to authenticate (the repo may be private);
/// any other status fails with the status. No `registry.toml` is needed.
fn check_status(repo: &str, status: reqwest::StatusCode) -> Result<()> {
    if status.is_success() {
        return Ok(());
    }
    if status.as_u16() == 404 || status.as_u16() == 403 {
        anyhow::bail!("{}", crate::templates::private_repo_message(repo));
    }
    anyhow::bail!("Template source '{repo}' returned HTTP {status}")
}

/// One authenticated-if-available GET to
/// `api.github.com/repos/<repo>/commits/<ref>`, interpreted by
/// [`check_status`].
fn check_source(repo: &str, git_ref: &str) -> Result<()> {
    let probe = TemplateSource {
        name: "probe".to_string(),
        repo: repo.to_string(),
        git_ref: git_ref.to_string(),
    };
    let url = format!("https://api.github.com/repos/{repo}/commits/{git_ref}");
    let response = authed_get(&url, &probe)
        .send()
        .context("Failed to reach api.github.com")?;
    check_status(repo, response.status())
}

/// Append one `[[template_sources]]` table as raw text, keeping every other
/// byte of the file untouched. The `ref` line is written only when given.
fn append_source(name: &str, repo: &str, git_ref: Option<&str>) -> Result<()> {
    let path = crate::config::config_file_path()?;
    let mut text = match std::fs::read_to_string(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => {
            return Err(e)
                .with_context(|| format!("Failed to read config file: {}", path.display()));
        }
        Ok(text) => text,
    };
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str("[[template_sources]]\n");
    text.push_str(&format!("name = \"{}\"\n", escape(name)));
    text.push_str(&format!("repo = \"{}\"\n", escape(repo)));
    if let Some(git_ref) = git_ref {
        text.push_str(&format!("ref = \"{}\"\n", escape(git_ref)));
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, text)
        .with_context(|| format!("Failed to write config file: {}", path.display()))?;
    Ok(())
}

/// Add a template source. The config file is written only after the
/// reachability check passes (`--no-check` skips it, for offline use).
pub fn add(name: &str, repo_or_url: &str, git_ref: Option<&str>, no_check: bool) -> Result<()> {
    if name == OFFICIAL_NAME {
        anyhow::bail!("Name '{OFFICIAL_NAME}' is reserved for the official registry");
    }
    if !is_valid_name(name) {
        anyhow::bail!("Invalid source name '{name}': must match [a-z0-9][a-z0-9-]*");
    }
    let repo = normalize_repo(repo_or_url);
    if !is_valid_repo(&repo) {
        anyhow::bail!("Invalid repo '{repo}': must be `owner/name`");
    }
    if let Some(git_ref) = git_ref {
        if git_ref.is_empty() {
            anyhow::bail!("Invalid ref: must be non-empty (default '{OFFICIAL_REF}')");
        }
    }
    for source in load_sources()? {
        if source.name == name {
            anyhow::bail!(
                "Source '{name}' already exists; \
                 run 'texforge template source remove {name}' first"
            );
        }
    }
    let effective_ref = git_ref.unwrap_or(OFFICIAL_REF);
    if !no_check {
        check_source(&repo, effective_ref)?;
    }
    append_source(name, &repo, git_ref)?;
    println!("  ◇ Source '{name}' added ({repo})");
    Ok(())
}

/// Render the source table: the official source first (marked as default),
/// then the configured ones; columns are name, `owner/name` and ref.
fn render_table(sources: &[TemplateSource]) -> String {
    let mut name_width = "NAME".len();
    let mut repo_width = "REPO".len();
    let names: Vec<String> = sources
        .iter()
        .map(|s| {
            if s.is_official() {
                format!("{} (default)", s.name)
            } else {
                s.name.clone()
            }
        })
        .collect();
    for (source, name) in sources.iter().zip(&names) {
        name_width = name_width.max(name.len());
        repo_width = repo_width.max(source.repo.len());
    }
    let mut out = format!("{:<name_width$}  {:<repo_width$}  REF\n", "NAME", "REPO");
    for (source, name) in sources.iter().zip(&names) {
        out.push_str(&format!(
            "{:<name_width$}  {:<repo_width$}  {}\n",
            name, source.repo, source.git_ref
        ));
    }
    out
}

/// Render the sources as one JSON array: objects with exactly the keys
/// `name`, `repo`, `ref` and `official`, the official source first.
fn render_json(sources: &[TemplateSource]) -> String {
    let entries: Vec<serde_json::Value> = sources
        .iter()
        .map(|s| {
            serde_json::json!({
                "name": s.name,
                "repo": s.repo,
                "ref": s.git_ref,
                "official": s.is_official(),
            })
        })
        .collect();
    serde_json::to_string(&entries).unwrap_or_else(|_| "[]".to_string())
}

/// List the template sources: the official source first (marked as default,
/// never removable), then the configured ones. `--json` prints exactly one
/// JSON array on stdout and nothing else.
pub fn list(json: bool) -> Result<()> {
    let sources = load_sources()?;
    if json {
        println!("{}", render_json(&sources));
    } else {
        print!("{}", render_table(&sources));
    }
    Ok(())
}

/// Split a `key = value` line at the first `=` outside a double-quoted string.
fn split_kv(line: &str) -> Option<(&str, &str)> {
    let mut in_string = false;
    for (index, c) in line.char_indices() {
        match c {
            '"' => in_string = !in_string,
            '=' if !in_string => return Some((&line[..index], &line[index + 1..])),
            _ => {}
        }
    }
    None
}

/// Read a double-quoted TOML basic string, ignoring anything after its
/// closing quote (such as a trailing comment).
fn quoted(value: &str) -> Option<String> {
    let inner = value.trim().strip_prefix('"')?;
    let mut out = String::new();
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                if let Some(e) = chars.next() {
                    out.push(e);
                }
            }
            '"' => return Some(out),
            _ => out.push(c),
        }
    }
    None
}

/// A line starting a new TOML table ends a `[[template_sources]]` block.
fn is_header(line: &str) -> bool {
    line.trim_start().starts_with('[')
}

/// The `name` carried by a `[[template_sources]]` block, if any.
fn block_name(block: &[&str]) -> Option<String> {
    for line in block {
        if let Some((key, value)) = split_kv(line) {
            if key.trim() == "name" {
                return quoted(value);
            }
        }
    }
    None
}

/// Remove the `[[template_sources]]` block carrying `target`, splicing bytes
/// so every other byte (comments, spacing, trailing newline) is untouched.
/// Returns `None` when no block carries that name.
fn delete_block(text: &str, target: &str) -> Option<String> {
    let lines: Vec<&str> = text.split('\n').collect();
    let mut starts: Vec<usize> = Vec::with_capacity(lines.len());
    let mut offset = 0;
    for line in &lines {
        starts.push(offset);
        offset += line.len() + 1; // the '\n' that `split` removed
    }
    let mut index = 0;
    while index < lines.len() {
        if lines[index].trim() == "[[template_sources]]" {
            let mut end = index + 1;
            while end < lines.len() && !is_header(lines[end]) {
                end += 1;
            }
            if block_name(&lines[index..end]).as_deref() == Some(target) {
                let cut_end = if end < lines.len() {
                    starts[end]
                } else {
                    text.len()
                };
                let mut updated = String::with_capacity(text.len());
                updated.push_str(&text[..starts[index]]);
                updated.push_str(&text[cut_end..]);
                return Some(updated);
            }
            index = end;
        } else {
            index += 1;
        }
    }
    None
}

/// Remove a template source from the config file, preserving the rest.
/// Templates already installed from it are left untouched.
pub fn remove(name: &str) -> Result<()> {
    if name == OFFICIAL_NAME {
        anyhow::bail!("Source '{OFFICIAL_NAME}' is the default registry and cannot be removed");
    }
    let path = crate::config::config_file_path()?;
    let text = match std::fs::read_to_string(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            anyhow::bail!("Unknown source '{name}'");
        }
        Err(e) => {
            return Err(e)
                .with_context(|| format!("Failed to read config file: {}", path.display()));
        }
        Ok(text) => text,
    };
    let Some(updated) = delete_block(&text, name) else {
        anyhow::bail!("Unknown source '{name}'");
    };
    std::fs::write(&path, updated)
        .with_context(|| format!("Failed to write config file: {}", path.display()))?;
    println!("  ◇ Source '{name}' removed (installed templates were left untouched)");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Run `f` with `HOME` and `XDG_CONFIG_HOME` pointed at a fresh temp dir.
    /// The reachability check always runs with `--no-check`: never the network.
    fn with_temp_home(f: impl FnOnce(&std::path::Path)) {
        let _lock = crate::test_sync::ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let tmp = tempfile::tempdir().unwrap();
        let _restore = crate::test_sync::EnvGuard::capture(&["HOME", "XDG_CONFIG_HOME"]);
        std::env::set_var("HOME", tmp.path());
        std::env::set_var("XDG_CONFIG_HOME", tmp.path());
        f(tmp.path());
    }

    #[test]
    fn check_status_accepts_any_2xx() {
        for code in [200, 201, 204] {
            let status = reqwest::StatusCode::from_u16(code).unwrap();
            assert!(check_status("acme/templates", status).is_ok(), "{code}");
        }
    }

    #[test]
    fn check_status_404_and_403_name_both_auth_ways() {
        for code in [404, 403] {
            let status = reqwest::StatusCode::from_u16(code).unwrap();
            let err = check_status("acme/templates", status)
                .unwrap_err()
                .to_string();
            assert!(err.contains("acme/templates"), "{err}");
            assert!(err.contains("GITHUB_TOKEN"), "{err}");
            assert!(err.contains("gh auth login"), "{err}");
        }
    }

    #[test]
    fn check_status_other_failures_report_the_status() {
        let status = reqwest::StatusCode::from_u16(500).unwrap();
        let err = check_status("acme/templates", status)
            .unwrap_err()
            .to_string();
        assert!(err.contains("500"), "{err}");
    }

    #[test]
    fn normalize_repo_accepts_all_three_forms() {
        assert_eq!(normalize_repo("acme/templates"), "acme/templates");
        assert_eq!(
            normalize_repo("https://github.com/acme/templates"),
            "acme/templates"
        );
        assert_eq!(
            normalize_repo("https://github.com/acme/templates.git"),
            "acme/templates"
        );
    }

    #[test]
    fn add_then_list_shows_the_source() {
        with_temp_home(|_| {
            add("acme", "acme/templates", None, true).unwrap();
            let sources = load_sources().unwrap();
            assert_eq!(sources.len(), 2);
            assert_eq!(sources[0], TemplateSource::official());
            assert_eq!(sources[1].name, "acme");
            assert_eq!(sources[1].repo, "acme/templates");
            assert_eq!(sources[1].git_ref, "main");
            let table = render_table(&sources);
            assert!(table.contains("acme"), "{table}");
            assert!(table.contains("acme/templates"), "{table}");
            assert!(table.contains("official"), "{table}");
            assert!(table.contains("(default)"), "{table}");
        });
    }

    #[test]
    fn add_with_ref_keeps_the_ref() {
        with_temp_home(|_| {
            add("lab", "lab/tpls", Some("v2"), true).unwrap();
            let sources = load_sources().unwrap();
            assert_eq!(sources[1].git_ref, "v2");
        });
    }

    #[test]
    fn add_twice_fails_and_points_at_remove() {
        with_temp_home(|_| {
            add("acme", "acme/templates", None, true).unwrap();
            let err = add("acme", "other/repo", None, true)
                .unwrap_err()
                .to_string();
            assert!(err.contains("already exists"), "{err}");
            assert!(err.contains("remove"), "{err}");
            assert_eq!(load_sources().unwrap().len(), 2);
        });
    }

    #[test]
    fn remove_then_list_leaves_only_official() {
        with_temp_home(|_| {
            add("acme", "acme/templates", None, true).unwrap();
            remove("acme").unwrap();
            assert_eq!(load_sources().unwrap(), vec![TemplateSource::official()]);
        });
    }

    #[test]
    fn remove_unknown_and_official_fail() {
        with_temp_home(|_| {
            let err = remove("nope").unwrap_err().to_string();
            assert!(err.contains("Unknown source 'nope'"), "{err}");
            let err = remove("official").unwrap_err().to_string();
            assert!(err.contains("cannot be removed"), "{err}");
        });
    }

    #[test]
    fn official_name_is_rejected_on_add() {
        with_temp_home(|_| {
            let err = add("official", "acme/templates", None, true)
                .unwrap_err()
                .to_string();
            assert!(err.contains("reserved"), "{err}");
        });
    }

    #[test]
    fn bad_names_repos_and_refs_are_rejected() {
        with_temp_home(|_| {
            for bad in ["Acme", "ac_me", ""] {
                let err = add(bad, "acme/templates", None, true)
                    .unwrap_err()
                    .to_string();
                assert!(err.contains("Invalid source name"), "{err}");
            }
            for bad in ["noslash", "a/b/c", ""] {
                let err = add("ok-name", bad, None, true).unwrap_err().to_string();
                assert!(err.contains("Invalid repo"), "{err}");
            }
            let err = add("ok-name", "a/b", Some(""), true)
                .unwrap_err()
                .to_string();
            assert!(err.contains("Invalid ref"), "{err}");
            assert_eq!(load_sources().unwrap(), vec![TemplateSource::official()]);
        });
    }

    #[test]
    fn all_three_url_forms_land_on_the_same_entry() {
        for input in [
            "acme/templates",
            "https://github.com/acme/templates",
            "https://github.com/acme/templates.git",
        ] {
            with_temp_home(|_| {
                add("acme", input, None, true).unwrap();
                let sources = load_sources().unwrap();
                assert_eq!(sources.len(), 2);
                assert_eq!(sources[1].repo, "acme/templates");
            });
        }
    }

    #[test]
    fn comments_survive_add_and_remove_byte_for_byte() {
        with_temp_home(|_| {
            let path = crate::config::config_file_path().unwrap();
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            let before = "# a comment\n[user]\nname = \"Jane\"\n\n# another\n";
            std::fs::write(&path, before).unwrap();
            add(
                "acme",
                "https://github.com/acme/templates.git",
                Some("v2"),
                true,
            )
            .unwrap();
            let after_add = std::fs::read_to_string(&path).unwrap();
            assert!(after_add.starts_with(before), "{after_add}");
            remove("acme").unwrap();
            assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
        });
    }

    #[test]
    fn json_keys_are_exactly_name_repo_ref_official() {
        with_temp_home(|_| {
            add("acme", "acme/templates", None, true).unwrap();
            add("lab", "lab/tpls", Some("v2"), true).unwrap();
            let sources = load_sources().unwrap();
            let value: serde_json::Value = serde_json::from_str(&render_json(&sources)).unwrap();
            let entries = value.as_array().unwrap();
            assert_eq!(entries.len(), 3);
            for entry in entries {
                let keys: Vec<&str> = entry
                    .as_object()
                    .unwrap()
                    .keys()
                    .map(String::as_str)
                    .collect();
                assert_eq!(keys, vec!["name", "official", "ref", "repo"]);
            }
            assert_eq!(entries[0]["name"], "official");
            assert_eq!(entries[0]["official"], true);
            assert_eq!(entries[1]["name"], "acme");
            assert_eq!(entries[1]["repo"], "acme/templates");
            assert_eq!(entries[1]["ref"], "main");
            assert_eq!(entries[1]["official"], false);
            assert_eq!(entries[2]["ref"], "v2");
        });
    }
}
