//! `texforge spell` — manage the personal spell-check whitelist (dictionary)
//! from the CLI, reusing the whitelist mechanism the linter already reads.
//!
//! Two scopes: the project (the first of `PROJECT_WHITELIST_FILES` that
//! already exists, or `.texforge/spell-words` if none do) and the global
//! personal dictionary (`~/.texforge/spell-words`, shared by every project).
//! Adding is append-only and never rewrites existing bytes; removing rewrites
//! the file with only the matching lines dropped.

use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::domain::project::Project;
use crate::linter::{global_whitelist_path, parse_whitelist_words, PROJECT_WHITELIST_FILES};

/// Subcommands of `texforge spell`.
#[derive(Debug, Clone)]
pub enum SpellAction {
    /// Add one or more words to the whitelist.
    Add { words: Vec<String>, global: bool },
    /// List the effective whitelist words for the scope.
    List { global: bool },
    /// Remove one or more words from the whitelist.
    Remove { words: Vec<String>, global: bool },
}

/// Dispatch a `texforge spell` action.
pub fn execute(action: SpellAction) -> Result<()> {
    match action {
        SpellAction::Add { words, global } => add(&words, global),
        SpellAction::List { global } => list(global),
        SpellAction::Remove { words, global } => remove(&words, global),
    }
}

/// Resolve the file `add`/`remove` should target for the given scope.
/// Global always resolves to `global_whitelist_path()`. Project resolves to
/// the first of `PROJECT_WHITELIST_FILES` that already exists, or
/// `.texforge/spell-words` if none do (decision 4) — this does not require
/// the file to exist yet, only its parent project.
fn target_file(global: bool) -> Result<PathBuf> {
    if global {
        return global_whitelist_path()
            .context("Could not determine home directory for the global personal dictionary");
    }

    let project = Project::load()?;
    for name in PROJECT_WHITELIST_FILES {
        let p = project.root.join(name);
        if p.exists() {
            return Ok(p);
        }
    }
    Ok(project.root.join(".texforge").join("spell-words"))
}

fn add(words: &[String], global: bool) -> Result<()> {
    if words.is_empty() {
        anyhow::bail!("No words given; usage: texforge spell add <WORD>...");
    }

    let path = target_file(global)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create {}", parent.display()))?;
    }

    // Single read to find out what's already there; the write below only
    // appends and never rewrites existing bytes.
    let existing = fs::read_to_string(&path).unwrap_or_default();
    let mut known = parse_whitelist_words(&existing);

    let mut to_append: Vec<&str> = Vec::new();
    for w in words {
        let wl = w.to_lowercase();
        if known.contains(&wl) {
            println!("  ◇ '{}' already present in {}", w, path.display());
            continue;
        }
        known.insert(wl);
        to_append.push(w);
    }

    if to_append.is_empty() {
        return Ok(());
    }

    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("Failed to open {}", path.display()))?;

    let mut buf = String::new();
    if !existing.is_empty() && !existing.ends_with('\n') {
        buf.push('\n');
    }
    for w in &to_append {
        buf.push_str(w);
        buf.push('\n');
    }
    file.write_all(buf.as_bytes())
        .with_context(|| format!("Failed to write to {}", path.display()))?;

    for w in &to_append {
        println!("  ◇ Added '{}' to {}", w, path.display());
    }

    Ok(())
}

fn remove(words: &[String], global: bool) -> Result<()> {
    if words.is_empty() {
        anyhow::bail!("No words given; usage: texforge spell remove <WORD>...");
    }

    let path = target_file(global)?;

    let Ok(content) = fs::read_to_string(&path) else {
        for w in words {
            println!(
                "  ◇ '{}' was not in {} ({} does not exist)",
                w,
                path.display(),
                path.display()
            );
        }
        return Ok(());
    };

    let targets: HashSet<String> = words.iter().map(|w| w.to_lowercase()).collect();
    let mut removed: HashSet<String> = HashSet::new();
    let mut kept_lines: Vec<&str> = Vec::new();

    for line in content.lines() {
        let trimmed = line.trim();
        let is_word_line = !trimmed.is_empty() && !trimmed.starts_with('#');
        if is_word_line && targets.contains(&trimmed.to_lowercase()) {
            removed.insert(trimmed.to_lowercase());
            continue;
        }
        kept_lines.push(line);
    }

    for w in words {
        if removed.contains(&w.to_lowercase()) {
            println!("  ◇ Removed '{}' from {}", w, path.display());
        } else {
            println!("  ◇ '{}' was not in {}", w, path.display());
        }
    }

    // Nothing matched: leave the file byte-for-byte untouched rather than
    // rewriting it to the same content.
    if removed.is_empty() {
        return Ok(());
    }

    let mut new_content = kept_lines.join("\n");
    if content.ends_with('\n') && !new_content.is_empty() {
        new_content.push('\n');
    }
    fs::write(&path, new_content).with_context(|| format!("Failed to write {}", path.display()))?;

    Ok(())
}

