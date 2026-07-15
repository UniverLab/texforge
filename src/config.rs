//! Global user configuration system.
//!
//! Stores user preferences in `~/.texforge/config.toml` or `$XDG_CONFIG_HOME/texforge/config.toml`.
//! Supports TOML format with user, institution, defaults, and templates sections.

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

/// Global configuration structure
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub user: UserConfig,
    #[serde(default)]
    pub institution: InstitutionConfig,
    #[serde(default)]
    pub defaults: DefaultsConfig,
    #[serde(default)]
    pub templates: TemplatesConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UserConfig {
    pub name: Option<String>,
    pub email: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct InstitutionConfig {
    pub name: Option<String>,
    pub address: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DefaultsConfig {
    pub documentclass: Option<String>,
    pub fontsize: Option<String>,
    pub papersize: Option<String>,
    pub language: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TemplatesConfig {
    pub source: Option<String>,
    pub auto_update: Option<bool>,
    pub watch: Option<bool>,
}

/// Returns the path to the user's texforge config directory.
/// Prefers `$XDG_CONFIG_HOME/texforge`, falls back to `~/.texforge/config.toml`.
#[allow(dead_code)]
fn config_dir() -> Result<PathBuf> {
    if let Ok(xdg_config) = std::env::var("XDG_CONFIG_HOME") {
        let path = PathBuf::from(xdg_config).join("texforge");
        return Ok(path);
    }

    let home = dirs::home_dir().ok_or_else(|| anyhow!("Could not determine home directory"))?;
    Ok(home.join(".texforge"))
}

/// Returns the path to the config.toml file
pub fn config_file_path() -> Result<PathBuf> {
    if let Ok(xdg_config) = std::env::var("XDG_CONFIG_HOME") {
        let path = PathBuf::from(xdg_config).join("texforge/config.toml");
        return Ok(path);
    }

    let home = dirs::home_dir().ok_or_else(|| anyhow!("Could not determine home directory"))?;
    Ok(home.join(".texforge/config.toml"))
}

/// Load config from `~/.texforge/config.toml` or `XDG_CONFIG_HOME/texforge/config.toml`
pub fn load() -> Result<Config> {
    let path = config_file_path()?;

    if !path.exists() {
        // Return empty config if file doesn't exist
        return Ok(Config::default());
    }

    let content = fs::read_to_string(&path)
        .with_context(|| format!("Failed to read config file: {}", path.display()))?;

    toml::from_str(&content).context("Failed to parse config TOML")
}

/// Save config to `~/.texforge/config.toml` or `XDG_CONFIG_HOME/texforge/config.toml`
pub fn save(config: &Config) -> Result<()> {
    let path = config_file_path()?;
    let dir = path
        .parent()
        .ok_or_else(|| anyhow!("Invalid config path"))?;

    // Create directory if it doesn't exist
    fs::create_dir_all(dir).context("Failed to create config directory")?;

    let content = toml::to_string_pretty(config).context("Failed to serialize config")?;

    fs::write(&path, content)
        .with_context(|| format!("Failed to write config file: {}", path.display()))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_config() {
        let toml_str = r#"
[user]
name = "Jane Doe"
email = "jane@example.com"

[defaults]
documentclass = "article"
fontsize = "11pt"
"#;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.user.name, Some("Jane Doe".to_string()));
        assert_eq!(config.defaults.fontsize, Some("11pt".to_string()));
    }

    #[test]
    fn test_serialize_config() {
        let mut config = Config::default();
        config.user.name = Some("John Doe".to_string());
        config.user.email = Some("john@example.com".to_string());

        let toml_str = toml::to_string_pretty(&config).unwrap();
        assert!(toml_str.contains("John Doe"));
        assert!(toml_str.contains("john@example.com"));
    }

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert!(config.user.name.is_none());
        assert!(config.user.email.is_none());
        assert!(config.institution.name.is_none());
        assert!(config.defaults.documentclass.is_none());
        assert!(config.templates.source.is_none());
    }

    #[test]
    fn test_parse_institution() {
        let toml_str = r#"
[institution]
name = "University"
address = "123 Main St"
"#;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.institution.name, Some("University".to_string()));
        assert_eq!(config.institution.address, Some("123 Main St".to_string()));
    }

    #[test]
    fn test_parse_templates_section() {
        let toml_str = r#"
[templates]
source = "registry"
auto_update = true
watch = false
"#;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.templates.source, Some("registry".to_string()));
        assert_eq!(config.templates.auto_update, Some(true));
        assert_eq!(config.templates.watch, Some(false));
    }

    #[test]
    fn test_parse_defaults_all_fields() {
        let toml_str = r#"
[defaults]
documentclass = "report"
fontsize = "12pt"
papersize = "a4"
language = "spanish"
"#;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.defaults.documentclass, Some("report".to_string()));
        assert_eq!(config.defaults.fontsize, Some("12pt".to_string()));
        assert_eq!(config.defaults.papersize, Some("a4".to_string()));
        assert_eq!(config.defaults.language, Some("spanish".to_string()));
    }

    #[test]
    fn test_parse_empty_toml() {
        let config: Config = toml::from_str("").unwrap();
        let default = Config::default();
        assert_eq!(config.user.name, default.user.name);
        assert_eq!(config.user.email, default.user.email);
        assert_eq!(config.institution.name, default.institution.name);
        assert_eq!(config.defaults.documentclass, default.defaults.documentclass);
    }

    #[test]
    fn test_config_file_path_returns_result() {
        let result = config_file_path();
        assert!(result.is_ok());
        let path = result.unwrap();
        assert!(path.to_string_lossy().contains("texforge"));
        assert!(path.to_string_lossy().ends_with("config.toml"));
    }

    #[test]
    fn test_serialize_roundtrip() {
        let mut config = Config::default();
        config.user.name = Some("Test User".to_string());
        config.user.email = Some("test@test.com".to_string());
        config.institution.name = Some("Test Uni".to_string());
        config.defaults.language = Some("english".to_string());

        let serialized = toml::to_string_pretty(&config).unwrap();
        let deserialized: Config = toml::from_str(&serialized).unwrap();
        assert_eq!(config.user.name, deserialized.user.name);
        assert_eq!(config.user.email, deserialized.user.email);
        assert_eq!(config.institution.name, deserialized.institution.name);
        assert_eq!(config.defaults.language, deserialized.defaults.language);
    }
}
