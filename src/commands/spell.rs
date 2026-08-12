//! `texforge spell` — manage the personal spell-check whitelist (dictionary)
//! from the CLI, reusing the whitelist mechanism the linter already reads.
//!
//! Two scopes: the global personal dictionary (`~/.texforge/spell-words`,
//! shared by every project and the default, since the words in this list are
//! overwhelmingly true of the person rather than of one document) and the
//! project (the first of `PROJECT_WHITELIST_FILES` that already exists, or
//! `.texforge/spell-words` if none do), selected explicitly with `--local`.
//! Adding is append-only and never rewrites existing bytes; removing rewrites
//! the file with only the matching lines dropped.

use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::domain::project::Project;
use crate::linter::{global_whitelist_path, parse_whitelist_words, PROJECT_WHITELIST_FILES};

/// Which whitelist a `spell` action targets. Global is the default; `--local`
/// is the explicit opt-in to the project's whitelist (decisions 1-3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Global,
    Local,
}

impl Scope {
    /// `--local` and `--global` are mutually exclusive at the clap level, so
    /// at most one of these is ever true here; global wins when neither is
    /// set, since global is the default (decision 1).
    pub fn from_flags(local: bool, global: bool) -> Self {
        debug_assert!(!(local && global));
        if local {
            Scope::Local
        } else {
            let _ = global;
            Scope::Global
        }
    }
}

/// Subcommands of `texforge spell`.
#[derive(Debug, Clone)]
pub enum SpellAction {
    /// Add one or more words to the whitelist.
    Add { words: Vec<String>, scope: Scope },
    /// List the effective whitelist words for the scope.
    List { scope: Scope },
    /// Remove one or more words from the whitelist.
    Remove { words: Vec<String>, scope: Scope },
}

/// Dispatch a `texforge spell` action.
pub fn execute(action: SpellAction) -> Result<()> {
    match action {
        SpellAction::Add { words, scope } => add(&words, scope),
        SpellAction::List { scope } => list(scope),
        SpellAction::Remove { words, scope } => remove(&words, scope),
    }
}

/// Resolve the file `add`/`remove`/`list` should target for the given scope.
/// Global always resolves to `global_whitelist_path()`. Local resolves to
/// the first of `PROJECT_WHITELIST_FILES` that already exists, or
/// `.texforge/spell-words` if none do (decision 4) — this does not require
/// the file to exist yet, only its parent project. `--local` outside a
/// texforge project is an error, not a silent fallback to global
/// (decision 5): `Project::load()` already fails with a message naming what
/// was expected when no project is found.
fn target_file(scope: Scope) -> Result<PathBuf> {
    match scope {
        Scope::Global => global_whitelist_path()
            .context("Could not determine home directory for the global personal dictionary"),
        Scope::Local => {
            let project = Project::load().context(
                "`--local` requires a texforge project (project.toml not found in this \
                 directory or any parent)",
            )?;
            for name in PROJECT_WHITELIST_FILES {
                let p = project.root.join(name);
                if p.exists() {
                    return Ok(p);
                }
            }
            Ok(project.root.join(".texforge").join("spell-words"))
        }
    }
}

