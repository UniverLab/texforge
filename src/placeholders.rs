//! Placeholder resolution engine with 5-level precedence chain.
//!
//! Resolves `{{placeholder}}` tokens in template files according to:
//! 1. CLI arguments
//! 2. Project config (`./.texforge/config.toml`)
//! 3. User config (`~/.texforge/config.toml`)
//! 4. Template defaults (from `template.toml`)
//! 5. Interactive prompt (if required and no value found)

#![allow(dead_code)]

use crate::config;
use crate::manifest::Placeholder;
use anyhow::{anyhow, Result};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Placeholder resolver with precedence chain
pub struct PlaceholderResolver {
    /// Values from CLI (highest priority)
    cli_args: HashMap<String, String>,
    /// Values from project config
    project_config: HashMap<String, String>,
    /// Values from user config (loaded from ~/.texforge/config.toml)
    user_config: Option<config::Config>,
    /// Directory the `git config --get …` identity fallback runs in — the
    /// target project directory for `texforge new`.
    target_dir: PathBuf,
}

impl PlaceholderResolver {
    /// Create a new resolver (the git fallback runs in the current directory)
    pub fn new(cli_args: HashMap<String, String>) -> Self {
        let dir = std::env::current_dir().unwrap_or_default();
        Self::new_in(cli_args, &dir)
    }

    /// Create a resolver whose git identity fallback runs inside `dir`
    pub fn new_in(cli_args: HashMap<String, String>, dir: &Path) -> Self {
        let user_config = config::load().ok();
        let project_config = load_project_config().unwrap_or_default();

        Self {
            cli_args,
            project_config,
            user_config,
            target_dir: dir.to_path_buf(),
        }
    }

    /// Resolve a placeholder value using the 5-level precedence chain
    /// Returns None if not found, Err if resolution fails
    pub fn resolve(&self, placeholder: &Placeholder) -> Result<Option<String>> {
        // 1. Check CLI arguments first
        if let Some(value) = self.cli_args.get(&placeholder.name) {
            return Ok(Some(value.clone()));
        }

        // 2. Check project config
        if let Some(value) = self.project_config.get(&placeholder.name) {
            return Ok(Some(value.clone()));
        }

        // 3. Check user config
        if let Some(user_cfg) = &self.user_config {
            if let Some(value) = self.resolve_from_user_config(user_cfg, &placeholder.name) {
                return Ok(Some(value));
            }
        }

        // 4. Check template default
        if let Some(default) = &placeholder.default {
            let resolved = self.resolve_interpolations(default)?;
            return Ok(Some(resolved));
        }

        // If required and not found, error (caller should prompt)
        // If optional, return None
        Ok(None)
    }

    /// Resolve all placeholders in a set, filling required ones or erroring
    pub fn resolve_all(&self, placeholders: &[Placeholder]) -> Result<HashMap<String, String>> {
        let mut result = HashMap::new();

        for ph in placeholders {
            let resolved = self.resolve(ph)?;
            if let Some(value) = resolved {
                result.insert(ph.name.clone(), value);
            } else if ph.required {
                return Err(anyhow!(
                    "Required placeholder '{}' has no value ({})",
                    ph.name,
                    ph.description
                ));
            }
        }

        Ok(result)
    }

    /// Replace {{placeholder}} tokens in content
    pub fn substitute(&self, content: &str, values: &HashMap<String, String>) -> Result<String> {
        let mut result = content.to_string();

        for (key, value) in values {
            let token = format!("{{{{{}}}}}", key);
            result = result.replace(&token, value);
        }

        // Check for unresolved tokens
        if result.contains("{{") {
            return Err(anyhow!("Unresolved placeholders found in content"));
        }

        Ok(result)
    }

    /// Resolve `{{user.name}}`-style interpolations in defaults.
    ///
    /// A value missing from the texforge config falls back to
    /// `git config --get …` in the target directory, and finally to the
    /// empty string, so no raw `{{user.*}}` / `{{institution.*}}` token
    /// survives into a generated file.
    fn resolve_interpolations(&self, text: &str) -> Result<String> {
        let mut result = text.to_string();

        // {{user.name}}
        if result.contains("{{user.name}}") {
            let name = self
                .user_config
                .as_ref()
                .and_then(|cfg| cfg.user.name.clone())
                .filter(|value| !value.is_empty())
                .or_else(|| git_config_value(&self.target_dir, "user.name"))
                .unwrap_or_default();
            result = result.replace("{{user.name}}", &name);
        }

        // {{user.email}}
        if result.contains("{{user.email}}") {
            let email = self
                .user_config
                .as_ref()
                .and_then(|cfg| cfg.user.email.clone())
                .filter(|value| !value.is_empty())
                .or_else(|| git_config_value(&self.target_dir, "user.email"))
                .unwrap_or_default();
            result = result.replace("{{user.email}}", &email);
        }

        // {{institution.name}} — config only, no git fallback
        if result.contains("{{institution.name}}") {
            let name = self
                .user_config
                .as_ref()
                .and_then(|cfg| cfg.institution.name.clone())
                .filter(|value| !value.is_empty())
                .unwrap_or_default();
            result = result.replace("{{institution.name}}", &name);
        }

        Ok(clear_unresolved_identities(&result))
    }

