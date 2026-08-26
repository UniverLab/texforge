//! `texforge uninstall` command implementation.
//!
//! Removes everything texforge manages under `~/.texforge`: the downloaded
//! Tectonic engine, the template cache, the dictionary cache, configuration,
//! and (optionally) the personal spell dictionary. The texforge binary itself
//! is never touched — the command reports how it was installed so the user
//! can remove it themselves.
//!
//! The personal spell dictionary (`~/.texforge/spell-words`) is the user's
//! own writing, not a cache. It is preserved by default and only removed
//! when `--include-spell-words` is passed.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

#[derive(Debug, Clone)]
struct PlanItem {
    label: &'static str,
    path: PathBuf,
    is_dir: bool,
    exists: bool,
    size: u64,
    is_spell_words: bool,
}

#[derive(Debug)]
struct Plan {
    items: Vec<PlanItem>,
    binary_note: String,
    spell_words_included: bool,
}

pub fn execute(yes: bool, dry_run: bool, include_spell_words: bool) -> Result<()> {
    let Some(data_dir) = texforge_data_dir() else {
        println!("Could not determine home directory.");
        return Ok(());
    };
    execute_at(&data_dir, yes, dry_run, include_spell_words)
}

fn execute_at(data_dir: &Path, yes: bool, dry_run: bool, include_spell_words: bool) -> Result<()> {
    let plan = build_plan_at(data_dir, include_spell_words)?;
    print_plan(&plan);

    if dry_run {
        println!();
        println!("Dry run — nothing was removed.");
        return Ok(());
    }

    let removable: Vec<&PlanItem> = plan
        .items
        .iter()
        .filter(|item| item.exists && (!item.is_spell_words || plan.spell_words_included))
        .collect();

    if removable.is_empty() {
        println!();
        println!("Nothing to remove.");
        return Ok(());
    }

    if !yes {
        let confirmed = inquire::Confirm::new("  Proceed with removal?")
            .with_default(false)
            .prompt()
            .context("failed to read confirmation")?;
        if !confirmed {
            println!("Aborted.");
            return Ok(());
        }
    }

    execute_plan(&removable)?;

    try_remove_data_dir(data_dir)?;

    println!();
    println!("{}", plan.binary_note);

    Ok(())
}

fn build_plan_at(data_dir: &Path, include_spell_words: bool) -> Result<Plan> {
    let mut items = Vec::new();

    let bin_dir = data_dir.join("bin");
    items.push(PlanItem {
        label: "Managed Tectonic engine",
        exists: bin_dir.exists(),
        size: dir_size(&bin_dir),
        path: bin_dir,
        is_dir: true,
        is_spell_words: false,
    });

    let templates_dir = data_dir.join("templates");
    items.push(PlanItem {
        label: "Template cache",
        exists: templates_dir.exists(),
        size: dir_size(&templates_dir),
        path: templates_dir,
        is_dir: true,
        is_spell_words: false,
    });

    let dicts_dir = data_dir.join("dicts");
    items.push(PlanItem {
        label: "Dictionary cache",
        exists: dicts_dir.exists(),
        size: dir_size(&dicts_dir),
        path: dicts_dir,
        is_dir: true,
        is_spell_words: false,
    });

    let spell_words = data_dir.join("spell-words");
    items.push(PlanItem {
        label: "Personal spell dictionary (your own writing)",
        exists: spell_words.exists(),
        size: file_size(&spell_words),
        path: spell_words,
        is_dir: false,
        is_spell_words: true,
    });

    let other_items = collect_other_items(data_dir, &items);
    items.extend(other_items);

    let binary_note = binary_install_note();

    Ok(Plan {
        items,
        binary_note,
        spell_words_included: include_spell_words,
    })
}

fn collect_other_items(data_dir: &Path, existing: &[PlanItem]) -> Vec<PlanItem> {
    let mut others = Vec::new();
    let Ok(entries) = std::fs::read_dir(data_dir) else {
        return others;
    };
    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str == "bin"
            || name_str == "templates"
            || name_str == "dicts"
            || name_str == "spell-words"
        {
            continue;
        }
        if existing.iter().any(|item| item.path == path) {
            continue;
        }
        let is_dir = path.is_dir();
        let size = if is_dir {
            dir_size(&path)
        } else {
            file_size(&path)
        };
        let label: &'static str = if name_str == "config.toml" {
            "Configuration"
        } else {
            Box::leak(format!("Other: {name_str}").into_boxed_str()) as &str
        };
        others.push(PlanItem {
            label,
            path,
            is_dir,
            exists: true,
            size,
            is_spell_words: false,
        });
    }
    others
}

