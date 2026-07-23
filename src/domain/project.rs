//! Project configuration and metadata.

use std::path::PathBuf;

use anyhow::Result;
use serde::{Deserialize, Serialize};

/// Project configuration from project.toml.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectConfig {
    pub document: DocumentConfig,
    pub build: BuildConfig,
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
            },
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
            },
        };
        let cloned = config.clone();
        let debug_str = format!("{:?}", config);
        assert!(debug_str.contains("T"));
        assert_eq!(cloned.document.title, "T");
    }

    #[test]
    fn project_load_no_project_toml_errors() {
        let tmp = tempfile::tempdir().unwrap();
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
        let orig = std::env::current_dir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();
        let result = Project::load();
        std::env::set_current_dir(&orig).unwrap();
        assert!(result.is_err());
    }
}
