//! `texforge new` command implementation.

use std::collections::{BTreeMap, HashMap};
use std::path::{Component, Path};

use anyhow::{Context, Result};

use crate::manifest::TemplateManifest;
use crate::placeholders::PlaceholderResolver;
use crate::templates;
use crate::values_file;

/// Resolve `-t` against installed templates plus every configured remote
/// source: a bare name offered by more than one source fails naming every
/// `source/name` candidate, and an unreachable source warns once and
/// contributes nothing. Returns the reference `resolve` understands.
fn resolve_template_reference(template_name: &str) -> Result<String> {
    let names: Vec<String> = templates::load_sources()?
        .iter()
        .map(|s| s.name.clone())
        .collect();
    let mut union = templates::union_candidates()?;
    for failure in &union.failures {
        eprintln!(
            "{}",
            templates::refs::warning_line(&failure.source, &failure.reason)
        );
    }
    let candidates = &mut union.candidates;
    // The built-in `general` template is always usable, even fully offline:
    // `resolve` falls back to the embedded copy when nothing is cached.
    let builtin = templates::refs::Candidate {
        source: templates::OFFICIAL_NAME.to_string(),
        name: "general".to_string(),
    };
    if !candidates.contains(&builtin) {
        candidates.push(builtin);
    }
    Ok(templates::resolve_in(template_name, candidates, &names)?.reference())
}

