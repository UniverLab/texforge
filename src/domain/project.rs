//! Project configuration and metadata.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};

/// The pre-0.9 spelling of every `project.toml` key that still has a current
/// counterpart, as `(old, new)`. Tables carry their brackets; `template` and
/// `entry` never changed and are absent. Old keys are rejected, never accepted
/// as aliases: the error names every rename so the user edits the file by
/// hand. Add a rename by adding one line here.
const LEGACY_RENAMES: &[(&str, &str)] = &[
    ("[documento]", "[document]"),
    ("titulo", "title"),
    ("autor", "author"),
    ("[compilacion]", "[build]"),
    ("bibliografia", "bibliography"),
];

/// Project configuration from project.toml.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectConfig {
    pub document: DocumentConfig,
    pub build: BuildConfig,
    #[serde(default)]
    pub diagrams: Option<DiagramsConfig>,
    #[serde(default)]
    pub highlight: Option<HighlightConfig>,
}

/// `[highlight]` section of `project.toml`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HighlightConfig {
    /// Syntax-highlighting palette family (`github`, `one-light`).
    #[serde(default)]
    pub theme: Option<String>,
    /// Document-wide listing style (`light`, `light-mono`, `dark`,
    /// `dark-mono`, `ink`). A block's `style=` or a `[highlight.by_lang]`
    /// entry wins.
    #[serde(default)]
    pub style: Option<String>,
    /// Per-language styles, keyed by language name or alias
    /// (`[highlight.by_lang] bash = "dark"`).
    #[serde(default)]
    pub by_lang: HashMap<String, String>,
    /// Rewrite `\begin{lstlisting}` blocks too (off by default: without the
    /// opt-in, `listings` users keep real `listings.sty` behaviour).
    #[serde(default)]
    pub lstlisting: Option<bool>,
    /// Number every line of every block unless the block says otherwise.
    #[serde(default)]
    pub numbers: Option<bool>,
    /// Override the language's listing name (`Listing` / `Listado`).
    #[serde(default)]
    pub caption_name: Option<String>,
    /// Override the language's list-of-listings heading.
    #[serde(default)]
    pub list_name: Option<String>,
    /// Typewriter family for code blocks, from the monospace families the
    /// tectonic bundle ships. `document` (the default) keeps the preamble's
    /// own `\ttfamily` (Latin Modern Mono in the bundled templates).
    /// Unrecognised values fail the build by name.
    #[serde(default)]
    pub font: Option<String>,
}

/// `[diagrams]` section of `project.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagramsConfig {
    /// Document-wide default style preset (`default`, `editorial`,
    /// `monochrome`, `technical`). A `style=` on the environment itself
    /// overrides this.
    #[serde(default)]
    pub style: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentConfig {
    pub title: String,
    pub author: String,
    pub template: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildConfig {
    pub entry: String,
    #[serde(default)]
    pub bibliography: Option<String>,
    #[serde(default)]
    pub reproducible: Option<Reproducible>,
}

/// Reproducible-build setting from `project.toml` (`[build] reproducible`).
///
/// Accepts `true` to pin a fixed default epoch, `false` to disable, or a
/// number to pin an explicit `SOURCE_DATE_EPOCH` value. Absent means the build
/// is not reproducible.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Reproducible {
    Enabled(bool),
    Epoch(u64),
}

/// Represents a `TexForge` project.
#[derive(Debug)]
pub struct Project {
    pub root: PathBuf,
    pub config: ProjectConfig,
}

impl Project {
    /// Build a project from an explicit root and config. [`Project::load`]
    /// delegates to this; template-folder builds stage a rendered copy and
    /// run the pipeline on it through this constructor.
    pub fn from_parts(root: PathBuf, config: ProjectConfig) -> Self {
        Self { root, config }
    }

    /// Load project from current directory.
    pub fn load() -> Result<Self> {
        Self::load_from(&std::env::current_dir()?)
    }