fn print_plan(plan: &Plan) {
    println!("texforge uninstall");
    println!();

    let mut total: u64 = 0;
    let mut any_exists = false;

    for item in &plan.items {
        if item.is_spell_words && !plan.spell_words_included {
            println!(
                "  {} (preserved — pass --include-spell-words to remove)",
                item.label
            );
            if item.exists {
                println!("    path: {}", item.path.display());
                println!("    size: {}", format_size(item.size));
                any_exists = true;
            }
            continue;
        }
        if !item.exists {
            println!("  {} — not present", item.label);
            continue;
        }
        any_exists = true;
        println!("  {} — {}", item.label, format_size(item.size));
        println!("    path: {}", item.path.display());
        total += item.size;
    }

    println!();
    if any_exists {
        println!("  Total to remove: {}", format_size(total));
    } else {
        println!("  Nothing present under ~/.texforge.");
    }
}

fn execute_plan(items: &[&PlanItem]) -> Result<()> {
    println!();
    let mut any_failed = false;
    for item in items {
        print!("  Removing {}... ", item.label);
        let result = if item.is_dir {
            std::fs::remove_dir_all(&item.path)
        } else {
            std::fs::remove_file(&item.path)
        };
        match result {
            Ok(()) => println!("done"),
            Err(e) => {
                println!("FAILED ({e})");
                any_failed = true;
            }
        }
    }
    if any_failed {
        eprintln!();
        eprintln!("Some components could not be removed. You may need to delete them manually.");
    }
    Ok(())
}

fn try_remove_data_dir(data_dir: &Path) -> Result<()> {
    if !data_dir.exists() {
        return Ok(());
    }
    let Ok(entries) = std::fs::read_dir(data_dir) else {
        return Ok(());
    };
    if entries.filter_map(|e| e.ok()).next().is_some() {
        return Ok(());
    }
    let _ = std::fs::remove_dir(data_dir);
    Ok(())
}

fn texforge_data_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".texforge"))
}

fn dir_size(dir: &Path) -> u64 {
    let mut total = 0u64;
    for entry in walkdir::WalkDir::new(dir)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if entry.file_type().is_file() {
            if let Ok(meta) = entry.metadata() {
                total += meta.len();
            }
        }
    }
    total
}

fn file_size(path: &Path) -> u64 {
    std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}

fn format_size(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    format!("{size:.1} {}", UNITS[unit])
}

