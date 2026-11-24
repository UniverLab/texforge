//! Origin report for `texforge new`.
//!
//! Kept in its own submodule because `new.rs` is already over 800 lines.
//! After resolution, and before writing files, `new` prints one line per
//! manifest placeholder saying where the value came from — except values
//! the user supplied explicitly through `--values` files, which print
//! nothing — so a filled `author` or `institution` can always be traced.

use std::collections::HashMap;
use std::path::PathBuf;

use crate::manifest::TemplateManifest;
use crate::placeholders::{interpolation_tokens, InterpolationOrigin, PlaceholderOrigin};

/// One report line per manifest placeholder, in manifest order. Skips keys
/// supplied by `--values` files (the user's explicit input) and placeholders
/// that resolved to nothing. Every other origin is reported.
pub(crate) fn origin_lines(
    manifest: Option<&TemplateManifest>,
    values: &HashMap<String, String>,
    origins: &HashMap<String, PlaceholderOrigin>,
    values_files: &HashMap<String, PathBuf>,
) -> Vec<String> {
    let Some(manifest) = manifest else {
        return Vec::new();
    };
    manifest
        .placeholders
        .iter()
        .filter(|placeholder| !values_files.contains_key(&placeholder.name))
        .filter_map(|placeholder| {
            let value = values.get(&placeholder.name)?;
            let origin = origins.get(&placeholder.name)?;
            Some(origin_line(
                &placeholder.name,
                value,
                origin,
                placeholder.default.as_deref(),
            ))
        })
        .collect()
}

/// Print the [`origin_lines`] report, one line per resolved placeholder.
pub(crate) fn print_origins(
    manifest: Option<&TemplateManifest>,
    values: &HashMap<String, String>,
    origins: &HashMap<String, PlaceholderOrigin>,
    values_files: &HashMap<String, PathBuf>,
) {
    for line in origin_lines(manifest, values, origins, values_files) {
        println!("{line}");
    }
}

/// `  ◇ <name> = "<value>" (<where it came from>)` in clear: the manifest
/// holds only `String`, `Boolean` and `Enum` placeholders, so there is no
/// secret type to redact and `new` has no `--quiet` flag.
fn origin_line(
    name: &str,
    value: &str,
    origin: &PlaceholderOrigin,
    default: Option<&str>,
) -> String {
    let note = match origin {
        PlaceholderOrigin::Cli => "(from command line)".to_string(),
        PlaceholderOrigin::ProjectConfig => {
            "(from project config ./.texforge/config.toml)".to_string()
        }
        PlaceholderOrigin::UserConfig(path) => {
            format!("(from user config {})", path.display())
        }
        PlaceholderOrigin::TemplateDefault { via } if via.is_empty() => {
            "(from template default)".to_string()
        }
        PlaceholderOrigin::TemplateDefault { via } => {
            let parts = default
                .map_or_else(Vec::new, interpolation_tokens)
                .into_iter()
                .zip(via)
                .map(|(token, origin)| match origin {
                    InterpolationOrigin::UserConfig(path) => {
                        format!("{token} from user config {}", path.display())
                    }
                    InterpolationOrigin::Git(key) => {
                        format!("{token} from git config {key}")
                    }
                    InterpolationOrigin::Empty => {
                        format!("{token} empty: no user config and no git identity")
                    }
                })
                .collect::<Vec<_>>()
                .join("; ");
            if parts.is_empty() {
                "(from template default)".to_string()
            } else {
                format!("(from template default; {parts})")
            }
        }
    };
    format!("  ◇ {name} = \"{value}\" {note}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// End-to-end shape of the `new` report: a one-screen template whose
    /// `author` comes from the temp user config, whose `institution` falls
    /// back to the template default, whose `codename` the user supplied
    /// through `--values` (silent), and whose `motto` resolves to nothing
    /// (silent). Exactly the two expected lines print, in manifest order.
    #[test]
    fn test_origin_lines_report() {
        let _lock = crate::test_sync::ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let tmp = tempfile::tempdir().unwrap();
        // Declared after the lock: locals drop in reverse declaration order,
        // so the environment is restored while ENV_LOCK is still held.
        let _restore = crate::test_sync::EnvGuard::capture(&["HOME", "XDG_CONFIG_HOME"]);
        std::env::set_var("HOME", tmp.path());
        std::env::set_var("XDG_CONFIG_HOME", tmp.path());
        let cfg_dir = tmp.path().join("texforge");
        std::fs::create_dir_all(&cfg_dir).unwrap();
        std::fs::write(cfg_dir.join("config.toml"), "[user]\nname = \"Alice\"\n").unwrap();
        let cfg_path = crate::config::config_file_path().unwrap();

        let manifest_toml = r#"
id = "probe"
version = "0.1.0"
display_name = "Probe"
description = "Probe template"

[[placeholders]]
name = "author"
type = "string"
description = "Author"
required = false

[[placeholders]]
name = "institution"
type = "string"
description = "Institution"
required = false
default = "{{institution.name}}"

[[placeholders]]
name = "codename"
type = "string"
description = "Codename"
required = false
default = "plain"

[[placeholders]]
name = "motto"
type = "string"
description = "Motto"
required = false
"#;
        let manifest = crate::manifest::TemplateManifest::from_str(manifest_toml).unwrap();
        let mut files = HashMap::new();
        files.insert(
            "template.toml".to_string(),
            manifest_toml.as_bytes().to_vec(),
        );

        // `codename` arrives merged into the resolver input exactly like a
        // `--values` key does, yet stays silent through the exclusion map.
        let mut cli_args = HashMap::new();
        cli_args.insert("codename".to_string(), "from-file".to_string());
        let (values, origins) = super::super::new::resolve_placeholder_values_with_origins(
            &files,
            cli_args,
            tmp.path(),
        );
        let mut values_files = HashMap::new();
        values_files.insert(
            "codename".to_string(),
            PathBuf::from("/tmp/fake/values.toml"),
        );

        assert_eq!(
            origin_lines(Some(&manifest), &values, &origins, &values_files),
            vec![
                format!(
                    "  ◇ author = \"Alice\" (from user config {})",
                    cfg_path.display()
                ),
                "  ◇ institution = \"\" (from template default; \
                 {{institution.name}} empty: no user config and no git identity)"
                    .to_string(),
            ]
        );

        // No manifest placeholders at all: nothing prints.
        assert_eq!(
            origin_lines(None, &values, &origins, &values_files),
            Vec::<String>::new()
        );
    }
}