    /// Extract value from user config by placeholder name (convention: section.key)
    fn resolve_from_user_config(&self, cfg: &config::Config, placeholder: &str) -> Option<String> {
        // Try direct match in each section
        if let Some(name) = &cfg.user.name {
            if placeholder == "author" || placeholder == "user.name" {
                return Some(name.clone());
            }
        }
        if let Some(email) = &cfg.user.email {
            if placeholder == "email" || placeholder == "user.email" {
                return Some(email.clone());
            }
        }

        if let Some(name) = &cfg.institution.name {
            if placeholder == "institution" || placeholder == "institution.name" {
                return Some(name.clone());
            }
        }

        if let Some(dc) = &cfg.defaults.documentclass {
            if placeholder == "documentclass" {
                return Some(dc.clone());
            }
        }
        if let Some(lang) = &cfg.defaults.language {
            if placeholder == "language" {
                return Some(lang.clone());
            }
        }

        None
    }
}

/// `git config --get <key>` run in `dir`. Returns `None` when git is not
/// installed, the command fails (including "key not found"), or the value is
/// empty. The environment is passed through unchanged so callers keep
/// honouring `GIT_CONFIG_GLOBAL` / `GIT_CONFIG_SYSTEM`.
fn git_config_value(dir: &Path, key: &str) -> Option<String> {
    let output = Command::new("git")
        .arg("config")
        .arg("--get")
        .arg(key)
        .current_dir(dir)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

/// Replace every remaining `{{user.…}}` / `{{institution.…}}` token with the
/// empty string; any other `{{…}}` token is left untouched. Shared with
/// `crate::commands::new` so generated files never carry raw identity
/// placeholders through two different code paths.
pub(crate) fn clear_unresolved_identities(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("{{") {
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else {
            out.push_str(rest);
            return out;
        };
        out.push_str(&rest[..start]);
        let inner = &after[..end];
        if !inner.starts_with("user.") && !inner.starts_with("institution.") {
            out.push_str(&rest[start..start + 2 + end + 2]);
        }
        rest = &after[end + 2..];
    }
    out.push_str(rest);
    out
}

/// Load project-level config from ./.texforge/config.toml
fn load_project_config() -> Result<HashMap<String, String>> {
    let path = std::path::PathBuf::from(".texforge/config.toml");
    if !path.exists() {
        return Ok(HashMap::new());
    }

    let content = std::fs::read_to_string(&path)?;
    let values: toml::Table = toml::from_str(&content)?;
    let mut result = HashMap::new();

    // Flatten the TOML structure into a simple map
    flatten_toml(&values, "", &mut result);

    Ok(result)
}

/// Flatten nested TOML into key.subkey format
fn flatten_toml(table: &toml::Table, prefix: &str, result: &mut HashMap<String, String>) {
    for (key, value) in table.iter() {
        let full_key = if prefix.is_empty() {
            key.to_string()
        } else {
            format!("{}.{}", prefix, key)
        };

        match value {
            toml::Value::String(s) => {
                result.insert(full_key, s.clone());
            }
            toml::Value::Table(t) => {
                flatten_toml(t, &full_key, result);
            }
            toml::Value::Boolean(b) => {
                result.insert(full_key, b.to_string());
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{Placeholder, PlaceholderType};

    fn make_placeholder(name: &str, required: bool) -> Placeholder {
        Placeholder {
            name: name.to_string(),
            r#type: PlaceholderType::String,
            description: "test".to_string(),
            required,
            default: None,
            choices: None,
        }
    }

    #[test]
    fn test_resolve_cli_priority() {
        let mut cli_args = HashMap::new();
        cli_args.insert("title".to_string(), "My Title".to_string());

        let resolver = PlaceholderResolver {
            cli_args,
            project_config: HashMap::new(),
            user_config: None,
            target_dir: PathBuf::new(),
        };

        let ph = make_placeholder("title", true);
        let result = resolver.resolve(&ph).unwrap();
        assert_eq!(result, Some("My Title".to_string()));
    }

    #[test]
    fn test_resolve_missing_required() {
        let resolver = PlaceholderResolver {
            cli_args: HashMap::new(),
            project_config: HashMap::new(),
            user_config: None,
            target_dir: PathBuf::new(),
        };

        let mut ph = make_placeholder("title", true);
        ph.default = None;

        let result = resolver.resolve_all(&[ph]);
        assert!(result.is_err());
    }

    #[test]
    fn test_substitute_tokens() {
        let resolver = PlaceholderResolver {
            cli_args: HashMap::new(),
            project_config: HashMap::new(),
            user_config: None,
            target_dir: PathBuf::new(),
        };

        let mut values = HashMap::new();
        values.insert("title".to_string(), "My Document".to_string());
        values.insert("author".to_string(), "Jane Doe".to_string());

        let content = "\\title{{{title}}}\n\\author{{{author}}}";
        let result = resolver.substitute(content, &values).unwrap();

        assert_eq!(result, "\\title{My Document}\n\\author{Jane Doe}");
    }

    #[test]
    fn test_unresolved_tokens_error() {
        let resolver = PlaceholderResolver {
            cli_args: HashMap::new(),
            project_config: HashMap::new(),
            user_config: None,
            target_dir: PathBuf::new(),
        };

        let values = HashMap::new();
        let content = "\\title{{{title}}}";
        let result = resolver.substitute(content, &values);

        assert!(result.is_err());
    }

    #[test]
    fn test_resolve_project_config_priority() {
        let mut project_config = HashMap::new();
        project_config.insert("title".to_string(), "From Project".to_string());

        let resolver = PlaceholderResolver {
            cli_args: HashMap::new(),
            project_config,
            user_config: None,
            target_dir: PathBuf::new(),
        };

        let ph = make_placeholder("title", true);
        let result = resolver.resolve(&ph).unwrap();
        assert_eq!(result, Some("From Project".to_string()));
    }

    #[test]
    fn test_resolve_default_value() {
        let resolver = PlaceholderResolver {
            cli_args: HashMap::new(),
            project_config: HashMap::new(),
            user_config: None,
            target_dir: PathBuf::new(),
        };

        let mut ph = make_placeholder("title", false);
        ph.default = Some("Default Title".to_string());

        let result = resolver.resolve(&ph).unwrap();
        assert_eq!(result, Some("Default Title".to_string()));
    }

    #[test]
    fn test_resolve_optional_not_found() {
        let resolver = PlaceholderResolver {
            cli_args: HashMap::new(),
            project_config: HashMap::new(),
            user_config: None,
            target_dir: PathBuf::new(),
        };

        let ph = make_placeholder("title", false);
        let result = resolver.resolve(&ph).unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn test_resolve_all_with_optional_skipped() {
        let resolver = PlaceholderResolver {
            cli_args: HashMap::new(),
            project_config: HashMap::new(),
            user_config: None,
            target_dir: PathBuf::new(),
        };

        let mut required = make_placeholder("title", true);
        required.default = Some("Default".to_string());
        let optional = make_placeholder("author", false);

        let result = resolver.resolve_all(&[required, optional]).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result.get("title"), Some(&"Default".to_string()));
    }

    #[test]
    fn test_resolve_all_multiple_required() {
        let mut cli_args = HashMap::new();
        cli_args.insert("a".to_string(), "1".to_string());
        cli_args.insert("b".to_string(), "2".to_string());

        let resolver = PlaceholderResolver {
            cli_args,
            project_config: HashMap::new(),
            user_config: None,
            target_dir: PathBuf::new(),
        };

        let ph_a = make_placeholder("a", true);
        let ph_b = make_placeholder("b", true);
        let result = resolver.resolve_all(&[ph_a, ph_b]).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result.get("a"), Some(&"1".to_string()));
        assert_eq!(result.get("b"), Some(&"2".to_string()));
    }

    #[test]
    fn test_substitute_no_tokens() {
        let resolver = PlaceholderResolver {
            cli_args: HashMap::new(),
            project_config: HashMap::new(),
            user_config: None,
            target_dir: PathBuf::new(),
        };

        let values = HashMap::new();
        let content = "plain text without tokens";
        let result = resolver.substitute(content, &values).unwrap();
        assert_eq!(result, "plain text without tokens");
    }

    #[test]
    fn test_flatten_toml_string() {
        let mut table = toml::Table::new();
        table.insert("key".to_string(), toml::Value::String("value".to_string()));
        let mut result = HashMap::new();
        flatten_toml(&table, "", &mut result);
        assert_eq!(result.get("key"), Some(&"value".to_string()));
    }

    #[test]
    fn test_flatten_toml_nested() {
        let mut inner = toml::Table::new();
        inner.insert(
            "nested".to_string(),
            toml::Value::String("deep".to_string()),
        );
        let mut table = toml::Table::new();
        table.insert("section".to_string(), toml::Value::Table(inner));
        let mut result = HashMap::new();
        flatten_toml(&table, "", &mut result);
        assert_eq!(result.get("section.nested"), Some(&"deep".to_string()));
    }

    #[test]
    fn test_flatten_toml_boolean() {
        let mut table = toml::Table::new();
        table.insert("flag".to_string(), toml::Value::Boolean(true));
        let mut result = HashMap::new();
        flatten_toml(&table, "", &mut result);
        assert_eq!(result.get("flag"), Some(&"true".to_string()));
    }

    #[test]
    fn test_flatten_toml_with_prefix() {
        let mut table = toml::Table::new();
        table.insert("key".to_string(), toml::Value::String("val".to_string()));
        let mut result = HashMap::new();
        flatten_toml(&table, "pre", &mut result);
        assert_eq!(result.get("pre.key"), Some(&"val".to_string()));
    }

    #[test]
    fn test_resolve_interpolation_user_name() {
        let mut user_config = config::Config::default();
        user_config.user.name = Some("Alice".to_string());

        let resolver = PlaceholderResolver {
            cli_args: HashMap::new(),
            project_config: HashMap::new(),
            user_config: Some(user_config),
            target_dir: PathBuf::new(),
        };

        let result = resolver.resolve_interpolations("{{user.name}}").unwrap();
        assert_eq!(result, "Alice");
    }

    #[test]
    fn test_resolve_interpolation_user_email() {
        let mut user_config = config::Config::default();
        user_config.user.email = Some("alice@test.com".to_string());

        let resolver = PlaceholderResolver {
            cli_args: HashMap::new(),
            project_config: HashMap::new(),
            user_config: Some(user_config),
            target_dir: PathBuf::new(),
        };

        let result = resolver.resolve_interpolations("{{user.email}}").unwrap();
        assert_eq!(result, "alice@test.com");
    }

    #[test]
    fn test_resolve_interpolation_institution() {
        let mut user_config = config::Config::default();
        user_config.institution.name = Some("MIT".to_string());

        let resolver = PlaceholderResolver {
            cli_args: HashMap::new(),
            project_config: HashMap::new(),
            user_config: Some(user_config),
            target_dir: PathBuf::new(),
        };

        let result = resolver
            .resolve_interpolations("{{institution.name}}")
            .unwrap();
        assert_eq!(result, "MIT");
    }

    #[test]
    fn test_resolve_interpolation_no_config() {
        let resolver = PlaceholderResolver {
            cli_args: HashMap::new(),
            project_config: HashMap::new(),
            user_config: None,
            target_dir: PathBuf::new(),
        };

        // No config and no usable target directory for the git fallback:
        // the token resolves to the empty string, never stays raw.
        let result = resolver.resolve_interpolations("{{user.name}}").unwrap();
        assert_eq!(result, "");
    }

    /// Without a texforge config and without any git identity (global and
    /// system config pointed at an empty file, inside the env lock), identity
    /// interpolations resolve to the empty string — and a mixed string keeps
    /// its non-identity text.
    #[test]
    fn test_resolve_interpolation_no_config_no_git_identity() {
        let _lock = crate::test_sync::ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let tmp = tempfile::tempdir().unwrap();
        let empty_git_config = tmp.path().join("empty.gitconfig");
        std::fs::write(&empty_git_config, "").unwrap();
        // Declared after the lock: locals drop in reverse declaration order,
        // so the environment is restored while ENV_LOCK is still held.
        let _restore = crate::test_sync::EnvGuard::capture(&[
            "HOME",
            "XDG_CONFIG_HOME",
            "GIT_CONFIG_GLOBAL",
            "GIT_CONFIG_SYSTEM",
        ]);
        std::env::set_var("HOME", tmp.path());
        std::env::set_var("XDG_CONFIG_HOME", tmp.path());
        std::env::set_var("GIT_CONFIG_GLOBAL", &empty_git_config);
        std::env::set_var("GIT_CONFIG_SYSTEM", &empty_git_config);

        let resolver = PlaceholderResolver {
            cli_args: HashMap::new(),
            project_config: HashMap::new(),
            user_config: None,
            target_dir: tmp.path().to_path_buf(),
        };

        let name = resolver.resolve_interpolations("{{user.name}}").unwrap();
        assert_eq!(name, "");
        let email = resolver.resolve_interpolations("{{user.email}}").unwrap();
        assert_eq!(email, "");
        let mixed = resolver
            .resolve_interpolations("{{user.name}} @ {{institution.name}}")
            .unwrap();
        assert_eq!(mixed, " @ ");
    }

    #[test]
    fn test_clear_unresolved_identities_only_touches_identity_tokens() {
        assert_eq!(
            clear_unresolved_identities("hi {{user.name}} / {{institution.name}}"),
            "hi  / "
        );
        // Non-identity tokens survive for the strict `substitute` path.
        assert_eq!(
            clear_unresolved_identities("\\title{{{title}}} {{language}}"),
            "\\title{{{title}}} {{language}}"
        );
        // An unterminated token is left as-is instead of eating the tail.
        assert_eq!(
            clear_unresolved_identities("a {{user.name"),
            "a {{user.name"
        );
    }

    #[test]
    fn test_resolve_from_user_config_author() {
        let mut user_config = config::Config::default();
        user_config.user.name = Some("Bob".to_string());

        let resolver = PlaceholderResolver {
            cli_args: HashMap::new(),
            project_config: HashMap::new(),
            user_config: Some(user_config),
            target_dir: PathBuf::new(),
        };

        let result =
            resolver.resolve_from_user_config(resolver.user_config.as_ref().unwrap(), "author");
        assert_eq!(result, Some("Bob".to_string()));
    }

    #[test]
    fn test_resolve_from_user_config_email() {
        let mut user_config = config::Config::default();
        user_config.user.email = Some("bob@test.com".to_string());

        let resolver = PlaceholderResolver {
            cli_args: HashMap::new(),
            project_config: HashMap::new(),
            user_config: Some(user_config),
            target_dir: PathBuf::new(),
        };

        let result =
            resolver.resolve_from_user_config(resolver.user_config.as_ref().unwrap(), "email");
        assert_eq!(result, Some("bob@test.com".to_string()));
    }

    #[test]
    fn test_resolve_from_user_config_institution() {
        let mut user_config = config::Config::default();
        user_config.institution.name = Some("Stanford".to_string());

        let resolver = PlaceholderResolver {
            cli_args: HashMap::new(),
            project_config: HashMap::new(),
            user_config: Some(user_config),
            target_dir: PathBuf::new(),
        };

        let result = resolver
            .resolve_from_user_config(resolver.user_config.as_ref().unwrap(), "institution");
        assert_eq!(result, Some("Stanford".to_string()));
    }

    #[test]
    fn test_resolve_from_user_config_documentclass() {
        let mut user_config = config::Config::default();
        user_config.defaults.documentclass = Some("report".to_string());

        let resolver = PlaceholderResolver {
            cli_args: HashMap::new(),
            project_config: HashMap::new(),
            user_config: Some(user_config),
            target_dir: PathBuf::new(),
        };

        let result = resolver
            .resolve_from_user_config(resolver.user_config.as_ref().unwrap(), "documentclass");
        assert_eq!(result, Some("report".to_string()));
    }

    #[test]
    fn test_resolve_from_user_config_language() {
        let mut user_config = config::Config::default();
        user_config.defaults.language = Some("spanish".to_string());

        let resolver = PlaceholderResolver {
            cli_args: HashMap::new(),
            project_config: HashMap::new(),
            user_config: Some(user_config),
            target_dir: PathBuf::new(),
        };

        let result =
            resolver.resolve_from_user_config(resolver.user_config.as_ref().unwrap(), "language");
        assert_eq!(result, Some("spanish".to_string()));
    }

    #[test]
    fn test_resolve_from_user_config_unknown_key() {
        let user_config = config::Config::default();

        let resolver = PlaceholderResolver {
            cli_args: HashMap::new(),
            project_config: HashMap::new(),
            user_config: Some(user_config),
            target_dir: PathBuf::new(),
        };

        let result = resolver
            .resolve_from_user_config(resolver.user_config.as_ref().unwrap(), "unknown_key");
        assert_eq!(result, None);
    }

    #[test]
    fn test_cli_overrides_project_config() {
        let mut cli_args = HashMap::new();
        cli_args.insert("title".to_string(), "CLI Value".to_string());
        let mut project_config = HashMap::new();
        project_config.insert("title".to_string(), "Project Value".to_string());

        let resolver = PlaceholderResolver {
            cli_args,
            project_config,
            user_config: None,
            target_dir: PathBuf::new(),
        };

        let ph = make_placeholder("title", true);
        let result = resolver.resolve(&ph).unwrap();
        assert_eq!(result, Some("CLI Value".to_string()));
    }
}
