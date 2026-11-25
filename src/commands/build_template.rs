//! `texforge build --values` for template folders.
//!
//! A folder carrying a `template.toml` (with or without a `project.toml`)
//! builds to a filled PDF: placeholders are rendered into a temporary copy
//! outside the source tree, then the normal build pipeline runs on that
//! staging copy. No existing file of the source folder is modified; the only
//! thing written is the PDF, in the current directory unless `--out` says
//! otherwise.

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result};

use crate::commands::{build, new};
use crate::domain::project::{BuildConfig, DocumentConfig, Project, ProjectConfig};
use crate::manifest::TemplateManifest;
use crate::utils::sanitize_filename;
use crate::values_file;

/// Top-level directories never staged into the temporary copy.
const SKIPPED_DIRS: &[&str] = &[".git", "build", ".texforge"];

/// Build the template folder `dir` (default: current directory) to a filled PDF.
///
/// `values_paths` holds every `--values` file in command-line order: they
/// are read and merged key by key before validation, exactly as in
/// `texforge new --values`; `out` overrides the PDF destination.
pub fn execute(
    dir: Option<&Path>,
    values_paths: &[PathBuf],
    out: Option<&Path>,
    verbose: bool,
    reproducible: Option<Option<u64>>,
    log: Option<&Path>,
    max_overfull: Option<f32>,
) -> Result<()> {
    if values_paths.is_empty() {
        anyhow::bail!("--out requires --values")
    };
    let cwd = std::env::current_dir()?;
    let root = dir.map(|d| cwd.join(d)).unwrap_or_else(|| cwd.clone());
    let dir_label = dir.unwrap_or(&root);
    if !root.is_dir() {
        anyhow::bail!(
            "Template folder not found or not a directory: {}",
            dir_label.display()
        );
    }
    let root_canon = root
        .canonicalize()
        .with_context(|| format!("Failed to resolve {}", dir_label.display()))?;

    let manifest_path = root.join("template.toml");
    if !manifest_path.exists() {
        let where_label = dir
            .map(|d| format!("'{}'", d.display()))
            .unwrap_or_else(|| "the current folder".to_string());
        if root.join("project.toml").exists() {
            anyhow::bail!("--values needs a template.toml in {where_label} to fill");
        }
        anyhow::bail!(
            "--values needs a template.toml in {where_label} to fill \
             (no project.toml or template.toml found)"
        );
    }
    let manifest = TemplateManifest::from_file(&manifest_path)?;

    let (file_values, origins) = values_file::load_many(values_paths)?;
    new::validate_values_keys(Some(&manifest), &file_values, &manifest.id, &origins)?;
    let cli_args: HashMap<String, String> = file_values
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    let files = HashMap::from([("template.toml".to_string(), std::fs::read(&manifest_path)?)]);
    let resolved = new::resolve_placeholder_values(&files, cli_args, &root);
    let missing = new::missing_required(Some(&manifest), &resolved);
    if !missing.is_empty() {
        let list = missing
            .iter()
            .map(|(key, description)| {
                format!(
                    "  - {key}: {description} {}",
                    new::origin_note(key, &origins)
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        let sources = values_paths
            .iter()
            .map(|path| format!("'{}'", path.display()))
            .collect::<Vec<_>>()
            .join(", ");
        anyhow::bail!("Required placeholders have no value (values files {sources}):\n{list}");
    }

    // Resolve the PDF destination before staging or compiling.
    let pdf_dest = resolve_pdf_dest(&root_canon, &cwd, out)?;

    let stage = stage_rendered_copy(&root_canon, values_paths, &resolved)?;

    let config = if root.join("project.toml").exists() {
        Project::load_from(&root)?.config
    } else {
        ProjectConfig {
            document: DocumentConfig {
                title: manifest_title(&manifest),
                author: String::new(),
                template: manifest.id.clone(),
            },
            build: BuildConfig {
                entry: "main.tex".to_string(),
                bibliography: None,
                reproducible: None,
            },
            diagrams: None,
            highlight: None,
        }
    };
    let project = Project::from_parts(stage.path().to_path_buf(), config);
    println!("Building project: {}", project.config.document.title);

    let epoch = build::resolve_epoch(reproducible, project.config.build.reproducible);
    if epoch.is_some() {
        println!("  ◇ reproducible build (SOURCE_DATE_EPOCH pinned)");
    }

    let build_dir = tempfile::tempdir()?;
    println!("  ◇ temp: {}", build_dir.path().display());
    let log_path = build::resolve_log_path(log, pdf_dest.with_extension("log"));
    let (pdf_src, warnings, _raw) =
        build::compile_project(&project, build_dir.path(), verbose, epoch, &log_path)?;

    if let Some(parent) = pdf_dest.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create {}", parent.display()))?;
        }
    }
    std::fs::copy(&pdf_src, &pdf_dest)?;
    println!("wrote {}", pdf_dest.display());

    build::enforce_max_overfull(&warnings, max_overfull)?;

    Ok(())
}

/// The `[document] title` for a folder with only a `template.toml`: the
/// manifest's display name, falling back to its id.
fn manifest_title(manifest: &TemplateManifest) -> String {
    if manifest.display_name.is_empty() {
        manifest.id.clone()
    } else {
        manifest.display_name.clone()
    }
}

/// Where the filled PDF goes: `--out`, or `<folder-name>.pdf` in the current
/// directory (where the command runs). Relative `--out` paths resolve against
/// the current directory too. The staged copy is what compiles, so the PDF
/// written next to the sources never feeds back into a later build (`*.pdf`
/// is skipped when staging).
fn resolve_pdf_dest(root_canon: &Path, cwd: &Path, out: Option<&Path>) -> Result<PathBuf> {
    match out {
        Some(path) => {
            let absolute = if path.is_absolute() {
                path.to_path_buf()
            } else {
                cwd.join(path)
            };
            Ok(lexical_normalize(&absolute))
        }
        None => {
            let folder = root_canon
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("");
            if folder.is_empty() {
                anyhow::bail!("Cannot derive the PDF name from the template folder");
            }
            Ok(cwd.join(format!("{}.pdf", sanitize_filename(folder))))
        }
    }
}

/// Collapse `.`, `..` and duplicate separators lexically (no filesystem
/// access, so destinations whose parents do not exist yet still resolve).
fn lexical_normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            _ => out.push(component.as_os_str()),
        }
    }
    out
}

