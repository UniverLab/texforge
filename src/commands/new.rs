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

    // Create the project directory before resolving placeholders: the author
    // fallback runs `git config --get user.name` in the target directory and
    // git needs a real path to start from.
    std::fs::create_dir_all(project_dir)?;

    // Resolve any placeholders the template declares (defaults, project/user
    // config). Missing values are left as-is rather than failing generation.
    // The project name doubles as the document title unless overridden.
    let mut cli_args = HashMap::new();
    cli_args.insert("title".to_string(), name.to_string());
    let values = resolve_placeholder_values(&resolved.files, cli_args, project_dir);

    // Write all template files into the (now existing) project directory
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
    let author = values.get("author").map_or("Author", String::as_str);
    let project_toml = format!(
        r#"[document]
title = "{name}"
author = "{author}"
template = "{template_name}"

[build]
entry = "main.tex"
bibliography = "bib/references.bib"
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
/// without declared placeholders. `project_dir` is where the git identity
/// fallback (`git config --get user.name`) runs.
fn resolve_placeholder_values(
    files: &HashMap<String, Vec<u8>>,
    cli_args: HashMap<String, String>,
    project_dir: &Path,
) -> HashMap<String, String> {
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

    let resolver = PlaceholderResolver::new_in(cli_args, project_dir);
    for ph in &manifest.placeholders {
        if let Ok(Some(value)) = resolver.resolve(ph) {
            values.insert(ph.name.clone(), value);
        }
    }
    values
}

/// Replace `{{name}}` tokens with resolved values. Unresolved non-identity
/// tokens are left untouched (lenient — never fails generation); any
/// remaining `{{user.*}}` / `{{institution.*}}` token is cleared to the
/// empty string so no raw identity placeholder reaches a generated file.
fn apply_substitutions(content: &str, values: &HashMap<String, String>) -> String {
    let mut out = content.to_string();
    for (key, value) in values {
        out = out.replace(&format!("{{{{{}}}}}", key), value);
    }
    crate::placeholders::clear_unresolved_identities(&out)
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

    #[test]
    fn name_with_backslash_is_error() {
        assert!(validate_project_name("a\\b").is_err());
    }

    #[test]
    fn name_with_absolute_path_is_error() {
        assert!(validate_project_name("/etc/passwd").is_err());
    }

    #[test]
    fn name_with_special_char_is_error() {
        assert!(validate_project_name("project@name").is_err());
        assert!(validate_project_name("project#1").is_err());
        assert!(validate_project_name("project$").is_err());
        assert!(validate_project_name("project!").is_err());
        assert!(validate_project_name("project&test").is_err());
        assert!(validate_project_name("project|test").is_err());
        assert!(validate_project_name("project;test").is_err());
        assert!(validate_project_name("project`test").is_err());
        assert!(validate_project_name("project\"test").is_err());
        assert!(validate_project_name("project'test").is_err());
        assert!(validate_project_name("project*test").is_err());
        assert!(validate_project_name("project?test").is_err());
    }

    #[test]
    fn name_with_only_whitespace_is_error() {
        assert!(validate_project_name("   ").is_err());
        assert!(validate_project_name("\t").is_err());
    }

    #[test]
    fn apply_substitutions_replaces_tokens() {
        let mut values = HashMap::new();
        values.insert("title".to_string(), "My Doc".to_string());
        values.insert("author".to_string(), "Jane".to_string());

        let content = "\\title{{{title}}}\n\\author{{{author}}}";
        let result = apply_substitutions(content, &values);
        assert_eq!(result, "\\title{My Doc}\n\\author{Jane}");
    }

    #[test]
    fn apply_substitutions_leaves_unmatched_tokens() {
        let values = HashMap::new();
        let content = "\\title{{{title}}}";
        let result = apply_substitutions(content, &values);
        assert_eq!(result, "\\title{{{title}}}");
    }

    #[test]
    fn apply_substitutions_empty_content() {
        let values = HashMap::new();
        let result = apply_substitutions("", &values);
        assert_eq!(result, "");
    }

    #[test]
    fn apply_substitutions_multiple_same_token() {
        let mut values = HashMap::new();
        values.insert("x".to_string(), "Y".to_string());
        let result = apply_substitutions("{{x}} and {{x}}", &values);
        assert_eq!(result, "Y and Y");
    }

    #[test]
    fn resolve_placeholder_values_empty_files() {
        let files = HashMap::new();
        let cli_args = HashMap::new();
        let result = resolve_placeholder_values(&files, cli_args, Path::new("."));
        assert!(result.is_empty());
    }

    #[test]
    fn resolve_placeholder_values_invalid_toml() {
        let mut files = HashMap::new();
        files.insert("template.toml".to_string(), b"not valid {{{ toml".to_vec());
        let cli_args = HashMap::new();
        let result = resolve_placeholder_values(&files, cli_args, Path::new("."));
        assert!(result.is_empty());
    }

    #[test]
    fn resolve_placeholder_values_non_utf8() {
        let mut files = HashMap::new();
        files.insert("template.toml".to_string(), vec![0xFF, 0xFE]);
        let cli_args = HashMap::new();
        let result = resolve_placeholder_values(&files, cli_args, Path::new("."));
        assert!(result.is_empty());
    }

    #[test]
    fn name_with_hyphens_is_ok() {
        assert!(validate_project_name("my-tesis-v2").is_ok());
    }

    #[test]
    fn name_with_underscores_is_ok() {
        assert!(validate_project_name("my_tesis").is_ok());
    }

    #[test]
    fn name_with_dots_is_ok() {
        assert!(validate_project_name("my.tesis").is_ok());
    }

    #[test]
    fn name_with_single_char_is_ok() {
        assert!(validate_project_name("a").is_ok());
    }

    /// Runs `git` with `args` inside `dir`, asserting it succeeds.
    fn run_git(args: &[&str], dir: &Path) {
        let out = std::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&out.stderr)
        );
    }

    /// Generates `demo` from the embedded general template under a fully
    /// isolated environment and hands the project directory to `body`.
    ///
    /// - `HOME` / `XDG_CONFIG_HOME` point at a fresh tempdir: empty texforge
    ///   config and empty template cache, nothing from the developer's real
    ///   `~/.texforge`.
    /// - `GIT_CONFIG_GLOBAL` / `GIT_CONFIG_SYSTEM` point at an empty file, so
    ///   the developer's real git identity cannot leak in.
    /// - With `Some(name)`, the tempdir becomes a git repo whose local
    ///   `user.name` is that value.
    /// - The download override makes the network step of
    ///   `templates::resolve("general")` fail deterministically, so the
    ///   embedded template is used even when the sandbox has network.
    ///
    /// Lock order stays ENV then CWD, matching the rest of the suite.
    fn with_generated_project(git_name: Option<&str>, body: impl FnOnce(&Path)) {
        let _env = crate::test_sync::ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        let empty_git_config = root.path().join("empty.gitconfig");
        std::fs::write(&empty_git_config, "").unwrap();
        // Declared after the lock: locals drop in reverse declaration order,
        // so the environment is restored while ENV_LOCK is still held.
        let _restore = crate::test_sync::EnvGuard::capture(&[
            "HOME",
            "XDG_CONFIG_HOME",
            "GIT_CONFIG_GLOBAL",
            "GIT_CONFIG_SYSTEM",
        ]);
        std::env::set_var("HOME", &home);
        std::env::set_var("XDG_CONFIG_HOME", &home);
        std::env::set_var("GIT_CONFIG_GLOBAL", &empty_git_config);
        std::env::set_var("GIT_CONFIG_SYSTEM", &empty_git_config);

        if let Some(git_name) = git_name {
            run_git(&["init"], root.path());
            run_git(&["config", "user.name", git_name], root.path());
        }

        let _cwd = crate::test_sync::CWD_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let orig = std::env::current_dir().unwrap();
        std::env::set_current_dir(root.path()).unwrap();
        crate::templates::set_download_override(|_| {
            Err(anyhow::anyhow!("network disabled in tests"))
        });
        let result = execute("demo", Some("general"));
        crate::templates::clear_download_override();
        std::env::set_current_dir(orig).unwrap();

        result.unwrap();
        body(&root.path().join("demo"));
    }

    #[test]
    fn generated_general_project_inputs_body_without_inputenc() {
        with_generated_project(None, |project| {
            let main = std::fs::read_to_string(project.join("main.tex")).unwrap();
            assert!(
                main.contains("\\input{sections/body}"),
                "main.tex must \\input the body file:\n{main}"
            );
            assert!(
                !main.contains("inputenc"),
                "main.tex must not load inputenc:\n{main}"
            );
            let body = std::fs::read_to_string(project.join("sections/body.tex")).unwrap();
            assert!(
                body.contains("\\section{Introduction}"),
                "body.tex must carry the moved skeleton:\n{body}"
            );
        });
    }

    #[test]
    fn author_falls_back_to_repo_local_git_user_name() {
        with_generated_project(Some("Ada Lovelace"), |project| {
            let manifest = std::fs::read_to_string(project.join("project.toml")).unwrap();
            assert!(
                manifest.contains("author = \"Ada Lovelace\""),
                "project.toml must pick up the git identity:\n{manifest}"
            );
        });
    }

    #[test]
    fn no_generated_file_contains_raw_placeholders_without_identity() {
        with_generated_project(None, |project| {
            for entry in walkdir::WalkDir::new(project) {
                let entry = entry.unwrap();
                if !entry.file_type().is_file() {
                    continue;
                }
                let text = std::fs::read_to_string(entry.path()).unwrap();
                assert!(
                    !text.contains("{{"),
                    "{} contains a raw placeholder:\n{text}",
                    entry.path().display()
                );
            }
        });
    }
}
