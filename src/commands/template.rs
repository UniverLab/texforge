//! `texforge template` command implementation.

use std::collections::HashSet;

use anyhow::Result;

use crate::templates;
use crate::templates::refs::warning_line;

/// One installed or remote template, with its source.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    name: String,
    source: String,
}

/// What `list` shows: installed entries, remote entries not yet installed,
/// one warning per unreachable source, and the official failure, if any.
struct Listing {
    installed: Vec<Entry>,
    available: Vec<Entry>,
    warnings: Vec<(String, String)>,
    official_error: Option<String>,
}

/// Gather installed templates plus, unless `include_remote` is false, the
/// remote listing of every configured source. An unreachable source yields a
/// warning and contributes nothing; it never hides the other sources.
fn collect_listing(include_remote: bool) -> Result<Listing> {
    let installed: Vec<Entry> = templates::list_cached_entries()?
        .into_iter()
        .map(|e| Entry {
            name: e.name,
            source: e.source,
        })
        .collect();
    let mut listing = Listing {
        installed,
        available: Vec::new(),
        warnings: Vec::new(),
        official_error: None,
    };
    if !include_remote {
        return Ok(listing);
    }
    let known: HashSet<(String, String)> = listing
        .installed
        .iter()
        .map(|e| (e.name.clone(), e.source.clone()))
        .collect();
    for source in templates::load_sources()? {
        match templates::list_remote_from(&source) {
            Ok(names) => {
                for name in names {
                    if name == "general" && source.is_official() {
                        continue;
                    }
                    let entry = Entry {
                        name,
                        source: source.name.clone(),
                    };
                    if !known.contains(&(entry.name.clone(), entry.source.clone()))
                        && !listing.available.contains(&entry)
                    {
                        listing.available.push(entry);
                    }
                }
            }
            Err(e) => {
                let reason = format!("{e:#}");
                if source.is_official() {
                    listing.official_error = Some(reason);
                } else {
                    listing.warnings.push((source.name.clone(), reason));
                }
            }
        }
    }
    listing
        .available
        .sort_by(|a, b| (&a.name, &a.source).cmp(&(&b.name, &b.source)));
    Ok(listing)
}

/// Human lines for the `Installed:` section. The built-in template keeps its
/// existing `  - general (built-in)` text; every other entry shows its source.
fn installed_lines(entries: &[Entry]) -> Vec<String> {
    let mut lines = vec!["  - general (built-in)".to_string()];
    let mut rest: Vec<String> = entries
        .iter()
        .filter(|e| !(e.name == "general" && e.source == templates::OFFICIAL_NAME))
        .map(|e| format!("  - {} ({})", e.name, e.source))
        .collect();
    rest.sort();
    lines.extend(rest);
    lines
}

/// Human lines for the `Available (not installed):` section.
fn available_lines(entries: &[Entry]) -> Vec<String> {
    entries
        .iter()
        .map(|e| format!("  - {} ({})", e.name, e.source))
        .collect()
}

/// One JSON object per template: installed entries plus, without `--local`,
/// the available ones. Sorted by name, then source.
fn json_items(listing: &Listing) -> Vec<serde_json::Value> {
    let mut items = Vec::new();
    items.push(serde_json::json!({"name": "general", "source": "built-in", "installed": true}));
    for e in &listing.installed {
        if e.name == "general" && e.source == templates::OFFICIAL_NAME {
            continue;
        }
        items.push(serde_json::json!({"name": e.name, "source": e.source, "installed": true}));
    }
    for e in &listing.available {
        items.push(serde_json::json!({"name": e.name, "source": e.source, "installed": false}));
    }
    items.sort_by(|a, b| {
        (a["name"].as_str(), a["source"].as_str()).cmp(&(b["name"].as_str(), b["source"].as_str()))
    });
    items
}