/// Create a new project from a template.
///
/// `values_path` is the optional `--values` file: its keys must all be
/// placeholders the template declares, they win over every other source, and
/// it is echoed into the project as `values.toml`.
pub fn execute(name: &str, template: Option<&str>, values_path: Option<&Path>) -> Result<()> {
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

    let template_ref = resolve_template_reference(template_name)?;
    let resolved = templates::resolve(&template_ref)?;
    let manifest = template_manifest(&resolved.files);
    let file_values = load_values(values_path, manifest.as_ref(), template_name)?;

    // Create the project directory before resolving placeholders: the author
    // fallback runs `git config --get user.name` in the target directory and
    // git needs a real path to start from.
    std::fs::create_dir_all(project_dir)?;

    // Resolve any placeholders the template declares (defaults, project/user
    // config). Missing values are left as-is rather than failing generation.
    // The project name doubles as the document title unless the values file
    // overrides it.
    let mut cli_args: HashMap<String, String> = file_values
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    cli_args
        .entry("title".to_string())
        .or_insert_with(|| name.to_string());
    let values = resolve_placeholder_values(&resolved.files, cli_args, project_dir);

    // A required placeholder with no value fails generation instead of leaving
    // a raw `{{name}}` in the project. Remove the directory this run created.
    let missing = missing_required(manifest.as_ref(), &values);
    if !missing.is_empty() {
        let _ = std::fs::remove_dir_all(project_dir);
        let list = missing
            .iter()
            .map(|(key, description)| format!("  - {key}: {description}"))
            .collect::<Vec<_>>()
            .join("\n");
        anyhow::bail!("Required placeholders have no value:\n{list}");
    }

    write_project_files(&resolved.files, &values, project_dir)?;

    if values_path.is_some() {
        std::fs::write(
            project_dir.join("values.toml"),
            values_file::render(&file_values),
        )
        .context("Failed to write values.toml")?;
    }

    // Generate project.toml. Values are LaTeX source and may hold quotes or
    // backslashes, so every interpolated string goes through the TOML escaper:
    // a raw `author` would otherwise produce a manifest nothing can parse.
    let author = values.get("author").map_or("Author", String::as_str);
    let title = values.get("title").map_or(name, String::as_str);
    let project_toml = format!(
        r#"[document]
title = {title}
author = {author}
template = {template_name}

[build]
entry = "main.tex"
bibliography = "bib/references.bib"
"#,
        title = values_file::toml_string(title),
        author = values_file::toml_string(author),
        template_name = values_file::toml_string(template_name),
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

/// Parse a template's `template.toml`. Returns `None` when the file is
/// absent, not UTF-8 or does not parse: such a template declares no
/// placeholders at all.
fn template_manifest(files: &HashMap<String, Vec<u8>>) -> Option<TemplateManifest> {
    let text = std::str::from_utf8(files.get("template.toml")?).ok()?;
    TemplateManifest::from_str(text).ok()
}

/// Load the `--values` file, if any, and check its keys against the
/// placeholders the template declares.
fn load_values(
    values_path: Option<&Path>,
    manifest: Option<&TemplateManifest>,
    template_name: &str,
) -> Result<BTreeMap<String, String>> {
    let Some(path) = values_path else {
        return Ok(BTreeMap::new());
    };
    let values = values_file::load(path)?;
    validate_values_keys(manifest, &values, template_name)?;
    Ok(values)
}

/// Every key of the values file must be a placeholder the template declares,
/// otherwise it would silently do nothing. A template without a parsable
/// manifest declares nothing, so every key is unknown there.
fn validate_values_keys(
    manifest: Option<&TemplateManifest>,
    values: &BTreeMap<String, String>,
    template_name: &str,
) -> Result<()> {
    let declared: Vec<&str> = manifest.map_or_else(Vec::new, |manifest| {
        manifest
            .placeholders
            .iter()
            .map(|placeholder| placeholder.name.as_str())
            .collect()
    });
    let unknown: Vec<&str> = values
        .keys()
        .map(String::as_str)
        .filter(|key| !declared.contains(key))
        .collect();
    if unknown.is_empty() {
        return Ok(());
    }
    let declared_list = if declared.is_empty() {
        "none".to_string()
    } else {
        declared.join(", ")
    };
    anyhow::bail!(
        "Values file defines keys template '{template_name}' does not declare: {} (declared: {declared_list})",
        unknown.join(", ")
    )
}

/// The required placeholders left without a value after resolution, as
/// `(name, description)` in declaration order.
fn missing_required<'a>(
    manifest: Option<&'a TemplateManifest>,
    values: &HashMap<String, String>,
) -> Vec<(&'a str, &'a str)> {
    let Some(manifest) = manifest else {
        return Vec::new();
    };
    manifest
        .placeholders
        .iter()
        .filter(|placeholder| placeholder.required && !values.contains_key(&placeholder.name))
        .map(|placeholder| (placeholder.name.as_str(), placeholder.description.as_str()))
        .collect()
}

/// Write all template files into the project directory, substituting
/// `{{placeholder}}` tokens in `.tex` files only — other files (code samples,
/// images) are copied verbatim.
fn write_project_files(
    files: &HashMap<String, Vec<u8>>,
    values: &HashMap<String, String>,
    project_dir: &Path,
) -> Result<()> {
    for (rel_path, content) in files {
        // Skip template.toml — it's metadata, not a project file
        if rel_path == "template.toml" {
            continue;
        }
        let dest = project_dir.join(rel_path);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        if rel_path.ends_with(".tex") {
            let text = String::from_utf8_lossy(content);
            let substituted = apply_substitutions(&text, values);
            std::fs::write(&dest, substituted)
        } else {
            std::fs::write(&dest, content)
        }
        .with_context(|| format!("Failed to write {}", dest.display()))?;
    }
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

    let Some(manifest) = template_manifest(files) else {
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

    /// Runs `texforge new` on `demo` with the embedded general template in a
    /// fully isolated environment and returns the command result plus the
    /// temporary root (so a failing test can check what was left behind).
    ///
    /// - `HOME` / `XDG_CONFIG_HOME` point at a fresh tempdir: empty texforge
    ///   config and empty template cache, nothing from the developer's real
    ///   `~/.texforge`.
    /// - `GIT_CONFIG_GLOBAL` / `GIT_CONFIG_SYSTEM` point at an empty file, so
    ///   the developer's real git identity cannot leak in.
    /// - With `Some(name)`, the tempdir becomes a git repo whose local
    ///   `user.name` is that value.
    /// - With `Some(files)`, the download override serves those template files
    ///   instead of the network; without it the override fails
    ///   deterministically, so the embedded template is used even when the
    ///   sandbox has network.
    /// - With `Some((file_name, content))`, that values file is written in the
    ///   temp root and passed as `--values`.
    ///
    /// Lock order stays ENV then CWD, matching the rest of the suite.
    fn run_new(
        git_name: Option<&str>,
        files: Option<HashMap<String, Vec<u8>>>,
        values: Option<(&str, &str)>,
    ) -> (Result<()>, tempfile::TempDir) {
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

        let values_path = values.map(|(file_name, content)| {
            let path = root.path().join(file_name);
            std::fs::write(&path, content).unwrap();
            path
        });

        let _cwd = crate::test_sync::CWD_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let orig = std::env::current_dir().unwrap();
        std::env::set_current_dir(root.path()).unwrap();
        match files {
            Some(files) => crate::templates::set_download_override(move |_| Ok(files.clone())),
            None => crate::templates::set_download_override(|_| {
                Err(anyhow::anyhow!("network disabled in tests"))
            }),
        }
        crate::templates::set_list_override(|_| Err(anyhow::anyhow!("network disabled in tests")));
        let result = execute("demo", Some("general"), values_path.as_deref());
        crate::templates::clear_download_override();
        crate::templates::clear_list_override();
        std::env::set_current_dir(orig).unwrap();

        (result, root)
    }

    /// Runs `new` successfully on the embedded general template and hands the
    /// generated project directory to `body`.
    fn with_generated_project(git_name: Option<&str>, body: impl FnOnce(&Path)) {
        let (result, root) = run_new(git_name, None, None);
        result.unwrap();
        body(&root.path().join("demo"));
    }

    /// Template files for a `contract` template declaring `placeholders`
    /// (manifest snippets) and no `main.tex` tokens.
    fn contract_template(placeholders: &str) -> HashMap<String, Vec<u8>> {
        let manifest = format!(
            r#"
id = "contract"
version = "0.1.0"
display_name = "Contract"
description = "A contract generated from data"

{placeholders}
"#
        );
        let mut files = HashMap::new();
        files.insert("template.toml".to_string(), manifest.into_bytes());
        files.insert(
            "main.tex".to_string(),
            b"\\documentclass{article}\n\\begin{document}\nok\n\\end{document}\n".to_vec(),
        );
        files
    }

    const TWO_REQUIRED: &str = r#"
[[placeholders]]
name = "client"
type = "string"
description = "Client legal name"
required = true

[[placeholders]]
name = "reference"
type = "string"
description = "Contract reference"
required = true
"#;

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

    #[test]
    fn toml_and_yaml_values_generate_the_same_main_tex() {
        let toml = "title = \"Contract\"\nauthor = \"Acme Ltd\"\nlanguage = \"english\"\n";
        let yaml = "title: Contract\nauthor: Acme Ltd\nlanguage: english\n";
        let (toml_result, toml_root) = run_new(None, None, Some(("client.toml", toml)));
        let (yaml_result, yaml_root) = run_new(None, None, Some(("client.yaml", yaml)));
        toml_result.unwrap();
        yaml_result.unwrap();

        let from_toml = std::fs::read_to_string(toml_root.path().join("demo/main.tex")).unwrap();
        let from_yaml = std::fs::read_to_string(yaml_root.path().join("demo/main.tex")).unwrap();
        assert_eq!(from_toml, from_yaml);
        assert!(
            from_toml.contains("\\title{Contract}"),
            "the values file must fill the title:\n{from_toml}"
        );
        assert!(
            from_toml.contains("\\author{Acme Ltd}"),
            "the values file must fill the author:\n{from_toml}"
        );
    }

    #[test]
    fn values_file_wins_over_the_template_default() {
        let (result, root) = run_new(
            None,
            None,
            Some(("client.toml", "language = \"spanish\"\n")),
        );
        result.unwrap();
        let main = std::fs::read_to_string(root.path().join("demo/main.tex")).unwrap();
        assert!(
            main.contains("\\usepackage[spanish]{babel}"),
            "the file value must beat the template default 'english':\n{main}"
        );
    }

    #[test]
    fn title_key_in_the_values_file_overrides_the_project_name() {
        let (result, root) = run_new(
            None,
            None,
            Some(("client.toml", "title = \"Service Agreement\"\n")),
        );
        result.unwrap();
        let main = std::fs::read_to_string(root.path().join("demo/main.tex")).unwrap();
        assert!(
            main.contains("\\title{Service Agreement}"),
            "the values file title must override the project name:\n{main}"
        );
        assert!(
            !main.contains("\\title{demo}"),
            "the project name must not win over the values file:\n{main}"
        );
    }

    #[test]
    fn latex_characters_in_a_value_reach_the_output_unchanged() {
        let (result, root) = run_new(
            None,
            None,
            Some((
                "client.toml",
                "title = \"A \\\\& B 50\\\\% \\\\textbf{x}_{y} {braces}\"\n",
            )),
        );
        result.unwrap();
        let main = std::fs::read_to_string(root.path().join("demo/main.tex")).unwrap();
        assert!(
            main.contains("\\title{A \\& B 50\\% \\textbf{x}_{y} {braces}}"),
            "values are LaTeX source and must pass through verbatim:\n{main}"
        );
    }

    /// A LaTeX `author` reaches `main.tex` verbatim *and* leaves a parsable
    /// `project.toml`: interpolating it raw yields an unreadable manifest that
    /// breaks every later `texforge build`.
    #[test]
    fn latex_author_keeps_project_toml_parsable() {
        let (result, root) = run_new(
            None,
            None,
            Some(("client.toml", "author = \"A \\\\& \\\"B\\\" \"\n")),
        );
        result.unwrap();
        let project = root.path().join("demo");
        let manifest = std::fs::read_to_string(project.join("project.toml")).unwrap();
        let parsed: toml::Table = toml::from_str(&manifest).unwrap_or_else(|error| {
            panic!("project.toml must parse ({error}):\n{manifest}");
        });
        assert_eq!(
            parsed["document"]["author"].as_str(),
            Some("A \\& \"B\" "),
            "the author must survive into project.toml:\n{manifest}"
        );
        let main = std::fs::read_to_string(project.join("main.tex")).unwrap();
        assert!(
            main.contains("\\author{A \\& \"B\" }"),
            "the author must reach main.tex verbatim:\n{main}"
        );
    }

    #[test]
    fn values_title_reaches_project_toml() {
        let (result, root) = run_new(
            None,
            None,
            Some(("client.toml", "title = \"My \\\"Great\\\" Report\"\n")),
        );
        result.unwrap();
        let project = root.path().join("demo");
        let manifest = std::fs::read_to_string(project.join("project.toml")).unwrap();
        let parsed: toml::Table = toml::from_str(&manifest).unwrap_or_else(|error| {
            panic!("project.toml must parse ({error}):\n{manifest}");
        });
        assert_eq!(
            parsed["document"]["title"].as_str(),
            Some("My \"Great\" Report"),
            "the values title must reach project.toml:\n{manifest}"
        );
    }

    #[test]
    fn project_toml_title_defaults_to_the_project_name() {
        let (result, root) = run_new(None, None, None);
        result.unwrap();
        let project = root.path().join("demo");
        let manifest = std::fs::read_to_string(project.join("project.toml")).unwrap();
        let parsed: toml::Table = toml::from_str(&manifest).unwrap_or_else(|error| {
            panic!("project.toml must parse ({error}):\n{manifest}");
        });
        assert_eq!(
            parsed["document"]["title"].as_str(),
            Some("demo"),
            "without a values title the project name is the title:\n{manifest}"
        );
    }

    #[test]
    fn title_with_a_backslash_round_trips_into_project_toml() {
        let (result, root) = run_new(None, None, Some(("client.toml", "title = \"A \\\\ B\"\n")));
        result.unwrap();
        let project = root.path().join("demo");
        let manifest = std::fs::read_to_string(project.join("project.toml")).unwrap();
        let parsed: toml::Table = toml::from_str(&manifest).unwrap_or_else(|error| {
            panic!("project.toml must parse ({error}):\n{manifest}");
        });
        assert_eq!(
            parsed["document"]["title"].as_str(),
            Some("A \\ B"),
            "a backslash title must round-trip through project.toml:\n{manifest}"
        );
    }

    #[test]
    fn unknown_values_key_lists_the_unknown_and_declared_names_and_creates_no_directory() {
        let (result, root) = run_new(None, None, Some(("client.toml", "client = \"Acme\"\n")));
        let error = result.unwrap_err().to_string();
        assert!(error.contains("client"), "{error}");
        assert!(
            error.contains("title"),
            "declared names must be listed: {error}"
        );
        assert!(
            error.contains("language"),
            "declared names must be listed: {error}"
        );
        assert!(
            !root.path().join("demo").exists(),
            "no project directory may be created for an unknown key"
        );
    }

    #[test]
    fn a_template_without_a_manifest_declares_no_known_key() {
        let mut files = HashMap::new();
        files.insert("main.tex".to_string(), b"\\documentclass{article}".to_vec());
        let (result, root) = run_new(None, Some(files), Some(("client.toml", "title = \"X\"\n")));
        let error = result.unwrap_err().to_string();
        assert!(error.contains("title"), "{error}");
        assert!(error.contains("none"), "{error}");
        assert!(!root.path().join("demo").exists());
    }

    #[test]
    fn two_missing_required_placeholders_are_both_reported_and_the_directory_removed() {
        let (result, root) = run_new(None, Some(contract_template(TWO_REQUIRED)), None);
        let error = result.unwrap_err().to_string();
        assert!(error.contains("client"), "{error}");
        assert!(error.contains("Client legal name"), "{error}");
        assert!(error.contains("reference"), "{error}");
        assert!(error.contains("Contract reference"), "{error}");
        assert!(
            !root.path().join("demo").exists(),
            "the directory created by this run must be removed"
        );
    }

    #[test]
    fn required_placeholders_satisfied_by_the_values_file_pass() {
        let values = "client = \"Acme Ltd\"\nreference = \"C-2026-001\"\n";
        let (result, root) = run_new(
            None,
            Some(contract_template(TWO_REQUIRED)),
            Some(("client.toml", values)),
        );
        result.unwrap();
        assert!(root.path().join("demo/main.tex").is_file());
    }

    #[test]
    fn values_toml_is_written_sorted_with_the_header_and_escaped_lines() {
        let values = "title = \"Contract\"\nauthor = \"Ada \\\"A\\\" \\\\ Co\"\n";
        let (result, root) = run_new(None, None, Some(("client.toml", values)));
        result.unwrap();
        let written = std::fs::read_to_string(root.path().join("demo/values.toml")).unwrap();
        let lines: Vec<&str> = written.lines().collect();
        assert_eq!(
            lines[0],
            "# Values used by texforge new; run texforge new with --values on this file to regenerate."
        );
        assert_eq!(lines[1], "author = \"Ada \\\"A\\\" \\\\ Co\"");
        assert_eq!(lines[2], "title = \"Contract\"");
        assert_eq!(
            lines.len(),
            3,
            "only the values file keys are written:\n{written}"
        );
    }

    #[test]
    fn no_values_toml_without_the_flag() {
        with_generated_project(None, |project| {
            assert!(
                !project.join("values.toml").exists(),
                "values.toml must only exist when --values is used"
            );
        });
    }
}
