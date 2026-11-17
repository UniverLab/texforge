//! `texforge template` command implementation.

use anyhow::Result;

use crate::templates;

fn installed_lines(cached: &[String]) -> Vec<String> {
    let mut lines = vec!["general (built-in)".to_string()];
    for name in cached {
        if name.as_str() != "general" {
            lines.push(name.clone());
        }
    }
    lines
}

/// List available templates. By default also queries the remote registry;
/// pass `include_remote = false` (via `--local`) to list only installed ones.
pub fn list(include_remote: bool) -> Result<()> {
    let cached = templates::list_cached()?;
    let installed: std::collections::HashSet<&str> = cached.iter().map(String::as_str).collect();

    println!("Installed:");
    for line in installed_lines(&cached) {
        println!("  - {}", line);
    }

    if include_remote {
        print!("\nFetching remote registry...");
        match templates::list_remote() {
            Ok(remote) => {
                println!("\r                            \r"); // clear line
                let available: Vec<&str> = remote
                    .iter()
                    .map(String::as_str)
                    .filter(|n| *n != "general" && !installed.contains(n))
                    .collect();
                if available.is_empty() {
                    println!("Available (not installed): none");
                } else {
                    println!("Available (not installed):");
                    for name in available {
                        println!("  - {}", name);
                    }
                }
                println!("\nRun 'texforge template add <name>' to install.");
            }
            Err(e) => {
                println!("\nCould not reach registry: {}", e);
            }
        }
    }

    Ok(())
}

/// Add a template from the registry or from a local folder.
///
/// `source` is either a path to a template folder or a plain registry name.
pub fn add(source: &str, force: bool) -> Result<()> {
    if std::path::Path::new(source).is_dir() {
        println!("Installing template from '{}'...", source);
        let id = templates::install_from_dir(std::path::Path::new(source), force)?;
        println!("  ◇ Template '{}' installed", id);
        return Ok(());
    }

    if !templates::is_plain_name(source) {
        anyhow::bail!(
            "'{}' is neither a local folder nor a registry template name. \
             Use a registry name (texforge template list) or a path to a template folder.",
            source
        );
    }

    let was_local = templates::installed_from_local(source)?;
    if was_local && !force {
        anyhow::bail!(
            "Template '{}' is installed from a local folder; remove it first or pass --force",
            source
        );
    }

    println!("Downloading template '{}'...", source);
    templates::download(source)?;
    if was_local {
        // Only drop the marker once the registry copy is on disk: a failed
        // download must leave the local install exactly as it was.
        templates::clear_local_marker(source)?;
    }
    println!("  ◇ Template '{}' installed", source);
    Ok(())
}

/// Remove a template from local cache.
pub fn remove(name: &str) -> Result<()> {
    let path = templates::remove_cached(name)?;
    println!("  ◇ Removed template '{}' ({})", name, path.display());
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

/// Refresh cached templates, bypassing the TTL.
pub fn refresh(name: Option<&str>) -> Result<()> {
    match name {
        Some(n) => {
            println!("Refreshing template '{}'...", n);
            templates::refresh(n)?;
            println!("  ◇ Template '{}' refreshed", n);
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
    fn add_rejects_a_path_that_is_not_a_folder_nor_a_name() {
        for source in ["a/b", "https://x.y/z"] {
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
    fn add_installs_a_local_folder_and_needs_force_to_replace_it() {
        with_temp_home(|home| {
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
            crate::templates::clear_download_override();

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
            crate::templates::clear_download_override();

            assert!(err.contains("network is down"), "{err}");
            let installed = home.join(".texforge/templates/lab-report");
            assert!(installed.join(".local_source").is_file());
            assert_eq!(std::fs::read(installed.join("main.tex")).unwrap(), b"local");
        });
    }

    #[test]
    fn list_local_only() {
        // include_remote=false should succeed without network
        list(false).unwrap();
    }

    #[test]
    fn list_cached_returns_installed() {
        let cached = templates::list_cached().unwrap();
        // Should be a Vec (possibly empty)
        let _ = cached;
    }

    #[test]
    fn installed_lines_prints_general_once() {
        let cached = vec!["general".to_string(), "mine".to_string()];
        assert_eq!(
            installed_lines(&cached),
            vec!["general (built-in)".to_string(), "mine".to_string()]
        );
    }

    #[test]
    fn installed_lines_without_cache_lists_only_general() {
        let cached: Vec<String> = vec![];
        assert_eq!(
            installed_lines(&cached),
            vec!["general (built-in)".to_string()]
        );
    }

    #[test]
    fn installed_lines_keeps_other_names_that_start_with_general() {
        let cached = vec!["apa-general".to_string(), "general".to_string()];
        assert_eq!(
            installed_lines(&cached),
            vec!["general (built-in)".to_string(), "apa-general".to_string()]
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