/// Copy the template folder into a temporary directory outside the source
/// tree, substituting placeholders in `.tex` files and skipping
/// `template.toml`, `.git/`, `build/`, `.texforge/`, every values file and
/// every `*.pdf`. All other files are copied byte-for-byte.
pub(crate) fn stage_rendered_copy(
    root_canon: &Path,
    values_paths: &[PathBuf],
    values: &HashMap<String, String>,
) -> Result<tempfile::TempDir> {
    let stage = tempfile::tempdir()?;
    let mut values_rels = Vec::new();
    for values_path in values_paths {
        let values_abs = if values_path.is_absolute() {
            values_path.to_path_buf()
        } else {
            std::env::current_dir()?.join(values_path)
        };
        // The values files exist (they were just loaded), so resolve
        // symlinks: a purely lexical prefix check misses an inside-folder
        // values file when the folder itself sits under a symlinked prefix.
        let values_abs = values_abs
            .canonicalize()
            .unwrap_or_else(|_| lexical_normalize(&values_abs));
        if let Some(values_rel) = values_abs
            .strip_prefix(root_canon)
            .ok()
            .map(Path::to_path_buf)
        {
            values_rels.push(values_rel);
        }
    }
    stage_dir(root_canon, root_canon, stage.path(), &values_rels, values)?;
    Ok(stage)
}

/// Mirror `source` (a directory inside the template folder) into `dest`,
/// rendering `.tex` files with `values`.
fn stage_dir(
    root_canon: &Path,
    source: &Path,
    dest: &Path,
    values_rels: &[PathBuf],
    values: &HashMap<String, String>,
) -> Result<()> {
    let entries = std::fs::read_dir(source)
        .with_context(|| format!("Failed to read {}", source.display()))?;
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        let relative = path.strip_prefix(root_canon).with_context(|| {
            format!(
                "Failed to resolve {} inside the template folder",
                path.display()
            )
        })?;
        if is_skipped(relative, values_rels, &entry.file_type()?) {
            continue;
        }
        let target = dest.join(path.file_name().unwrap());
        if entry.file_type()?.is_dir() {
            std::fs::create_dir_all(&target)?;
            stage_dir(root_canon, &path, &target, values_rels, values)?;
        } else {
            if path.extension().and_then(|e| e.to_str()) == Some("tex") {
                let text = std::fs::read(&path)?;
                let rendered = new::apply_substitutions(&String::from_utf8_lossy(&text), values);
                std::fs::write(&target, rendered)?;
            } else {
                std::fs::copy(&path, &target)?;
            }
        }
    }
    Ok(())
}

