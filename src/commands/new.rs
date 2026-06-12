//! `texforge new` command implementation.

use std::collections::HashMap;
use std::path::{Component, Path};

use anyhow::{Context, Result};

use crate::manifest::TemplateManifest;
use crate::placeholders::PlaceholderResolver;
use crate::templates;

/// Create a new project from a template.
pub fn execute(name: &str, template: Option<&str>) -> Result<()> {
    validate_project_name(name)?;

    let template_name = template.unwrap_or("general");
    let project_dir = Path::new(name);

    if project_dir.exists() {
        anyhow::bail!("Directory '{}' already exists", name);
    }

    println!(
        "Creating project '{}' with template '{}'...",
        name, template_name
    );

    let resolved = templates::resolve(template_name)?;

    // Resolve any placeholders the template declares (defaults, project/user
    // config). Missing values are left as-is rather than failing generation.
    let values = resolve_placeholder_values(&resolved.files);

    // Create project directory and write all template files
    for (rel_path, content) in &resolved.files {
        // Skip template.toml — it's metadata, not a project file
        if rel_path == "template.toml" {
            continue;
        }
        let dest = project_dir.join(rel_path);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // Substitute {{placeholder}} tokens in .tex files only — other files
        // (code samples, images) are copied verbatim.
        if rel_path.ends_with(".tex") {
            let text = String::from_utf8_lossy(content);
            let substituted = apply_substitutions(&text, &values);
            std::fs::write(&dest, substituted)
        } else {
            std::fs::write(&dest, content)
        }
        .with_context(|| format!("Failed to write {}", dest.display()))?;
    }

    // Generate project.toml
    let project_toml = format!(
        r#"[documento]
titulo = "{name}"
autor = "Author"
template = "{template_name}"

[compilacion]
entry = "main.tex"
bibliografia = "bib/references.bib"
"#
    );
    std::fs::write(project_dir.join("project.toml"), project_toml)?;

    // Ensure assets/images directory exists
    std::fs::create_dir_all(project_dir.join("assets/images"))?;

    println!("  ◇ Project '{}' created successfully", name);
    println!();
    println!("  cd {}", name);
    println!("  texforge build");

    Ok(())
}

/// Resolve placeholder values from a template's manifest, if present.
/// Returns an empty map for templates without a (valid) `template.toml` or
/// without declared placeholders.
fn resolve_placeholder_values(files: &HashMap<String, Vec<u8>>) -> HashMap<String, String> {
    let mut values = HashMap::new();

    let Some(toml_bytes) = files.get("template.toml") else {
        return values;
    };
    let Ok(text) = std::str::from_utf8(toml_bytes) else {
        return values;
    };
    let Ok(manifest) = TemplateManifest::from_str(text) else {
        return values;
    };

    let resolver = PlaceholderResolver::new(HashMap::new());
    for ph in &manifest.placeholders {
        if let Ok(Some(value)) = resolver.resolve(ph) {
            values.insert(ph.name.clone(), value);
        }
    }
    values
}

/// Replace `{{name}}` tokens with resolved values. Unresolved tokens are left
/// untouched (lenient — never fails generation).
fn apply_substitutions(content: &str, values: &HashMap<String, String>) -> String {
    let mut out = content.to_string();
    for (key, value) in values {
        out = out.replace(&format!("{{{{{}}}}}", key), value);
    }
    out
}

/// Validate project name: no empty, no path traversal, no special chars.
pub(crate) fn validate_project_name(name: &str) -> Result<()> {
    if name.is_empty() {
        anyhow::bail!("Project name cannot be empty");
    }

    // Reject path traversal
    let path = Path::new(name);
    for component in path.components() {
        match component {
            Component::ParentDir => {
                anyhow::bail!("Project name cannot contain '..' (path traversal)");
            }
            Component::RootDir | Component::Prefix(_) => {
                anyhow::bail!("Project name cannot be an absolute path");
            }
            _ => {}
        }
    }

    // Reject names with slashes (implicit subdirectories)
    if name.contains('/') || name.contains('\\') {
        anyhow::bail!("Project name cannot contain path separators");
    }

    // Reject names with spaces
    if name.contains(' ') {
        anyhow::bail!("Project name cannot contain spaces — use hyphens instead (e.g. 'mi-tesis')");
    }

    // Reject problematic characters
    let invalid_chars = ['@', '#', '$', '!', '&', '|', ';', '`', '"', '\'', '*', '?'];
    if let Some(c) = name.chars().find(|c| invalid_chars.contains(c)) {
        anyhow::bail!("Project name contains invalid character: '{}'", c);
    }

    // Reject names that are only whitespace
    if name.trim().is_empty() {
        anyhow::bail!("Project name cannot be only whitespace");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_name_is_error() {
        assert!(validate_project_name("").is_err());
    }

    #[test]
    fn name_with_spaces_is_error() {
        assert!(validate_project_name("my project").is_err());
    }

    #[test]
    fn name_with_dotdot_is_error() {
        assert!(validate_project_name("../evil").is_err());
    }

    #[test]
    fn name_with_slash_is_error() {
        assert!(validate_project_name("a/b").is_err());
    }

    #[test]
    fn valid_name_is_ok() {
        assert!(validate_project_name("mi-tesis").is_ok());
    }
}