fn binary_install_note() -> String {
    let Ok(current_exe) = std::env::current_exe() else {
        return "The texforge binary was not removed. Could not determine its location.".into();
    };

    let is_cargo = crate::version_checker::current_exe_is_cargo_managed(&current_exe);

    if is_cargo {
        format!(
            "The texforge binary was NOT removed.\n\
             It was installed via cargo and is managed by cargo.\n\
             To remove it, run: cargo uninstall texforge\n\
             Binary location: {}",
            current_exe.display()
        )
    } else {
        format!(
            "The texforge binary was NOT removed.\n\
             To remove it manually, run: rm -f {}\n\
             (or the equivalent for your install method)",
            current_exe.display()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn plan_lists_each_component() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join(".texforge");
        fs::create_dir_all(&dir).unwrap();
        fs::create_dir_all(dir.join("bin")).unwrap();
        fs::create_dir_all(dir.join("templates")).unwrap();
        fs::create_dir_all(dir.join("dicts")).unwrap();
        fs::write(dir.join("spell-words"), "hello\n").unwrap();
        fs::write(dir.join("config.toml"), "[user]\n").unwrap();

        let plan = build_plan_at(&dir, false).unwrap();

        let labels: Vec<&str> = plan.items.iter().map(|i| i.label).collect();
        assert!(labels.contains(&"Managed Tectonic engine"));
        assert!(labels.contains(&"Template cache"));
        assert!(labels.contains(&"Dictionary cache"));
        assert!(labels.contains(&"Personal spell dictionary (your own writing)"));
        assert!(labels.contains(&"Configuration"));
    }

    #[test]
    fn dry_run_leaves_filesystem_untouched() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join(".texforge");
        fs::create_dir_all(&dir).unwrap();
        fs::create_dir_all(dir.join("templates")).unwrap();
        fs::write(dir.join("spell-words"), "myword\n").unwrap();

        execute_at(&dir, false, true, false).unwrap();

        assert!(dir.join("templates").exists());
        assert!(dir.join("spell-words").exists());
    }

    #[test]
    fn spell_dictionary_survives_default_uninstall() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join(".texforge");
        fs::create_dir_all(&dir).unwrap();
        fs::create_dir_all(dir.join("templates")).unwrap();
        fs::write(dir.join("spell-words"), "myword\n").unwrap();

        execute_at(&dir, true, false, false).unwrap();

        assert!(dir.join("spell-words").exists());
        assert!(!dir.join("templates").exists());
    }

    #[test]
    fn spell_dictionary_removed_when_explicitly_requested() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join(".texforge");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("spell-words"), "myword\n").unwrap();

        execute_at(&dir, true, false, true).unwrap();

        assert!(!dir.join("spell-words").exists());
    }

    #[test]
    fn missing_component_is_skipped_not_erroring() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join(".texforge");
        fs::create_dir_all(&dir).unwrap();
        fs::create_dir_all(dir.join("templates")).unwrap();

        let result = execute_at(&dir, true, false, false);
        assert!(result.is_ok());
        assert!(!dir.join("templates").exists());
    }

    #[test]
    fn missing_spell_words_with_include_flag_is_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join(".texforge");
        fs::create_dir_all(&dir).unwrap();
        fs::create_dir_all(dir.join("templates")).unwrap();
        assert!(!dir.join("spell-words").exists());

        let result = execute_at(&dir, true, false, true);
        assert!(result.is_ok());
        assert!(!dir.join("templates").exists());
    }

    #[test]
    fn format_size_bytes() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(512), "512 B");
    }

    #[test]
    fn format_size_kilobytes() {
        assert_eq!(format_size(2048), "2.0 KB");
    }

    #[test]
    fn format_size_megabytes() {
        assert_eq!(format_size(65 * 1024 * 1024), "65.0 MB");
    }

    #[test]
    fn dir_size_missing_dir_is_zero() {
        let tmp = tempfile::tempdir().unwrap();
        let missing = tmp.path().join("does-not-exist");
        assert_eq!(dir_size(&missing), 0);
    }

    #[test]
    fn dir_size_counts_nested_files() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("a"), b"1234").unwrap();
        let sub = tmp.path().join("sub");
        fs::create_dir_all(&sub).unwrap();
        fs::write(sub.join("b"), b"12345678").unwrap();
        assert_eq!(dir_size(tmp.path()), 12);
    }

    #[test]
    fn plan_spell_words_marked_correctly() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join(".texforge");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("spell-words"), "word\n").unwrap();

        let plan = build_plan_at(&dir, false).unwrap();
        let spell_item = plan.items.iter().find(|i| i.is_spell_words).unwrap();
        assert!(spell_item.exists);
        assert_eq!(
            spell_item.label,
            "Personal spell dictionary (your own writing)"
        );
    }

    #[test]
    fn plan_with_no_spell_words_flag_still_lists_it() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join(".texforge");
        fs::create_dir_all(&dir).unwrap();

        let plan = build_plan_at(&dir, false).unwrap();
        assert!(plan.items.iter().any(|i| i.is_spell_words));
        assert!(!plan.spell_words_included);
    }

    #[test]
    fn empty_data_dir_reports_nothing_present() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join(".texforge");
        fs::create_dir_all(&dir).unwrap();

        let plan = build_plan_at(&dir, false).unwrap();
        let existing_items: Vec<_> = plan.items.iter().filter(|i| i.exists).collect();
        assert!(existing_items.is_empty());
    }

    #[test]
    fn data_dir_removed_when_empty_after_uninstall() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join(".texforge");
        fs::create_dir_all(&dir).unwrap();
        fs::create_dir_all(dir.join("templates")).unwrap();

        execute_at(&dir, true, false, false).unwrap();

        assert!(!dir.exists());
    }

    #[test]
    fn data_dir_kept_when_spell_words_preserved() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join(".texforge");
        fs::create_dir_all(&dir).unwrap();
        fs::create_dir_all(dir.join("templates")).unwrap();
        fs::write(dir.join("spell-words"), "word\n").unwrap();

        execute_at(&dir, true, false, false).unwrap();

        assert!(dir.exists());
        assert!(dir.join("spell-words").exists());
    }
}