    /// Load project from an explicit root directory.
    pub fn load_from(root: &Path) -> Result<Self> {
        let config_path = root.join("project.toml");

        if !config_path.exists() {
            anyhow::bail!("No project.toml found in {}", root.display());
        }

        let content = std::fs::read_to_string(&config_path)?;
        let table: toml::Table =
            toml::from_str(&content).map_err(|e| toml_error(&config_path, &content, &e))?;
        reject_legacy_keys(&config_path, &table)?;
        let config: ProjectConfig =
            toml::from_str(&content).map_err(|e| toml_error(&config_path, &content, &e))?;

        Ok(Self::from_parts(root.to_path_buf(), config))
    }
}

/// Turn a TOML error into one that names the manifest and, when the error
/// carries a span, its line and column.
fn toml_error(config_path: &Path, content: &str, error: &toml::de::Error) -> anyhow::Error {
    match error.span() {
        Some(span) => {
            let (line, column) = line_and_column(content, span.start);
            anyhow::anyhow!(
                "{}: TOML error at line {line}, column {column}: {}",
                config_path.display(),
                error.message()
            )
        }
        None => anyhow::anyhow!("{}: {}", config_path.display(), error),
    }
}

fn line_and_column(content: &str, index: usize) -> (usize, usize) {
    let before = &content[..index.min(content.len())];
    let line = before.matches('\n').count() + 1;
    let column = before
        .rsplit('\n')
        .next()
        .map_or(1, |l| l.chars().count() + 1);
    (line, column)
}

/// Fail when the manifest still uses a pre-0.9 key, listing every rename and
/// the hand-edit fix. The manifest is never rewritten.
fn reject_legacy_keys(config_path: &Path, table: &toml::Table) -> Result<()> {
    let offenders = legacy_offenders(table);
    if offenders.is_empty() {
        return Ok(());
    }

    let renames: Vec<String> = offenders
        .iter()
        .map(|(old, new)| format!("  {old} -> {new}"))
        .collect();
    let fixes: Vec<String> = offenders.iter().map(|(_, new)| fix_line(new)).collect();
    anyhow::bail!(
        "{} uses keys renamed in texforge 0.9:\n{}\n\n\
         Edit the file by hand (it is never modified for you):\n{}",
        config_path.display(),
        renames.join("\n"),
        fixes.join("\n")
    )
}

/// The `(old, new)` renames present in the manifest. Table renames are
/// top-level keys; key renames are looked for at the top level and inside
/// every top-level table, so they are caught wherever they were left.
fn legacy_offenders(table: &toml::Table) -> Vec<(&'static str, &'static str)> {
    LEGACY_RENAMES
        .iter()
        .copied()
        .filter(|(old, _)| match table_name(old) {
            Some(name) => table.contains_key(name),
            None => {
                table.contains_key(*old)
                    || table
                        .values()
                        .filter_map(toml::Value::as_table)
                        .any(|t| t.contains_key(*old))
            }
        })
        .collect()
}

/// The key a bracketed rename names (`"[documento]"` -> `"documento"`).
fn table_name(key: &str) -> Option<&str> {
    key.strip_prefix('[').and_then(|k| k.strip_suffix(']'))
}

