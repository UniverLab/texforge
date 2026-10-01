//! Configuration commands: simple key access and interactive wizard
//!
//! Usage:
//!   texforge config                      # Interactive wizard
//!   texforge config list                 # List all settings
//!   texforge config name                 # Show value
//!   texforge config name "Jheison"       # Set value

use crate::config;
use anyhow::Result;
use inquire::{Select, Text};

const AVAILABLE_LANGUAGES: &[&str] = &[
    "english",
    "spanish",
    "french",
    "german",
    "portuguese",
    "italian",
    "dutch",
    "russian",
    "chinese",
    "japanese",
];

const BANNER: &str = r#"
 ███████████          █████ █████ ███████████                                     
░█░░░███░░░█         ░░███ ░░███ ░░███░░░░░░█                                     
░   ░███  ░   ██████  ░░███ ███   ░███   █ ░   ██████  ████████   ███████  ██████ 
    ░███     ███░░███  ░░█████    ░███████    ███░░███░░███░░███ ███░░███ ███░░███
    ░███    ░███████    ███░███   ░███░░░█   ░███ ░███ ░███ ░░░ ░███ ░███░███████ 
    ░███    ░███░░░    ███ ░░███  ░███  ░    ░███ ░███ ░███     ░███ ░███░███░░░  
    █████   ░░██████  █████ █████ █████      ░░██████  █████    ░░███████░░██████ 
   ░░░░░     ░░░░░░  ░░░░░ ░░░░░ ░░░░░        ░░░░░░  ░░░░░      ░░░░░███ ░░░░░░  
                                                                 ███ ░███         
                                                                ░░██████          
                                                                 ░░░░░░           
"#;

/// Get a config value by key (e.g., "name", "email", "institution", "language")
pub fn get(key: &str) -> Result<()> {
    let config = config::load()?;

    match key {
        "name" => {
            if let Some(name) = &config.user.name {
                println!("{}", name);
            } else {
                println!("(not set)");
            }
        }
        "email" => {
            if let Some(email) = &config.user.email {
                println!("{}", email);
            } else {
                println!("(not set)");
            }
        }
        "institution" => {
            if let Some(inst) = &config.institution.name {
                println!("{}", inst);
            } else {
                println!("(not set)");
            }
        }
        "language" => {
            if let Some(lang) = &config.defaults.language {
                println!("{}", lang);
            } else {
                println!("(not set)");
            }
        }
        _ => {
            anyhow::bail!(
                "Unknown config key: {}. Available: name, email, institution, language",
                key
            );
        }
    }

    Ok(())
}

/// Set a config value by key
pub fn set(key: &str, value: &str) -> Result<()> {
    let mut config = config::load()?;

    match key {
        "name" => {
            config.user.name = Some(value.to_string());
        }
        "email" => {
            config.user.email = Some(value.to_string());
        }
        "institution" => {
            config.institution.name = Some(value.to_string());
        }
        "language" => {
            config.defaults.language = Some(value.to_string());
        }
        _ => {
            anyhow::bail!(
                "Unknown config key: {}. Available: name, email, institution, language",
                key
            );
        }
    }

    config::save(&config)?;
    println!("✓ Set {} = {}", key, value);
    Ok(())
}

/// List all configuration values
pub fn list() -> Result<()> {
    let config = config::load()?;

    println!("Global configuration:\n");

    println!("[User]");
    if let Some(name) = &config.user.name {
        println!("  name       = {}", name);
    } else {
        println!("  name       = (not set)");
    }
    if let Some(email) = &config.user.email {
        println!("  email      = {}", email);
    } else {
        println!("  email      = (not set)");
    }

    println!();
    println!("[Institution]");
    if let Some(inst) = &config.institution.name {
        println!("  name       = {}", inst);
    } else {
        println!("  name       = (not set)");
    }

    println!();
    println!("[Defaults]");
    if let Some(lang) = &config.defaults.language {
        println!("  language   = {}", lang);
    } else {
        println!("  language   = (not set)");
    }

    Ok(())
}

