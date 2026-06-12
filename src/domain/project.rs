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