/// List available templates. By default also queries every configured remote
/// source (`include_remote = false` via `--local` lists only installed ones).
/// With `json`, stdout is exactly one JSON array and nothing else.
pub fn list(include_remote: bool, json: bool) -> Result<()> {
    let listing = collect_listing(include_remote)?;
    if json {
        for (source, reason) in &listing.warnings {
            eprintln!("{}", warning_line(source, reason));
        }
        if let Some(e) = &listing.official_error {
            eprintln!("Could not reach registry: {e}");
        }
        println!("{}", serde_json::to_string(&json_items(&listing))?);
        return Ok(());
    }
    println!("Installed:");
    for line in installed_lines(&listing.installed) {
        println!("{line}");
    }
    if include_remote {
        for (source, reason) in &listing.warnings {
            println!("{}", warning_line(source, reason));
        }
        if let Some(e) = &listing.official_error {
            println!("\nCould not reach registry: {e}");
        }
        if listing.available.is_empty() && listing.official_error.is_some() {
            return Ok(());
        }
        if listing.available.is_empty() {
            println!("Available (not installed): none");
        } else {
            println!("Available (not installed):");
            for line in available_lines(&listing.available) {
                println!("{line}");
            }
        }
        println!("\nRun 'texforge template add <name>' (or '<source>/<name>') to install.");
    }
    Ok(())
}

/// Valid source names (official first), for unknown-source errors.
fn source_names() -> Result<Vec<String>> {
    Ok(templates::load_sources()?
        .iter()
        .map(|s| s.name.clone())
        .collect())
}

/// Warn once per unreachable source on stderr (non-list commands).
fn warn_failures(failures: &[templates::refs::SourceFailure]) {
    for failure in failures {
        eprintln!("{}", warning_line(&failure.source, &failure.reason));
    }
}

/// Add a template from a registry source or from a local folder.
///
/// `source` is a path to a template folder, a bare template name, or
/// `source/name`. A bare name offered by more than one source fails naming
/// every candidate; an unknown source part fails listing valid sources.
pub fn add(source: &str, force: bool) -> Result<()> {
    if std::path::Path::new(source).is_dir() {
        println!("Installing template from '{}'...", source);
        let id = templates::install_from_dir(std::path::Path::new(source), force)?;
        println!("  ◇ Template '{}' installed", id);
        return Ok(());
    }

    // Fail fast on malformed references and unknown sources before touching
    // the network: neither needs a single source to answer.
    let parsed = templates::refs::parse_ref(source)?;
    let names = source_names()?;
    if let Some(wanted) = &parsed.0 {
        if !names.iter().any(|s| s == wanted) {
            anyhow::bail!(
                "Unknown template source '{wanted}'. Valid sources: {}",
                names.join(", ")
            );
        }
    }
    let union = templates::union_candidates()?;
    warn_failures(&union.failures);
    let found = templates::resolve_in(source, &union.candidates, &names)?;

    let sources = templates::load_sources()?;
    let from = sources
        .iter()
        .find(|s| s.name == found.source)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "Unknown template source '{}'. Valid sources: {}",
                found.source,
                names.join(", ")
            )
        })?;

    let flat = templates::installed_from_local(&found.reference())?;
    if flat && !force {
        anyhow::bail!(
            "Template '{}' is installed from a local folder; remove it first or pass --force",
            found.reference()
        );
    }

    println!("Downloading template '{}'...", found.reference());
    templates::download_from(from, &found.name)?;
    if flat {
        // Only drop the marker once the registry copy is on disk: a failed
        // download must leave the local install exactly as it was.
        templates::clear_local_marker(&found.reference())?;
    }
    println!("  ◇ Template '{}' installed", found.reference());
    Ok(())
}

/// Remove a template from local cache. Accepts `name` or `source/name`;
/// a bare name offered by more than one installed source fails naming both.
pub fn remove(name: &str) -> Result<()> {
    let names = source_names()?;
    let candidates = templates::installed_candidates()?;
    let found = templates::resolve_in(name, &candidates, &names)?;
    let path = templates::remove_cached(&found.reference())?;
    println!(
        "  ◇ Removed template '{}' ({})",
        found.reference(),
        path.display()
    );
    Ok(())
}

