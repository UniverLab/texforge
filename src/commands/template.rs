//! `texforge template` command implementation.

use anyhow::Result;

use crate::templates;

/// List available templates. By default also queries the remote registry;
/// pass `include_remote = false` (via `--local`) to list only installed ones.
pub fn list(include_remote: bool) -> Result<()> {
    let cached = templates::list_cached()?;
    let installed: std::collections::HashSet<&str> = cached.iter().map(String::as_str).collect();

    println!("Installed:");
    println!("  - general (built-in)");
    for name in &cached {
        println!("  - {}", name);
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

/// Add a template from the registry.
pub fn add(name: &str) -> Result<()> {
    println!("Downloading template '{}'...", name);
    templates::download(name)?;
    println!("  ◇ Template '{}' installed", name);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn ensure_rustls() {
        let _ = rustls::crypto::ring::default_provider().install_default();
    }

    #[test]
    fn validate_general_template() {
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
    fn validate_general_has_all_required_files() {
        ensure_rustls();
        let resolved = templates::resolve("general").unwrap();
        assert!(resolved.files.contains_key("template.toml"));
        assert!(resolved.files.contains_key("main.tex"));
    }
}
