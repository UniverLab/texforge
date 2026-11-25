//! Installing a template from a local folder.

use std::path::Path;

use anyhow::{Context, Result};

use crate::manifest::TemplateManifest;
use crate::utils;

use super::safety::is_safe_relative;
use super::LOCAL_SOURCE_MARKER;

/// True when `name` is a plain registry template name: non-empty and made of
/// ASCII letters, digits, `-` and `_` only. This also makes it safe to use as
/// a single directory name.
pub(crate) fn is_plain_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Install the template living in `source` under its manifest `id`.
///
/// Returns the installed name.
pub(crate) fn install_from_dir(source: &Path, force: bool) -> Result<String> {
    let manifest_path = source.join("template.toml");
    if !manifest_path.is_file() {
        anyhow::bail!(
            "No template.toml in '{}'; a template needs a manifest",
            source.display()
        );
    }
    let manifest = TemplateManifest::from_file(&manifest_path)?;
    if !is_plain_name(&manifest.id) {
        anyhow::bail!(
            "Template id '{}' is not a plain name (ASCII letters, digits, '-' and '_' only)",
            manifest.id
        );
    }
    if !source.join("main.tex").is_file() {
        anyhow::bail!("No main.tex in '{}'", source.display());
    }

    let dest = utils::templates_dir()?.join(&manifest.id);
    if dest.exists() {
        if !force {
            anyhow::bail!(
                "Template '{}' already exists; pass --force to replace it",
                manifest.id
            );
        }
        std::fs::remove_dir_all(&dest)?;
    }
    copy_dir(source, &dest)?;
    std::fs::write(dest.join(LOCAL_SOURCE_MARKER), "")?;
    Ok(manifest.id)
}

/// Copy every regular file and directory under `source` into `dest`,
/// skipping anything else (symlinks included) and any path that escapes it.
fn copy_dir(source: &Path, dest: &Path) -> Result<()> {
    std::fs::create_dir_all(dest)?;
    for entry in walkdir::WalkDir::new(source)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let rel = entry
            .path()
            .strip_prefix(source)
            .with_context(|| format!("Failed to relativize '{}'", entry.path().display()))?
            .to_string_lossy()
            .to_string();
        if rel.is_empty() || !is_safe_relative(&rel) {
            continue;
        }
        let file_type = entry.file_type();
        if file_type.is_dir() {
            std::fs::create_dir_all(dest.join(&rel))?;
        } else if file_type.is_file() {
            let target = dest.join(&rel);
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write_fixture(dir: &Path, id: &str) {
        fs::create_dir_all(dir).unwrap();
        fs::write(
            dir.join("template.toml"),
            format!(
                "id = \"{id}\"\nversion = \"0.1.0\"\ndisplay_name = \"Fixture\"\ndescription = \"Fixture template\"\n"
            ),
        )
        .unwrap();
        fs::write(dir.join("main.tex"), "\\documentclass{article}").unwrap();
    }

    fn with_temp_home(f: impl FnOnce(&Path)) {
        let _lock = crate::test_sync::ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let tmp = tempfile::tempdir().unwrap();
        let _restore = crate::test_sync::EnvGuard::capture(&["HOME", "XDG_CONFIG_HOME"]);
        std::env::set_var("HOME", tmp.path());
        std::env::set_var("XDG_CONFIG_HOME", tmp.path());
        f(tmp.path());
    }

    fn installed_dir(home: &Path, id: &str) -> std::path::PathBuf {
        home.join(".texforge").join("templates").join(id)
    }

    #[test]
    fn plain_names_are_ascii_letters_digits_dash_underscore() {
        assert!(is_plain_name("apa-general_2"));
        assert!(!is_plain_name(""));
        assert!(!is_plain_name("a/b"));
        assert!(!is_plain_name("../evil"));
        assert!(!is_plain_name("https://x.y/z"));
    }

    #[test]
    fn installs_a_valid_folder_recursively() {
        with_temp_home(|home| {
            let src = home.join("src");
            write_fixture(&src, "lab-report");
            fs::create_dir_all(src.join("sections")).unwrap();
            fs::write(src.join("sections/body.tex"), "body").unwrap();
            fs::write(src.join("refs.bib"), "@misc{a}").unwrap();

            let id = install_from_dir(&src, false).unwrap();
            assert_eq!(id, "lab-report");

            let dest = installed_dir(home, "lab-report");
            assert_eq!(
                fs::read_to_string(dest.join("main.tex")).unwrap(),
                "\\documentclass{article}"
            );
            assert_eq!(
                fs::read_to_string(dest.join("sections/body.tex")).unwrap(),
                "body"
            );
            assert_eq!(
                fs::read_to_string(dest.join("refs.bib")).unwrap(),
                "@misc{a}"
            );
            let marker = dest.join(LOCAL_SOURCE_MARKER);
            assert!(marker.is_file());
            assert_eq!(fs::read(&marker).unwrap(), b"");
            assert!(!dest.join(".cache_meta.json").exists());
        });
    }

    #[test]
    fn install_fails_without_a_manifest() {
        with_temp_home(|home| {
            let src = home.join("src");
            fs::create_dir_all(&src).unwrap();
            fs::write(src.join("main.tex"), "x").unwrap();
            let err = install_from_dir(&src, false).unwrap_err().to_string();
            assert!(err.contains("No template.toml"), "{err}");
            assert!(!installed_dir(home, "src").exists());
        });
    }

    #[test]
    fn install_fails_without_main_tex() {
        with_temp_home(|home| {
            let src = home.join("src");
            write_fixture(&src, "lab-report");
            fs::remove_file(src.join("main.tex")).unwrap();
            let err = install_from_dir(&src, false).unwrap_err().to_string();
            assert!(err.contains("No main.tex"), "{err}");
        });
    }

    #[test]
    fn install_rejects_a_manifest_id_that_is_not_plain() {
        with_temp_home(|home| {
            let src = home.join("src");
            write_fixture(&src, "../evil");
            let err = install_from_dir(&src, false).unwrap_err().to_string();
            assert!(err.contains("is not a plain name"), "{err}");
            assert!(!home.join(".texforge/templates/evil").exists());
        });
    }

    #[test]
    fn collision_needs_force_and_force_replaces() {
        with_temp_home(|home| {
            let src = home.join("src");
            write_fixture(&src, "lab-report");
            install_from_dir(&src, false).unwrap();

            let stale = installed_dir(home, "lab-report").join("stale.tex");
            fs::write(&stale, "stale").unwrap();

            let err = install_from_dir(&src, false).unwrap_err().to_string();
            assert_eq!(
                err,
                "Template 'lab-report' already exists; pass --force to replace it"
            );

            install_from_dir(&src, true).unwrap();
            assert!(!stale.exists());
            assert!(installed_dir(home, "lab-report").join("main.tex").is_file());
        });
    }
}