/// Validate template compatibility.
pub fn validate(name: &str) -> Result<()> {
    let resolved = templates::resolve(name)?;
    if resolved.files.contains_key("template.toml") {
        println!("  ◇ Template '{}' is valid", name);
    } else {
        anyhow::bail!("Template '{}' is missing template.toml", name);
    }
    Ok(())
}

/// Refresh cached templates, bypassing the TTL. Accepts `name` or
/// `source/name`; a bare name offered by more than one installed source
/// fails naming both.
pub fn refresh(name: Option<&str>) -> Result<()> {
    match name {
        Some(n) => {
            let names = source_names()?;
            let candidates = templates::installed_candidates()?;
            let found = templates::resolve_in(n, &candidates, &names)?;
            println!("Refreshing template '{}'...", found.reference());
            templates::refresh(&found.reference())?;
            println!("  ◇ Template '{}' refreshed", found.reference());
        }
        None => {
            templates::refresh_all()?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ensure_rustls() {
        let _ = rustls::crypto::ring::default_provider().install_default();
    }

    fn with_temp_home(f: impl FnOnce(&std::path::Path)) {
        let _lock = crate::test_sync::ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let tmp = tempfile::tempdir().unwrap();
        let _restore = crate::test_sync::EnvGuard::capture(&["HOME", "XDG_CONFIG_HOME"]);
        std::env::set_var("HOME", tmp.path());
        std::env::set_var("XDG_CONFIG_HOME", tmp.path());
        f(tmp.path());
        crate::templates::clear_list_override();
        crate::templates::clear_download_override();
    }

    fn write_config(home: &std::path::Path, contents: &str) {
        let dir = home.join("texforge");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("config.toml"), contents).unwrap();
    }

    const ACME_CONFIG: &str =
        "[[template_sources]]\nname = \"acme\"\nrepo = \"acme/texforge-templates\"\n";

    fn two_sources() {
        crate::templates::set_list_override(|source| match source {
            "official" => Ok(vec!["official-tpl".to_string()]),
            "acme" => Ok(vec!["acme-tpl".to_string()]),
            other => Err(anyhow::anyhow!("unexpected source {other}")),
        });
    }

    fn entry(name: &str, source: &str) -> Entry {
        Entry {
            name: name.to_string(),
            source: source.to_string(),
        }
    }

    fn write_flat_template(home: &std::path::Path, name: &str) {
        let dir = home.join(".texforge/templates").join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("template.toml"), "id = \"x\"\n").unwrap();
        std::fs::write(dir.join("main.tex"), "flat-content").unwrap();
    }

    #[test]
    fn validate_general_template() {
        ensure_rustls();
        // "general" falls back to embedded, which has template.toml
        validate("general").unwrap();
    }

    #[test]
    fn validate_unknown_template_errors() {
        ensure_rustls();
        let result = validate("definitely-not-a-template-xyz");
        assert!(result.is_err());
    }

    #[test]
    fn remove_nonexistent_template_errors() {
        let result = remove("definitely-not-cached-xyz");
        assert!(result.is_err());
    }

    #[test]
    fn add_rejects_paths_that_are_neither_a_folder_nor_a_reference() {
        for source in ["https://x.y/z", "a/b/c"] {
            let msg = add(source, false).unwrap_err().to_string();
            assert_eq!(
                msg,
                format!(
                    "'{source}' is neither a local folder nor a registry template name. \
                     Use a registry name (texforge template list) or a path to a template folder."
                )
            );
        }
    }

    #[test]
    fn add_rejects_an_unknown_source_naming_valid_sources() {
        with_temp_home(|home| {
            write_config(home, ACME_CONFIG);
            crate::templates::set_list_override(|_| Ok(Vec::new()));
            let msg = add("nope/tpl", false).unwrap_err().to_string();
            assert_eq!(
                msg,
                "Unknown template source 'nope'. Valid sources: official, acme"
            );
        });
    }

    #[test]
    fn add_installs_a_local_folder_and_needs_force_to_replace_it() {
        with_temp_home(|home| {
            crate::templates::set_list_override(|_| Ok(Vec::new()));
            let src = home.join("official").join("lab-report");
            std::fs::create_dir_all(&src).unwrap();
            std::fs::write(
                src.join("template.toml"),
                "id = \"lab-report\"\nversion = \"0.1.0\"\n\
                 display_name = \"Lab report\"\ndescription = \"Lab report\"\n",
            )
            .unwrap();
            std::fs::write(src.join("main.tex"), "x").unwrap();
            let source = src.to_string_lossy().to_string();

            add(&source, false).unwrap();
            assert!(home
                .join(".texforge/templates/lab-report/.local_source")
                .is_file());

            let msg = add(&source, false).unwrap_err().to_string();
            assert_eq!(
                msg,
                "Template 'lab-report' already exists; pass --force to replace it"
            );

            add(&source, true).unwrap();
        });
    }

    #[test]
    fn add_refuses_to_replace_a_local_install_by_registry_download_without_force() {
        with_temp_home(|home| {
            crate::templates::set_list_override(|_| Ok(Vec::new()));
            let src = home.join("official").join("lab-report");
            std::fs::create_dir_all(&src).unwrap();
            std::fs::write(
                src.join("template.toml"),
                "id = \"lab-report\"\nversion = \"0.1.0\"\n\
                 display_name = \"Lab report\"\ndescription = \"Lab report\"\n",
            )
            .unwrap();
            std::fs::write(src.join("main.tex"), "x").unwrap();
            add(&src.to_string_lossy(), false).unwrap();

            let msg = add("lab-report", false).unwrap_err().to_string();
            assert_eq!(
                msg,
                "Template 'lab-report' is installed from a local folder; \
                 remove it first or pass --force"
            );
        });
    }

    #[test]
    fn add_force_replaces_a_local_install_with_the_registry_copy() {
        with_temp_home(|home| {
            crate::templates::set_list_override(|_| Ok(Vec::new()));
            let src = home.join("official").join("lab-report");
            std::fs::create_dir_all(&src).unwrap();
            std::fs::write(
                src.join("template.toml"),
                "id = \"lab-report\"\nversion = \"0.1.0\"\n\
                 display_name = \"Lab report\"\ndescription = \"Lab report\"\n",
            )
            .unwrap();
            std::fs::write(src.join("main.tex"), "local").unwrap();
            add(&src.to_string_lossy(), false).unwrap();
            let installed = home.join(".texforge/templates/lab-report");
            assert!(installed.join(".local_source").is_file());

            crate::templates::set_download_override(|_name| {
                let mut files = std::collections::HashMap::new();
                files.insert("main.tex".into(), b"registry".to_vec());
                Ok(files)
            });
            add("lab-report", true).unwrap();

            assert!(!installed.join(".local_source").exists());
            assert!(installed.join(".cache_meta.json").is_file());
            assert_eq!(
                std::fs::read(installed.join("main.tex")).unwrap(),
                b"registry"
            );
        });
    }

    #[test]
    fn add_force_keeps_the_local_marker_when_the_download_fails() {
        with_temp_home(|home| {
            crate::templates::set_list_override(|_| Ok(Vec::new()));
            let src = home.join("official").join("lab-report");
            std::fs::create_dir_all(&src).unwrap();
            std::fs::write(
                src.join("template.toml"),
                "id = \"lab-report\"\nversion = \"0.1.0\"\n\
                 display_name = \"Lab report\"\ndescription = \"Lab report\"\n",
            )
            .unwrap();
            std::fs::write(src.join("main.tex"), "local").unwrap();
            add(&src.to_string_lossy(), false).unwrap();

            crate::templates::set_download_override(|_name| {
                Err(anyhow::anyhow!("network is down"))
            });
            let err = add("lab-report", true).unwrap_err().to_string();

            assert!(err.contains("network is down"), "{err}");
            let installed = home.join(".texforge/templates/lab-report");
            assert!(installed.join(".local_source").is_file());
            assert_eq!(std::fs::read(installed.join("main.tex")).unwrap(), b"local");
        });
    }

    #[test]
    fn list_with_two_sources_shows_both_suffixes() {
        with_temp_home(|home| {
            write_config(home, ACME_CONFIG);
            two_sources();
            write_flat_template(home, "thesis");
            let listing = collect_listing(true).unwrap();
            assert!(listing.warnings.is_empty());
            let installed = installed_lines(&listing.installed);
            assert!(installed.contains(&"  - general (built-in)".to_string()));
            assert!(installed.contains(&"  - thesis (official)".to_string()));
            let available = available_lines(&listing.available);
            assert_eq!(
                available,
                vec![
                    "  - acme-tpl (acme)".to_string(),
                    "  - official-tpl (official)".to_string(),
                ]
            );
        });
    }

    #[test]
    fn clash_fails_naming_both_candidates_as_source_name() {
        with_temp_home(|home| {
            write_config(home, ACME_CONFIG);
            crate::templates::set_list_override(|_| Ok(vec!["dup".to_string()]));
            let msg = add("dup", false).unwrap_err().to_string();
            assert!(msg.contains("  - acme/dup"), "{msg}");
            assert!(msg.contains("  - official/dup"), "{msg}");
            assert!(!home.join(".texforge/templates/dup").exists(), "{msg}");
            assert!(!home.join(".texforge/templates/acme").exists(), "{msg}");
        });
    }

    #[test]
    fn qualified_reference_picks_the_right_source() {
        with_temp_home(|home| {
            write_config(home, ACME_CONFIG);
            crate::templates::set_list_override(|_| Ok(vec!["dup".to_string()]));
            crate::templates::set_download_override(|_name| {
                let mut files = std::collections::HashMap::new();
                files.insert("main.tex".into(), b"acme-content".to_vec());
                Ok(files)
            });
            add("acme/dup", false).unwrap();
            let nested = home.join(".texforge/templates/acme/dup/main.tex");
            assert_eq!(std::fs::read(nested).unwrap(), b"acme-content");
            assert!(!home.join(".texforge/templates/dup").exists());
        });
    }

    #[test]
    fn unreachable_source_warns_while_official_still_lists() {
        with_temp_home(|home| {
            write_config(home, ACME_CONFIG);
            crate::templates::set_list_override(|source| match source {
                "official" => Ok(vec!["cv".to_string()]),
                _ => Err(anyhow::anyhow!("connection refused")),
            });
            let listing = collect_listing(true).unwrap();
            assert_eq!(listing.warnings.len(), 1);
            assert_eq!(listing.warnings[0].0, "acme");
            assert!(listing.official_error.is_none());
            assert_eq!(listing.available, vec![entry("cv", "official")]);
            assert!(
                warning_line("acme", "connection refused").starts_with("Warning: source 'acme':")
            );
            list(true, false).unwrap();
        });
    }

    #[test]
    fn flat_install_still_resolves() {
        with_temp_home(|home| {
            crate::templates::set_list_override(|_| Ok(Vec::new()));
            write_flat_template(home, "legacy-tpl");
            let names = source_names().unwrap();
            let candidates = templates::installed_candidates().unwrap();
            let found = templates::resolve_in("legacy-tpl", &candidates, &names).unwrap();
            assert_eq!(found.source, "official");
            assert_eq!(found.reference(), "legacy-tpl");
            // A fresh flat cache resolves from disk without touching the network.
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs();
            std::fs::write(
                home.join(".texforge/templates/legacy-tpl/.cache_meta.json"),
                format!("{{\"fetched_at\":{now}}}"),
            )
            .unwrap();
            crate::templates::set_download_override(|_name| {
                Err(anyhow::anyhow!("download must not happen"))
            });
            let resolved = templates::resolve(&found.reference()).unwrap();
            assert_eq!(resolved.files.get("main.tex").unwrap(), b"flat-content");
        });
    }

    #[test]
    fn list_json_has_name_source_and_installed() {
        with_temp_home(|home| {
            write_config(home, ACME_CONFIG);
            two_sources();
            write_flat_template(home, "thesis");
            let listing = collect_listing(true).unwrap();
            let text = serde_json::to_string(&json_items(&listing)).unwrap();
            let value: serde_json::Value = serde_json::from_str(&text).unwrap();
            let array = value.as_array().expect("top level is an array");
            assert!(!array.is_empty());
            for item in array {
                let keys: std::collections::BTreeSet<&str> = item
                    .as_object()
                    .expect("item is an object")
                    .keys()
                    .map(String::as_str)
                    .collect();
                assert_eq!(keys, ["installed", "name", "source"].into_iter().collect());
            }
            let by_name = |name: &str, source: &str| {
                array
                    .iter()
                    .find(|v| v["name"] == name && v["source"] == source)
                    .unwrap_or_else(|| panic!("missing {source}/{name} in {text}"))
                    .clone()
            };
            assert_eq!(by_name("general", "built-in")["installed"], true);
            assert_eq!(by_name("thesis", "official")["installed"], true);
            assert_eq!(by_name("acme-tpl", "acme")["installed"], false);
            assert_eq!(by_name("official-tpl", "official")["installed"], false);
        });
    }

    #[test]
    fn bare_name_offered_only_by_non_official_resolves() {
        with_temp_home(|home| {
            write_config(home, ACME_CONFIG);
            crate::templates::set_list_override(|source| match source {
                "official" => Ok(Vec::new()),
                _ => Ok(vec!["solo".to_string()]),
            });
            let names = source_names().unwrap();
            let union = templates::union_candidates().unwrap();
            assert!(union.failures.is_empty());
            let found = templates::resolve_in("solo", &union.candidates, &names).unwrap();
            assert_eq!(found.source, "acme");
            assert_eq!(found.reference(), "acme/solo");
        });
    }

    #[test]
    fn remove_qualified_deletes_only_that_source_install() {
        with_temp_home(|home| {
            write_config(home, ACME_CONFIG);
            crate::templates::set_list_override(|_| Ok(Vec::new()));
            write_flat_template(home, "dup");
            let nested = home.join(".texforge/templates/acme/dup");
            std::fs::create_dir_all(&nested).unwrap();
            std::fs::write(nested.join("template.toml"), "id = \"x\"\n").unwrap();
            remove("acme/dup").unwrap();
            assert!(!nested.exists());
            assert!(home.join(".texforge/templates/dup").is_dir());
        });
    }

    #[test]
    fn list_local_only() {
        with_temp_home(|_home| {
            // include_remote=false, human mode should succeed without network
            list(false, false).unwrap();
        });
    }

    #[test]
    fn installed_lines_prints_general_once() {
        let cached = vec![entry("general", "official"), entry("mine", "official")];
        assert_eq!(
            installed_lines(&cached),
            vec![
                "  - general (built-in)".to_string(),
                "  - mine (official)".to_string()
            ]
        );
    }

    #[test]
    fn installed_lines_without_cache_lists_only_general() {
        let cached: Vec<Entry> = vec![];
        assert_eq!(
            installed_lines(&cached),
            vec!["  - general (built-in)".to_string()]
        );
    }

    #[test]
    fn installed_lines_keeps_other_names_that_start_with_general() {
        let cached = vec![
            entry("apa-general", "official"),
            entry("general", "official"),
        ];
        assert_eq!(
            installed_lines(&cached),
            vec![
                "  - general (built-in)".to_string(),
                "  - apa-general (official)".to_string()
            ]
        );
    }

    #[test]
    fn validate_general_has_all_required_files() {
        ensure_rustls();
        let resolved = templates::resolve("general").unwrap();
        assert!(resolved.files.contains_key("template.toml"));
        assert!(resolved.files.contains_key("main.tex"));
    }
}