fn list(global: bool) -> Result<()> {
    if global {
        let path = global_whitelist_path()
            .context("Could not determine home directory for the global personal dictionary")?;
        print_whitelist_file(&path);
        return Ok(());
    }

    let project = Project::load()?;
    let mut any = false;
    for name in PROJECT_WHITELIST_FILES {
        let p = project.root.join(name);
        if p.exists() {
            any = true;
            print_whitelist_file(&p);
        }
    }
    if !any {
        println!(
            "  ◇ No project whitelist file exists yet (checked: {})",
            PROJECT_WHITELIST_FILES.join(", ")
        );
    }
    Ok(())
}

fn print_whitelist_file(path: &Path) {
    match fs::read_to_string(path) {
        Ok(content) => {
            let words = parse_whitelist_words(&content);
            let mut sorted: Vec<&String> = words.iter().collect();
            sorted.sort();
            println!("{} ({} word(s)):", path.display(), sorted.len());
            for w in sorted {
                println!("  {}", w);
            }
        }
        Err(_) => println!("{} (not found)", path.display()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::linter::spell as linter_spell;
    use tempfile::TempDir;

    /// A minimal project.toml so `Project::load()` succeeds from `cwd`.
    fn write_project_toml(root: &Path) {
        fs::write(
            root.join("project.toml"),
            r#"[document]
title = "Test"
author = "Test"
template = "general"

[build]
entry = "main.tex"
"#,
        )
        .unwrap();
    }

    /// Runs `body` with `cwd` set to `root` for the duration, then restores
    /// the original working directory. `Project::load()` reads
    /// `std::env::current_dir()`, so scope commands need a real cwd change.
    fn with_cwd<T>(root: &Path, body: impl FnOnce() -> T) -> T {
        let orig = std::env::current_dir().unwrap();
        std::env::set_current_dir(root).unwrap();
        let result = body();
        std::env::set_current_dir(orig).unwrap();
        result
    }

    fn with_home<T>(home: &Path, body: impl FnOnce() -> T) -> T {
        let orig_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", home);
        let result = body();
        match orig_home {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
        result
    }

    #[test]
    fn add_creates_texforge_spell_words_when_no_whitelist_file_exists() {
        let project = TempDir::new().unwrap();
        write_project_toml(project.path());

        with_cwd(project.path(), || {
            add(&["docker".to_string()], false).unwrap();
        });

        let target = project.path().join(".texforge").join("spell-words");
        assert!(target.exists());
        assert_eq!(fs::read_to_string(&target).unwrap(), "docker\n");
        assert!(!project.path().join("spell-whitelist.txt").exists());
    }

    #[test]
    fn add_appends_to_spell_whitelist_txt_when_it_already_exists_and_does_not_create_texforge_dir()
    {
        let project = TempDir::new().unwrap();
        write_project_toml(project.path());
        fs::write(project.path().join("spell-whitelist.txt"), "acme\n").unwrap();

        with_cwd(project.path(), || {
            add(&["docker".to_string()], false).unwrap();
        });

        assert_eq!(
            fs::read_to_string(project.path().join("spell-whitelist.txt")).unwrap(),
            "acme\ndocker\n"
        );
        assert!(!project
            .path()
            .join(".texforge")
            .join("spell-words")
            .exists());
    }

    #[test]
    fn add_appends_to_texforge_spell_words_when_only_that_one_exists() {
        let project = TempDir::new().unwrap();
        write_project_toml(project.path());
        fs::create_dir_all(project.path().join(".texforge")).unwrap();
        fs::write(
            project.path().join(".texforge").join("spell-words"),
            "acme\n",
        )
        .unwrap();

        with_cwd(project.path(), || {
            add(&["docker".to_string()], false).unwrap();
        });

        assert_eq!(
            fs::read_to_string(project.path().join(".texforge").join("spell-words")).unwrap(),
            "acme\ndocker\n"
        );
        assert!(!project.path().join("spell-whitelist.txt").exists());
    }

    #[test]
    fn appending_preserves_prior_content_exactly_including_comment_and_ordering() {
        let project = TempDir::new().unwrap();
        write_project_toml(project.path());
        fs::write(
            project.path().join("spell-whitelist.txt"),
            "zeta\n# a note about zeta\nalpha\n",
        )
        .unwrap();

        with_cwd(project.path(), || {
            add(&["docker".to_string()], false).unwrap();
        });

        assert_eq!(
            fs::read_to_string(project.path().join("spell-whitelist.txt")).unwrap(),
            "zeta\n# a note about zeta\nalpha\ndocker\n"
        );
    }

    #[test]
    fn adding_an_already_present_word_adds_no_line_case_insensitively() {
        let project = TempDir::new().unwrap();
        write_project_toml(project.path());
        fs::write(project.path().join("spell-whitelist.txt"), "docker\n").unwrap();

        with_cwd(project.path(), || {
            add(&["docker".to_string()], false).unwrap();
            add(&["Docker".to_string()], false).unwrap();
            add(&["DOCKER".to_string()], false).unwrap();
        });

        assert_eq!(
            fs::read_to_string(project.path().join("spell-whitelist.txt")).unwrap(),
            "docker\n"
        );
    }

    #[test]
    fn remove_deletes_only_the_matching_line_and_leaves_comments_and_others_intact() {
        let project = TempDir::new().unwrap();
        write_project_toml(project.path());
        fs::write(
            project.path().join("spell-whitelist.txt"),
            "zeta\n# a note about docker\ndocker\nalpha\n",
        )
        .unwrap();

        with_cwd(project.path(), || {
            remove(&["docker".to_string()], false).unwrap();
        });

        assert_eq!(
            fs::read_to_string(project.path().join("spell-whitelist.txt")).unwrap(),
            "zeta\n# a note about docker\nalpha\n"
        );
    }

    #[test]
    fn remove_of_an_absent_word_changes_the_file_not_at_all() {
        let project = TempDir::new().unwrap();
        write_project_toml(project.path());
        let original = "zeta\nalpha\n";
        fs::write(project.path().join("spell-whitelist.txt"), original).unwrap();

        with_cwd(project.path(), || {
            remove(&["nonexistent".to_string()], false).unwrap();
        });

        assert_eq!(
            fs::read_to_string(project.path().join("spell-whitelist.txt")).unwrap(),
            original
        );
    }

    #[test]
    fn add_and_remove_target_global_file_under_home() {
        let home = TempDir::new().unwrap();
        with_home(home.path(), || {
            add(&["docker".to_string()], true).unwrap();
        });

        let global = home.path().join(".texforge").join("spell-words");
        assert_eq!(fs::read_to_string(&global).unwrap(), "docker\n");

        with_home(home.path(), || {
            remove(&["docker".to_string()], true).unwrap();
        });
        assert_eq!(fs::read_to_string(&global).unwrap(), "");
    }

    #[test]
    fn list_output_names_the_file_it_read() {
        let project = TempDir::new().unwrap();
        write_project_toml(project.path());
        fs::write(project.path().join("spell-whitelist.txt"), "docker\nacme\n").unwrap();

        // `list` only prints; assert indirectly via the underlying data it
        // would report, since stdout isn't captured here.
        with_cwd(project.path(), || {
            list(false).unwrap();
        });

        let words = parse_whitelist_words(
            &fs::read_to_string(project.path().join("spell-whitelist.txt")).unwrap(),
        );
        assert!(words.contains("docker"));
        assert!(words.contains("acme"));
    }

    /// End-to-end: a word added globally is then accepted by the linter in a
    /// project that has no whitelist file of its own (requirement 6).
    #[test]
    fn word_added_globally_is_accepted_by_the_linter_in_a_bare_project() {
        let home = TempDir::new().unwrap();
        fs::create_dir_all(home.path().join(".texforge").join("dicts")).unwrap();
        fs::write(
            home.path()
                .join(".texforge")
                .join("dicts")
                .join("english.txt"),
            "hello\nworld\n",
        )
        .unwrap();

        with_home(home.path(), || {
            add(&["docker".to_string()], true).unwrap();
        });

        let src = "\\begin{document}\nHello docker world\n\\end{document}";
        let files = vec![("main.tex".to_string(), src.to_string())];
        let project_root = TempDir::new().unwrap();

        let findings = with_home(home.path(), || {
            linter_spell::lint_files(&files, project_root.path(), Some("english"))
        })
        .unwrap();

        assert!(
            findings.is_empty(),
            "word added via `texforge spell add --global` must be accepted: {:?}",
            findings
        );
    }
}