/// True for every path the staging copy leaves behind: the top-level
/// metadata and cache directories, `template.toml`, the values files and
/// every `*.pdf`. `file_type` is already known so symlinks (never regular
/// files) are skipped too.
fn is_skipped(relative: &Path, values_rels: &[PathBuf], file_type: &std::fs::FileType) -> bool {
    if !file_type.is_file() && !file_type.is_dir() {
        return true;
    }
    let mut components = relative.components();
    if let Some(Component::Normal(top)) = components.next() {
        if file_type.is_dir() && SKIPPED_DIRS.contains(&top.to_string_lossy().as_ref()) {
            return true;
        }
        if relative == Path::new("template.toml") {
            return true;
        }
        if file_type.is_file() && values_rels.iter().any(|rel| relative == rel) {
            return true;
        }
    }
    file_type.is_file() && relative.extension().and_then(|e| e.to_str()) == Some("pdf")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A template folder exercising every staging rule: a `.tex` file with
    /// placeholders, a verbatim asset, and one of each skipped path.
    fn staging_fixture() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("template.toml"), "[template]\nid = \"fixture\"\n").unwrap();
        std::fs::write(root.join("main.tex"), "\\title{{client}} v{{version}}\n").unwrap();
        std::fs::write(root.join("notes.txt"), "keep {{client}} verbatim\n").unwrap();
        std::fs::create_dir_all(root.join("assets")).unwrap();
        std::fs::write(root.join("assets/logo.bin"), b"\x00\x01\x02").unwrap();
        std::fs::write(root.join("values.toml"), "client = \"Acme\"\n").unwrap();
        std::fs::write(root.join("stale.pdf"), b"%PDF-stale").unwrap();
        std::fs::create_dir_all(root.join(".git/objects")).unwrap();
        std::fs::write(root.join(".git/objects/x"), b"git").unwrap();
        std::fs::create_dir_all(root.join("build")).unwrap();
        std::fs::write(root.join("build/main.pdf"), b"%PDF-build").unwrap();
        std::fs::create_dir_all(root.join(".texforge")).unwrap();
        std::fs::write(root.join(".texforge/config.toml"), b"cached").unwrap();
        dir
    }

    fn staging_values() -> HashMap<String, String> {
        HashMap::from([
            ("client".to_string(), "Acme".to_string()),
            ("version".to_string(), "2".to_string()),
        ])
    }

    #[test]
    fn staging_substitutes_tex_and_skips_metadata() {
        let dir = staging_fixture();
        let root_canon = dir.path().canonicalize().unwrap();
        let values_file = root_canon.join("values.toml");
        let stage = stage_rendered_copy(
            &root_canon,
            std::slice::from_ref(&values_file),
            &staging_values(),
        )
        .unwrap();
        let staged = stage.path();

        assert_eq!(
            std::fs::read_to_string(staged.join("main.tex")).unwrap(),
            "\\titleAcme v2\n"
        );
        assert_eq!(
            std::fs::read_to_string(staged.join("notes.txt")).unwrap(),
            "keep {{client}} verbatim\n"
        );
        assert_eq!(
            std::fs::read(staged.join("assets/logo.bin")).unwrap(),
            b"\x00\x01\x02"
        );
        for skipped in [
            "template.toml",
            "values.toml",
            "stale.pdf",
            ".git",
            "build",
            ".texforge",
        ] {
            assert!(
                !staged.join(skipped).exists(),
                "{skipped} must not be staged"
            );
        }
    }

    #[test]
    fn staging_skips_values_file_by_absolute_path() {
        let dir = staging_fixture();
        let root_canon = dir.path().canonicalize().unwrap();
        let values_abs = root_canon.join("values.toml");
        let stage = stage_rendered_copy(
            &root_canon,
            std::slice::from_ref(&values_abs),
            &staging_values(),
        )
        .unwrap();
        assert!(!stage.path().join("values.toml").exists());
        assert!(stage.path().join("main.tex").exists());
    }

    #[test]
    fn staging_skips_nested_pdfs_but_keeps_tex() {
        let dir = staging_fixture();
        std::fs::write(dir.path().join("assets/old.pdf"), b"%PDF-old").unwrap();
        let root_canon = dir.path().canonicalize().unwrap();
        let values_file = root_canon.join("values.toml");
        let stage = stage_rendered_copy(
            &root_canon,
            std::slice::from_ref(&values_file),
            &staging_values(),
        )
        .unwrap();
        assert!(!stage.path().join("assets/old.pdf").exists());
        assert!(stage.path().join("main.tex").exists());
    }

    #[test]
    fn default_dest_is_cwd_and_relative_out_resolves_against_it() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("sub/mutual-nda");
        let cwd = dir.path().join("caller");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&cwd).unwrap();
        let root_canon = root.canonicalize().unwrap();

        assert_eq!(
            resolve_pdf_dest(&root_canon, &cwd, None).unwrap(),
            cwd.join("mutual-nda.pdf")
        );
        assert_eq!(
            resolve_pdf_dest(&root_canon, &cwd, Some(Path::new("/tmp/x/out.pdf"))).unwrap(),
            PathBuf::from("/tmp/x/out.pdf")
        );
        assert_eq!(
            resolve_pdf_dest(&root_canon, &cwd, Some(Path::new("sub/../client.pdf"))).unwrap(),
            cwd.join("client.pdf")
        );
        assert_eq!(
            resolve_pdf_dest(&root_canon, &cwd, Some(Path::new("../client.pdf"))).unwrap(),
            dir.path().join("client.pdf")
        );
    }

    #[test]
    fn manifest_title_prefers_display_name() {
        let manifest = TemplateManifest {
            id: "nda".to_string(),
            display_name: "Mutual NDA".to_string(),
            ..manifest_stub()
        };
        assert_eq!(manifest_title(&manifest), "Mutual NDA");
        let manifest = TemplateManifest {
            display_name: String::new(),
            ..manifest_stub()
        };
        assert_eq!(manifest_title(&manifest), "nda");
    }

    fn manifest_stub() -> TemplateManifest {
        TemplateManifest {
            id: "nda".to_string(),
            version: "0.1.0".to_string(),
            display_name: String::new(),
            description: "stub".to_string(),
            files: Default::default(),
            placeholders: Vec::new(),
            post_generate: Vec::new(),
        }
    }

    const TEMPLATE_TOML: &str = r#"
