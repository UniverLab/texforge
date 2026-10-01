//! Project configuration and metadata.

use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::Result;
use serde::{Deserialize, Serialize};

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
    /// `dark-mono`). A block's `style=` or a `[highlight.by_lang]` entry wins.
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
    /// Load project from current directory.
    pub fn load() -> Result<Self> {
        let root = std::env::current_dir()?;
        let config_path = root.join("project.toml");

        if !config_path.exists() {
            anyhow::bail!("No project.toml found in current directory");
        }

        let content = std::fs::read_to_string(&config_path)?;
        let config: ProjectConfig = toml::from_str(&content)?;

        Ok(Self { root, config })
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
}
