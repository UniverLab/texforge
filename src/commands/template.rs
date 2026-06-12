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