id = "fixture-nda"
version = "1.0.0"
display_name = "Fixture NDA"
description = "Test template with one required and one defaulted placeholder"

[[placeholders]]
name = "client"
type = "string"
description = "Client name"
required = true

[[placeholders]]
name = "version"
type = "string"
description = "Document version"
default = "1.0"
"#;

    const MAIN_TEX: &str = r#"\documentclass{article}
\begin{document}
Client: {{client}} v{{version}}.
\end{document}
"#;

    const PROJECT_TOML: &str = r#"[document]
title = "Both Doc"
author = "A"
template = "fixture-nda"

[build]
entry = "main.tex"
"#;

    /// A folder holding BOTH `project.toml` and `template.toml`, as the
    /// owner's legal documents do. Returns the temp dir and the folder.
    fn both_fixture() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("mutual-nda");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("project.toml"), PROJECT_TOML).unwrap();
        std::fs::write(folder.join("template.toml"), TEMPLATE_TOML).unwrap();
        std::fs::write(folder.join("main.tex"), MAIN_TEX).unwrap();
        std::fs::write(folder.join("values.toml"), "client = \"Acme\"\n").unwrap();
        (dir, folder)
    }

    /// A folder holding only `template.toml` and `main.tex`.
    fn template_only_fixture() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("services-agreement");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("template.toml"), TEMPLATE_TOML).unwrap();
        std::fs::write(folder.join("main.tex"), MAIN_TEX).unwrap();
        std::fs::write(folder.join("values.toml"), "client = \"Acme\"\n").unwrap();
        (dir, folder)
    }

    /// Run `body` with the current directory swapped to `dir`. Lock order
    /// stays CWD-only: these tests never touch the process environment.
    fn run_in(dir: &Path, body: impl FnOnce() -> Result<()>) -> Result<()> {
        let _cwd = crate::test_sync::CWD_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let orig = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir).unwrap();
        let result = body();
        std::env::set_current_dir(orig).unwrap();
        result
    }

    /// Run `execute` with a single `--values` file, mirroring the one-file CLI.
    fn execute_one(values_file: &str, out: Option<&Path>) -> Result<()> {
        execute(
            None,
            &[PathBuf::from(values_file)],
            out,
            false,
            None,
            None,
            None,
        )
    }

    fn execute_in(dir: Option<&Path>, values_file: &str, out: Option<&Path>) -> Result<()> {
        execute(
            dir,
            &[PathBuf::from(values_file)],
            out,
            false,
            None,
            None,
            None,
        )
    }

    fn tectonic_available() -> bool {
        crate::compiler::locate_tectonic().is_some()
    }

    fn tectonic_guard() -> Option<std::fs::File> {
        if !tectonic_available() {
            eprintln!("skipping: tectonic not available in environment");
            return None;
        }
        Some(crate::test_support::tectonic_lock())
    }

    /// Every regular file under `dir`, as `(relative path, bytes)`.
    fn snapshot(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
        let mut out = Vec::new();
        snapshot_dir(dir, dir, &mut out);
        out.sort();
        out
    }

    fn snapshot_dir(root: &Path, dir: &Path, out: &mut Vec<(PathBuf, Vec<u8>)>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                snapshot_dir(root, &path, out);
            } else {
                out.push((
                    path.strip_prefix(root).unwrap().to_path_buf(),
                    std::fs::read(&path).unwrap(),
                ));
            }
        }
    }

    #[test]
    fn both_files_build_with_values_to_cwd_pdf() {
        let Some(_tectonic) = tectonic_guard() else {
            return;
        };
        let (_dir, folder) = both_fixture();
        let expected = folder.join("mutual-nda.pdf");
        run_in(&folder, || execute_one("values.toml", None)).unwrap();
        assert!(expected.exists(), "PDF must land in the current directory");
        assert!(
            !folder.parent().unwrap().join("mutual-nda.pdf").exists(),
            "nothing may be written to the parent directory"
        );
        let text = crate::pdftext::extract_text(&expected).unwrap();
        assert!(text.contains("Acme"), "{text}");
        assert!(
            text.contains("1.0"),
            "defaulted placeholder is filled: {text}"
        );
        assert!(!text.contains("{{"), "no raw placeholder survives: {text}");
    }

    #[test]
    fn both_files_build_without_values_as_plain_project() {
        let Some(_tectonic) = tectonic_guard() else {
            return;
        };
        let (_dir, folder) = both_fixture();
        run_in(&folder, || {
            super::super::build::execute(false, None, None, None)
        })
        .unwrap();
        let pdf = folder.join("both-doc.pdf");
        assert!(pdf.exists(), "plain build names the PDF after the title");
        let text = crate::pdftext::extract_text(&pdf).unwrap();
        // TeX renders `{{client}}` as grouping around the bare name, so the
        // raw placeholder reaches the PDF as `client` — while a substituted
        // build would print `Acme` instead.
        assert!(text.contains("client"), "raw placeholder survives: {text}");
        assert!(
            !text.contains("Acme"),
            "no substitution happens without --values: {text}"
        );
    }

    #[test]
    fn template_only_builds_with_values() {
        let Some(_tectonic) = tectonic_guard() else {
            return;
        };
        let (_dir, folder) = template_only_fixture();
        let expected = folder.join("services-agreement.pdf");
        run_in(&folder, || execute_one("values.toml", None)).unwrap();
        assert!(expected.exists());
        let text = crate::pdftext::extract_text(&expected).unwrap();
        assert!(text.contains("Acme"), "{text}");
    }

    #[test]
    fn template_only_fails_without_values_naming_the_flag() {
        let (_dir, folder) = template_only_fixture();
        let err = run_in(&folder, || {
            super::super::build::execute(false, None, None, None)
        })
        .unwrap_err()
        .to_string();
        assert!(err.contains("--values"), "{err}");
    }

    #[test]
    fn missing_required_names_keys_and_values_file() {
        let (_dir, folder) = both_fixture();
        std::fs::write(folder.join("values.toml"), "version = \"2.0\"\n").unwrap();
        let err = run_in(&folder, || execute_one("values.toml", None))
            .unwrap_err()
            .to_string();
        assert!(err.contains("client"), "{err}");
        assert!(err.contains("values.toml"), "{err}");
    }

    #[test]
    fn unknown_keys_fail_as_new_does() {
        let (_dir, folder) = both_fixture();
        std::fs::write(folder.join("values.toml"), "client = \"Acme\"\nbogus = 1\n").unwrap();
        let err = run_in(&folder, || execute_one("values.toml", None))
            .unwrap_err()
            .to_string();
        assert!(err.contains("bogus"), "{err}");
    }

    #[test]
    fn two_values_files_merge_with_the_later_file_winning_in_staged_copy() {
        let (_dir, folder) = template_only_fixture();
        std::fs::write(
            folder.join("lab.toml"),
            "client = \"Lab\"\nversion = \"9\"\n",
        )
        .unwrap();
        std::fs::write(folder.join("client.toml"), "client = \"Acme\"\n").unwrap();
        let root_canon = folder.canonicalize().unwrap();
        let lab = root_canon.join("lab.toml");
        let client = root_canon.join("client.toml");
        let (file_values, _) =
            crate::values_file::load_many(&[lab.clone(), client.clone()]).unwrap();
        let cli_args: HashMap<String, String> = file_values
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        let files = HashMap::from([(
            "template.toml".to_string(),
            std::fs::read(folder.join("template.toml")).unwrap(),
        )]);
        let resolved = new::resolve_placeholder_values(&files, cli_args, &folder);
        let stage = stage_rendered_copy(&root_canon, &[lab, client], &resolved).unwrap();
        let staged = stage.path();
        assert!(
            std::fs::read_to_string(staged.join("main.tex"))
                .unwrap()
                .contains("Client: Acme v9."),
            "the second file wins on overlap and the first file's own key survives"
        );
        assert!(
            !staged.join("lab.toml").exists(),
            "every values file stays out of the staging copy"
        );
        assert!(
            !staged.join("client.toml").exists(),
            "every values file stays out of the staging copy"
        );
    }

    #[test]
    fn unknown_key_in_the_second_file_names_that_file() {
        let (_dir, folder) = both_fixture();
        std::fs::write(folder.join("lab.toml"), "client = \"Acme\"\n").unwrap();
        std::fs::write(folder.join("client.toml"), "client = \"Acme\"\nbogus = 1\n").unwrap();
        let lab = PathBuf::from("lab.toml");
        let client = PathBuf::from("client.toml");
        let err = run_in(&folder, || {
            execute(None, &[lab, client], None, false, None, None, None)
        })
        .unwrap_err()
        .to_string();
        assert!(err.contains("bogus"), "{err}");
        assert!(
            err.contains("client.toml"),
            "the error must name the file that set the key: {err}"
        );
    }

    #[test]
    fn missing_required_with_two_files_names_both_sources() {
        let (_dir, folder) = both_fixture();
        std::fs::write(folder.join("lab.toml"), "version = \"2.0\"\n").unwrap();
        std::fs::write(folder.join("client.toml"), "version = \"3.0\"\n").unwrap();
        let lab = PathBuf::from("lab.toml");
        let client = PathBuf::from("client.toml");
        let err = run_in(&folder, || {
            execute(None, &[lab, client], None, false, None, None, None)
        })
        .unwrap_err()
        .to_string();
        assert!(err.contains("client"), "{err}");
        assert!(err.contains("lab.toml"), "{err}");
        assert!(err.contains("client.toml"), "{err}");
        assert!(err.contains("not set by any file"), "{err}");
    }

    #[test]
    fn values_on_folder_without_template_fails() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("project.toml"), PROJECT_TOML).unwrap();
        std::fs::write(dir.path().join("values.toml"), "client = \"Acme\"\n").unwrap();
        let err = run_in(dir.path(), || execute_one("values.toml", None))
            .unwrap_err()
            .to_string();
        assert!(err.contains("template.toml"), "{err}");
    }

    #[test]
    fn values_on_folder_with_neither_file_fails() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("values.toml"), "client = \"Acme\"\n").unwrap();
        let err = run_in(dir.path(), || execute_one("values.toml", None))
            .unwrap_err()
            .to_string();
        assert!(err.contains("template.toml"), "{err}");
    }

    #[test]
    fn out_writes_to_the_given_path() {
        let Some(_tectonic) = tectonic_guard() else {
            return;
        };
        let (_dir, folder) = template_only_fixture();
        let dest = folder.parent().unwrap().join("custom-name.pdf");
        run_in(&folder, || execute_one("values.toml", Some(dest.as_path()))).unwrap();
        assert!(dest.exists());
        let text = crate::pdftext::extract_text(&dest).unwrap();
        assert!(text.contains("Acme"), "{text}");
    }

    #[test]
    fn successful_build_leaves_the_source_folder_untouched() {
        let Some(_tectonic) = tectonic_guard() else {
            return;
        };
        let (_dir, folder) = both_fixture();
        let before = snapshot(&folder);
        let out = folder.parent().unwrap().join("elsewhere.pdf");
        run_in(&folder, || execute_one("values.toml", Some(out.as_path()))).unwrap();
        assert_eq!(
            snapshot(&folder),
            before,
            "the source folder must be byte-identical"
        );
    }

    #[test]
    fn dir_builds_from_parent_to_cwd_and_leaves_dir_untouched() {
        let Some(_tectonic) = tectonic_guard() else {
            return;
        };
        let parent = tempfile::tempdir().unwrap();
        let tpl = parent.path().join("sub/template");
        std::fs::create_dir_all(&tpl).unwrap();
        std::fs::write(tpl.join("project.toml"), PROJECT_TOML).unwrap();
        std::fs::write(tpl.join("template.toml"), TEMPLATE_TOML).unwrap();
        std::fs::write(tpl.join("main.tex"), MAIN_TEX).unwrap();
        std::fs::write(parent.path().join("v.toml"), "client = \"Acme\"\n").unwrap();

        let before = snapshot(&tpl);
        run_in(parent.path(), || {
            execute_in(Some(Path::new("sub/template")), "v.toml", None)
        })
        .unwrap();
        let default_pdf = parent.path().join("template.pdf");
        assert!(default_pdf.exists(), "PDF must land in cwd, not DIR");
        assert!(!tpl.join("template.pdf").exists());
        assert_eq!(snapshot(&tpl), before, "DIR must remain untouched");
        assert!(crate::pdftext::extract_text(&default_pdf)
            .unwrap()
            .contains("Acme"));

        run_in(parent.path(), || {
            execute_in(
                Some(Path::new("sub/template")),
                "v.toml",
                Some(Path::new("custom.pdf")),
            )
        })
        .unwrap();
        assert!(parent.path().join("custom.pdf").exists());
        assert_eq!(snapshot(&tpl), before);
    }

    #[test]
    fn dir_dot_behaves_like_default() {
        let Some(_tectonic) = tectonic_guard() else {
            return;
        };
        let (_dir, folder) = template_only_fixture();
        run_in(&folder, || {
            execute_in(Some(Path::new(".")), "values.toml", None)
        })
        .unwrap();
        assert!(folder.join("services-agreement.pdf").exists());
    }

    #[test]
    fn missing_or_non_dir_fails_naming_it() {
        let parent = tempfile::tempdir().unwrap();
        std::fs::write(parent.path().join("v.toml"), "client = \"Acme\"\n").unwrap();
        std::fs::write(parent.path().join("not-a-dir.txt"), "hi").unwrap();

        let missing_err = run_in(parent.path(), || {
            execute_in(Some(Path::new("sub/missing")), "v.toml", None)
        })
        .unwrap_err()
        .to_string();
        assert!(missing_err.contains("sub/missing"), "{missing_err}");

        let file_err = run_in(parent.path(), || {
            execute_in(Some(Path::new("not-a-dir.txt")), "v.toml", None)
        })
        .unwrap_err()
        .to_string();
        assert!(file_err.contains("not-a-dir.txt"), "{file_err}");
    }
}