/// Interactive configuration wizard - asks for all 4 main fields
pub fn wizard() -> Result<()> {
    println!("{BANNER}");
    println!("Configuration Wizard\n");
    println!("Fill in your details to be used as placeholders in templates:\n");

    let config = config::load()?;

    let name = Text::new("Name")
        .with_default(config.user.name.as_deref().unwrap_or(""))
        .prompt()?;

    let email = Text::new("Email")
        .with_default(config.user.email.as_deref().unwrap_or("email@domain.com"))
        .prompt()?;

    let institution = Text::new("Institution")
        .with_default(config.institution.name.as_deref().unwrap_or(""))
        .prompt()?;

    let default_lang = config.defaults.language.as_deref().unwrap_or("english");
    let language_options: Vec<&str> = AVAILABLE_LANGUAGES.to_vec();

    let selected_language = if AVAILABLE_LANGUAGES.contains(&default_lang) {
        Select::new("Language", language_options)
            .with_help_message("↑↓ move  enter confirm")
            .prompt_skippable()
            .map(|opt| opt.unwrap_or(default_lang))
    } else {
        Select::new("Language", language_options)
            .with_help_message("↑↓ move  enter confirm")
            .prompt()
    };

    let language = selected_language?;

    // Save all values
    let mut new_config = config::load()?;
    new_config.user.name = Some(name);
    new_config.user.email = Some(email);
    new_config.institution.name = Some(institution);
    new_config.defaults.language = Some(language.to_string());

    config::save(&new_config)?;

    println!("\n✓ Configuration saved!");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs `f` with `HOME` and `XDG_CONFIG_HOME` both pointing at the same
    /// fresh tempdir, holding [`crate::test_sync::ENV_LOCK`] for the whole
    /// span. The lock is what makes the roundtrips below deterministic: both
    /// variables live in the process environment while Rust runs tests on
    /// parallel threads, so two overlapping tests would swap them out from
    /// under each other and read each other's temp directory. Pointing
    /// `HOME` at the tempdir too is what makes them *hermetic* — if
    /// `config_file_path` ever stops honouring `XDG_CONFIG_HOME`, the write
    /// lands under the tempdir instead of the developer's real
    /// `~/.texforge/config.toml`. Both variables are restored by
    /// [`crate::test_sync::EnvGuard`]'s `Drop`, so restoration also happens
    /// when `f` panics. A poisoned lock is recovered with `into_inner`: the
    /// failure itself is reported by the panicking test.
    fn with_temp_config(f: impl FnOnce()) {
        let _lock = crate::test_sync::ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let tmp = tempfile::tempdir().unwrap();
        // Declared after the lock: locals drop in reverse declaration order,
        // so the environment is restored while ENV_LOCK is still held,
        // before the next test may swap environment variables.
        let _restore = crate::test_sync::EnvGuard::capture(&["HOME", "XDG_CONFIG_HOME"]);
        std::env::set_var("HOME", tmp.path());
        std::env::set_var("XDG_CONFIG_HOME", tmp.path());
        f();
    }

    #[test]
    fn get_name_set_and_retrieve() {
        with_temp_config(|| {
            set("name", "Alice").unwrap();
            get("name").unwrap();
        });
    }

    #[test]
    fn get_email_set_and_retrieve() {
        with_temp_config(|| {
            set("email", "alice@test.com").unwrap();
            get("email").unwrap();
        });
    }

    #[test]
    fn get_institution_set_and_retrieve() {
        with_temp_config(|| {
            set("institution", "MIT").unwrap();
            get("institution").unwrap();
        });
    }

    #[test]
    fn get_language_set_and_retrieve() {
        with_temp_config(|| {
            set("language", "spanish").unwrap();
            get("language").unwrap();
        });
    }

    #[test]
    fn get_unknown_key_errors() {
        with_temp_config(|| {
            let result = get("unknown");
            assert!(result.is_err());
        });
    }

    #[test]
    fn set_unknown_key_errors() {
        with_temp_config(|| {
            let result = set("unknown", "value");
            assert!(result.is_err());
        });
    }

    #[test]
    fn get_unset_shows_not_set() {
        with_temp_config(|| {
            // name not set, should print "(not set)"
            get("name").unwrap();
        });
    }

    #[test]
    fn list_displays_all_sections() {
        with_temp_config(|| {
            set("name", "Bob").unwrap();
            set("email", "bob@test.com").unwrap();
            set("institution", "Stanford").unwrap();
            set("language", "english").unwrap();
            list().unwrap();
        });
    }

    #[test]
    fn list_with_unset_values() {
        with_temp_config(|| {
            // All unset — should print "(not set)" for each
            list().unwrap();
        });
    }

    #[test]
    fn set_then_get_roundtrip() {
        with_temp_config(|| {
            set("name", "Test").unwrap();
            get("name").unwrap();
            set("email", "test@test.com").unwrap();
            get("email").unwrap();
        });
    }

    #[test]
    fn set_overwrites_existing() {
        with_temp_config(|| {
            set("name", "First").unwrap();
            set("name", "Second").unwrap();
            get("name").unwrap();
        });
    }

    /// Inside `with_temp_config` every config path must live under the temp
    /// directory — whatever `config_file_path` does, the real home is
    /// unreachable.
    #[test]
    fn config_writes_never_reach_real_home() {
        let real_home = std::env::var_os("HOME");
        with_temp_config(|| {
            let path = config::config_file_path().unwrap();
            let temp_home = std::env::var_os("HOME").expect("with_temp_config sets HOME");
            assert!(
                path.starts_with(&temp_home),
                "config_file_path must stay inside the temp HOME: {path:?}"
            );
            if let Some(real) = real_home {
                assert!(
                    !path.starts_with(&real),
                    "config_file_path must never point into the real home: {path:?}"
                );
            }
        });
    }

    /// With only `HOME` set (to a tempdir) and `XDG_CONFIG_HOME` removed, the
    /// fallback branch must write under `<tempdir>/.texforge/` — never the
    /// real `~/.texforge`. This covers the branch a regression that skips the
    /// XDG lookup would take, which is exactly how a test once wrote into the
    /// developer's real config.
    #[test]
    fn save_falls_back_under_temp_home_when_xdg_is_unset() {
        let _lock = crate::test_sync::ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let tmp = tempfile::tempdir().unwrap();
        let _restore = crate::test_sync::EnvGuard::capture(&["HOME", "XDG_CONFIG_HOME"]);
        std::env::set_var("HOME", tmp.path());
        std::env::remove_var("XDG_CONFIG_HOME");

        let mut cfg = config::Config::default();
        cfg.user.email = Some("isolation@test.com".to_string());
        config::save(&cfg).unwrap();

        let expected = tmp.path().join(".texforge").join("config.toml");
        assert_eq!(config::config_file_path().unwrap(), expected);
        assert!(
            expected.exists(),
            "config::save must write under the temp HOME"
        );
        let body = std::fs::read_to_string(&expected).unwrap();
        assert!(
            body.contains("isolation@test.com"),
            "the file under the temp HOME must carry what was saved"
        );
    }
}