/// The plain line the user writes instead, e.g. `[document]` or
/// `title = "..."`.
fn fix_line(new: &str) -> String {
    if table_name(new).is_some() {
        format!("  {new}")
    } else {
        format!("  {new} = \"...\"")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_config_deserialize_full() {
        let toml_str = r#"
[document]
title = "My Thesis"
author = "Jane"
template = "general"

[build]
entry = "main.tex"
bibliography = "refs.bib"
"#;
        let config: ProjectConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.document.title, "My Thesis");
        assert_eq!(config.document.author, "Jane");
        assert_eq!(config.document.template, "general");
        assert_eq!(config.build.entry, "main.tex");
        assert_eq!(config.build.bibliography, Some("refs.bib".to_string()));
    }

    #[test]
    fn project_config_deserialize_no_bibliography() {
        let toml_str = r#"
[document]
title = "Doc"
author = "A"
template = "general"

[build]
entry = "main.tex"
"#;
        let config: ProjectConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.build.bibliography, None);
    }

    #[test]
    fn project_config_serialize_roundtrip() {
        let config = ProjectConfig {
            document: DocumentConfig {
                title: "Test".to_string(),
                author: "Author".to_string(),
                template: "general".to_string(),
            },
            build: BuildConfig {
                entry: "main.tex".to_string(),
                bibliography: Some("refs.bib".to_string()),
                reproducible: None,
            },
            diagrams: None,
            highlight: None,
        };
        let toml_str = toml::to_string_pretty(&config).unwrap();
        let parsed: ProjectConfig = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.document.title, "Test");
        assert_eq!(parsed.build.entry, "main.tex");
    }

    #[test]
    fn project_config_debug_clone() {
        let config = ProjectConfig {
            document: DocumentConfig {
                title: "T".to_string(),
                author: "A".to_string(),
                template: "general".to_string(),
            },
            build: BuildConfig {
                entry: "main.tex".to_string(),
                bibliography: None,
                reproducible: None,
            },
            diagrams: None,
            highlight: None,
        };
        let cloned = config.clone();
        let debug_str = format!("{:?}", config);
        assert!(debug_str.contains("T"));
        assert_eq!(cloned.document.title, "T");
    }

    #[test]
    fn project_load_no_project_toml_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let _cwd = crate::test_sync::CWD_LOCK.lock().unwrap();
        let orig = std::env::current_dir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();
        let result = Project::load();
        std::env::set_current_dir(&orig).unwrap();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("No project.toml"));
    }

    /// Load the project from `root` while holding the process-wide cwd lock.
    fn load_in(root: &std::path::Path) -> Result<Project> {
        let _cwd = crate::test_sync::CWD_LOCK.lock().unwrap();
        let orig = std::env::current_dir().unwrap();
        std::env::set_current_dir(root).unwrap();
        let result = Project::load();
        std::env::set_current_dir(&orig).unwrap();
        result
    }

    const PRE_09_MANIFEST: &str = "[documento]\n\
         titulo = \"Tesis\"\n\
         autor = \"Ada\"\n\
         template = \"general\"\n\
         \n\
         [compilacion]\n\
         entry = \"main.tex\"\n\
         bibliografia = \"refs.bib\"\n";

    /// FR5(a): both pre-0.9 tables yield every rename, the file path, the
    /// copy-paste fix, and leave the file byte-for-byte untouched.
    #[test]
    fn project_load_pre_09_manifest_lists_every_rename() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("project.toml");
        std::fs::write(&path, PRE_09_MANIFEST).unwrap();

        let err = load_in(tmp.path()).expect_err("pre-0.9 keys must be rejected");
        let msg = err.to_string();
        for rename in [
            "[documento] -> [document]",
            "titulo -> title",
            "autor -> author",
            "[compilacion] -> [build]",
            "bibliografia -> bibliography",
        ] {
            assert!(msg.contains(rename), "{rename} missing from:\n{msg}");
        }
        assert!(
            msg.contains(&path.display().to_string()),
            "the error must name the manifest:\n{msg}"
        );
        assert!(
            msg.contains("[document]") && msg.contains("title = \"...\""),
            "the error must show the manual fix:\n{msg}"
        );
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            PRE_09_MANIFEST,
            "the manifest must never be rewritten"
        );
    }

    /// FR5(b): a manifest mixing old and current keys lists only the old ones.
    #[test]
    fn project_load_mixed_manifest_lists_only_old_keys() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("project.toml"),
            "[document]\n\
             title = \"T\"\n\
             autor = \"Ada\"\n\
             template = \"general\"\n\
             \n\
             [build]\n\
             entry = \"main.tex\"\n\
             bibliografia = \"refs.bib\"\n",
        )
        .unwrap();

        let msg = load_in(tmp.path())
            .expect_err("old keys are rejected even beside current ones")
            .to_string();
        assert!(msg.contains("autor -> author"), "{msg}");
        assert!(msg.contains("bibliografia -> bibliography"), "{msg}");
        assert!(
            !msg.contains("documento") && !msg.contains("titulo") && !msg.contains("compilacion"),
            "only the offending keys are listed:\n{msg}"
        );
    }

    /// FR5(c): a current manifest is unaffected by the rename check.
    #[test]
    fn project_load_current_manifest_is_unaffected() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("project.toml"),
            "[document]\n\
             title = \"T\"\n\
             author = \"Ada\"\n\
             template = \"general\"\n\
             \n\
             [build]\n\
             entry = \"main.tex\"\n\
             bibliography = \"refs.bib\"\n",
        )
        .unwrap();

        let project = load_in(tmp.path()).expect("a current manifest loads");
        assert_eq!(project.config.document.title, "T");
        assert_eq!(project.config.document.author, "Ada");
        assert_eq!(project.config.build.entry, "main.tex");
        assert_eq!(
            project.config.build.bibliography.as_deref(),
            Some("refs.bib")
        );
    }

    /// FR4: an unknown key that is not in the rename table keeps failing, and
    /// the message names the file rather than the bare TOML text.
    #[test]
    fn project_load_unknown_key_error_names_the_file() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("project.toml");
        std::fs::write(
            &path,
            "[document]\n\
             title = \"T\"\n\
             author = \"A\"\n\
             template = \"general\"\n\
             \n\
             [compilation]\n\
             entry = \"main.tex\"\n",
        )
        .unwrap();

        let msg = load_in(tmp.path())
            .expect_err("an unknown table is not silently accepted")
            .to_string();
        assert!(
            msg.contains(&path.display().to_string()),
            "the error must name the manifest:\n{msg}"
        );
    }

    /// FR4: a syntax error reports the manifest's own line and column, not the
    /// bare `line 1, column 1` wording.
    #[test]
    fn project_load_syntax_error_names_the_file_and_position() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("project.toml");
        std::fs::write(
            &path,
            "[document]\ntitle = \"T\"\nauthor = \"A\"\n_BROKEN_ = = 1\n",
        )
        .unwrap();

        let msg = load_in(tmp.path())
            .expect_err("invalid TOML must fail")
            .to_string();
        assert!(
            msg.contains(&path.display().to_string()),
            "the error must name the manifest:\n{msg}"
        );
        assert!(
            msg.contains("line 4"),
            "the line of the syntax error must be reported:\n{msg}"
        );
    }

    #[test]
    fn project_load_valid_toml() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("project.toml"),
            "[document]\ntitle = \"T\"\nauthor = \"A\"\ntemplate = \"general\"\n\n[build]\nentry = \"main.tex\"\n",
        )
        .unwrap();
        let _cwd = crate::test_sync::CWD_LOCK.lock().unwrap();
        let orig = std::env::current_dir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();
        let result = Project::load();
        std::env::set_current_dir(&orig).unwrap();
        let project = result.unwrap();
        assert_eq!(project.config.document.title, "T");
        assert_eq!(project.config.build.entry, "main.tex");
    }

    #[test]
    fn project_load_invalid_toml_errors() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("project.toml"), "not valid {{{ toml").unwrap();
        let _cwd = crate::test_sync::CWD_LOCK.lock().unwrap();
        let orig = std::env::current_dir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();
        let result = Project::load();
        std::env::set_current_dir(&orig).unwrap();
        assert!(result.is_err());
    }

    #[test]
    fn project_config_reproducible_true() {
        let toml_str = r#"
[document]
title = "T"
author = "A"
template = "general"

[build]
entry = "main.tex"
reproducible = true
"#;
        let config: ProjectConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.build.reproducible, Some(Reproducible::Enabled(true)));
    }

    #[test]
    fn project_config_reproducible_false() {
        let toml_str = r#"
[document]
title = "T"
author = "A"
template = "general"

[build]
entry = "main.tex"
reproducible = false
"#;
        let config: ProjectConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(
            config.build.reproducible,
            Some(Reproducible::Enabled(false))
        );
    }

    #[test]
    fn project_config_reproducible_explicit_epoch() {
        let toml_str = r#"
[document]
title = "T"
author = "A"
template = "general"

[build]
entry = "main.tex"
reproducible = 1700000000
"#;
        let config: ProjectConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(
            config.build.reproducible,
            Some(Reproducible::Epoch(1700000000))
        );
    }

    #[test]
    fn project_config_reproducible_absent_is_off() {
        let toml_str = r#"
[document]
title = "T"
author = "A"
template = "general"

[build]
entry = "main.tex"
"#;
        let config: ProjectConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.build.reproducible, None);
    }

    #[test]
    fn project_config_reproducible_roundtrip() {
        let config = ProjectConfig {
            document: DocumentConfig {
                title: "T".to_string(),
                author: "A".to_string(),
                template: "general".to_string(),
            },
            build: BuildConfig {
                entry: "main.tex".to_string(),
                bibliography: None,
                reproducible: Some(Reproducible::Epoch(1700000000)),
            },
            diagrams: None,
            highlight: None,
        };
        let serialized = toml::to_string_pretty(&config).unwrap();
        let parsed: ProjectConfig = toml::from_str(&serialized).unwrap();
        assert_eq!(
            parsed.build.reproducible,
            Some(Reproducible::Epoch(1700000000))
        );
    }

    #[test]
    fn project_config_diagrams_style_present() {
        let toml_str = r#"
[document]
title = "T"
author = "A"
template = "general"

[build]
entry = "main.tex"

[diagrams]
style = "editorial"
"#;
        let config: ProjectConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(
            config.diagrams.and_then(|d| d.style),
            Some("editorial".to_string())
        );
    }

    #[test]
    fn project_config_diagrams_absent_is_none() {
        let toml_str = r#"
[document]
title = "T"
author = "A"
template = "general"

[build]
entry = "main.tex"
"#;
        let config: ProjectConfig = toml::from_str(toml_str).unwrap();
        assert!(config.diagrams.is_none());
    }

    #[test]
    fn project_config_highlight_full_section() {
        let toml_str = r#"
[document]
title = "T"
author = "A"
template = "general"

[build]
entry = "main.tex"

[highlight]
theme = "one-light"
style = "dark-mono"
lstlisting = true
numbers = true

[highlight.by_lang]
bash = "dark"
"#;
        let config: ProjectConfig = toml::from_str(toml_str).unwrap();
        let highlight = config.highlight.expect("highlight section");
        assert_eq!(highlight.theme.as_deref(), Some("one-light"));
        assert_eq!(highlight.style.as_deref(), Some("dark-mono"));
        assert_eq!(highlight.lstlisting, Some(true));
        assert_eq!(highlight.numbers, Some(true));
        assert_eq!(
            highlight.by_lang.get("bash").map(String::as_str),
            Some("dark"),
            "the per-language table parses as written"
        );
    }

    /// Neither style key is required: a `[highlight]` section without them
    /// still resolves, and an absent table is empty rather than missing.
    #[test]
    fn project_config_highlight_styles_default_to_absent() {
        let toml_str = r#"
[document]
title = "T"
author = "A"
template = "general"

[build]
entry = "main.tex"

[highlight]
theme = "github"
"#;
        let config: ProjectConfig = toml::from_str(toml_str).unwrap();
        let highlight = config.highlight.expect("highlight section");
        assert_eq!(highlight.style, None);
        assert_eq!(highlight.font, None);
        assert!(
            highlight.by_lang.is_empty(),
            "an absent table is empty, not an error"
        );
    }

    #[test]
    fn project_config_highlight_absent_is_none() {
        let toml_str = r#"
[document]
title = "T"
author = "A"
template = "general"

[build]
entry = "main.tex"
"#;
        let config: ProjectConfig = toml::from_str(toml_str).unwrap();
        assert!(config.highlight.is_none());
    }

    #[test]
    fn project_config_highlight_partial_keeps_nones() {
        let toml_str = r#"
[document]
title = "T"
author = "A"
template = "general"

[build]
entry = "main.tex"

[highlight]
theme = "github"
"#;
        let config: ProjectConfig = toml::from_str(toml_str).unwrap();
        let highlight = config.highlight.expect("highlight section");
        assert_eq!(highlight.theme.as_deref(), Some("github"));
        assert_eq!(highlight.lstlisting, None);
        assert_eq!(highlight.numbers, None);
    }

    #[test]
    fn project_config_highlight_font_parses() {
        let toml_str = r#"
[document]
title = "T"
author = "A"
template = "general"

[build]
entry = "main.tex"

[highlight]
font = "inconsolata"
"#;
        let config: ProjectConfig = toml::from_str(toml_str).unwrap();
        let highlight = config.highlight.expect("highlight section");
        assert_eq!(highlight.font.as_deref(), Some("inconsolata"));
    }
}