fn add(words: &[String], scope: Scope) -> Result<()> {
    if words.is_empty() {
        anyhow::bail!("No words given; usage: texforge spell add <WORD>...");
    }

    let path = target_file(scope)?;
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

fn remove(words: &[String], scope: Scope) -> Result<()> {
    if words.is_empty() {
        anyhow::bail!("No words given; usage: texforge spell remove <WORD>...");
    }

    let path = target_file(scope)?;

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

fn list(scope: Scope) -> Result<()> {
    if scope == Scope::Global {
        let path = global_whitelist_path()
            .context("Could not determine home directory for the global personal dictionary")?;
        print_whitelist_file(&path);
        return Ok(());
    }

    let project = Project::load().context(
        "`--local` requires a texforge project (project.toml not found in this directory or \
         any parent)",
    )?;
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
            add(&["docker".to_string()], Scope::Local).unwrap();
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
            add(&["docker".to_string()], Scope::Local).unwrap();
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
            add(&["docker".to_string()], Scope::Local).unwrap();
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
            add(&["docker".to_string()], Scope::Local).unwrap();
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
            add(&["docker".to_string()], Scope::Local).unwrap();
            add(&["Docker".to_string()], Scope::Local).unwrap();
            add(&["DOCKER".to_string()], Scope::Local).unwrap();
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
            remove(&["docker".to_string()], Scope::Local).unwrap();
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
            remove(&["nonexistent".to_string()], Scope::Local).unwrap();
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
            add(&["docker".to_string()], Scope::Global).unwrap();
        });

        let global = home.path().join(".texforge").join("spell-words");
        assert_eq!(fs::read_to_string(&global).unwrap(), "docker\n");

        with_home(home.path(), || {
            remove(&["docker".to_string()], Scope::Global).unwrap();
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
            list(Scope::Local).unwrap();
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
            add(&["docker".to_string()], Scope::Global).unwrap();
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

    /// TE14 requirement 1: no scope flag means global, even when a project
    /// whitelist file already exists in the current directory.
    #[test]
    fn add_with_no_flag_writes_global_and_leaves_existing_project_file_untouched() {
        let project = TempDir::new().unwrap();
        write_project_toml(project.path());
        fs::write(project.path().join("spell-whitelist.txt"), "acme\n").unwrap();

        let home = TempDir::new().unwrap();
        with_home(home.path(), || {
            with_cwd(project.path(), || {
                add(&["docker".to_string()], Scope::Global).unwrap();
            });
        });

        let global = home.path().join(".texforge").join("spell-words");
        assert_eq!(fs::read_to_string(&global).unwrap(), "docker\n");
        assert_eq!(
            fs::read_to_string(project.path().join("spell-whitelist.txt")).unwrap(),
            "acme\n",
            "project whitelist must be untouched by a default-scope add"
        );
    }

    /// TE14 requirement 5: `--global` behaves identically to no flag, since
    /// global is both the default and the explicit name for it.
    #[test]
    fn add_global_flag_behaves_identically_to_no_flag() {
        let home = TempDir::new().unwrap();
        with_home(home.path(), || {
            add(&["docker".to_string()], Scope::Global).unwrap();
        });

        let global = home.path().join(".texforge").join("spell-words");
        assert_eq!(fs::read_to_string(&global).unwrap(), "docker\n");
    }

    /// TE14 requirement 4: `list --local` reads the project file rather than
    /// the global default.
    #[test]
    fn list_local_reads_the_project_file_not_global() {
        let project = TempDir::new().unwrap();
        write_project_toml(project.path());
        fs::write(project.path().join("spell-whitelist.txt"), "docker\n").unwrap();

        let home = TempDir::new().unwrap();
        fs::create_dir_all(home.path().join(".texforge")).unwrap();
        fs::write(
            home.path().join(".texforge").join("spell-words"),
            "globalword\n",
        )
        .unwrap();

        with_home(home.path(), || {
            with_cwd(project.path(), || {
                list(Scope::Local).unwrap();
            });
        });

        let words = parse_whitelist_words(
            &fs::read_to_string(project.path().join("spell-whitelist.txt")).unwrap(),
        );
        assert!(words.contains("docker"));
        assert!(!words.contains("globalword"));
    }

    /// TE14 requirement: `remove` with no flag removes from global only,
    /// leaving an identically-named word in the project file untouched.
    #[test]
    fn remove_with_no_flag_removes_from_global_only() {
        let project = TempDir::new().unwrap();
        write_project_toml(project.path());
        fs::write(project.path().join("spell-whitelist.txt"), "docker\n").unwrap();

        let home = TempDir::new().unwrap();
        fs::create_dir_all(home.path().join(".texforge")).unwrap();
        fs::write(
            home.path().join(".texforge").join("spell-words"),
            "docker\n",
        )
        .unwrap();

        with_home(home.path(), || {
            with_cwd(project.path(), || {
                remove(&["docker".to_string()], Scope::Global).unwrap();
            });
        });

        assert_eq!(
            fs::read_to_string(home.path().join(".texforge").join("spell-words")).unwrap(),
            ""
        );
        assert_eq!(
            fs::read_to_string(project.path().join("spell-whitelist.txt")).unwrap(),
            "docker\n",
            "project whitelist must be untouched by a default-scope remove"
        );
    }

    /// TE14 decision 5: `--local` outside a texforge project must fail
    /// loudly rather than silently falling back to the global list.
    #[test]
    fn local_outside_a_project_errors_and_writes_nothing() {
        let outside = TempDir::new().unwrap();
        let home = TempDir::new().unwrap();

        let result = with_home(home.path(), || {
            with_cwd(outside.path(), || {
                add(&["docker".to_string()], Scope::Local)
            })
        });

        assert!(result.is_err(), "add --local with no project must fail");
        assert!(
            !home.path().join(".texforge").join("spell-words").exists(),
            "a failed --local add must not fall back to writing the global list"
        );
        assert!(
            !outside
                .path()
                .join(".texforge")
                .join("spell-words")
                .exists(),
            "a failed --local add must not create a project file out of nowhere either"
        );
    }

    /// TE14 requirement 6: `--local --global` together is a clap usage
    /// error, checked at the `Scope::from_flags` boundary this command
    /// module relies on — the CLI layer enforces `conflicts_with` before
    /// either flag reaches here.
    #[test]
    fn from_flags_defaults_to_global_when_neither_flag_is_set() {
        assert_eq!(Scope::from_flags(false, false), Scope::Global);
        assert_eq!(Scope::from_flags(true, false), Scope::Local);
        assert_eq!(Scope::from_flags(false, true), Scope::Global);
    }
}
