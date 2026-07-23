//! `texforge fmt` command implementation.

use std::path::Path;

use anyhow::Result;

use crate::domain::project::Project;
use crate::formatter;
use crate::utils;

/// Format `.tex` and `.bib` files.
pub fn execute(check: bool) -> Result<()> {
    let project = Project::load()?;

    let tex_files = utils::find_tex_files(&project.root)?;
    let bib_files = utils::find_bib_files(&project.root)?;
    let total = tex_files.len() + bib_files.len();

    if total == 0 {
        println!("No .tex or .bib files found");
        return Ok(());
    }

    let mut unformatted = 0;

    for file in &tex_files {
        unformatted += format_one(file, &project.root, check, formatter::format)?;
    }
    for file in &bib_files {
        unformatted += format_one(file, &project.root, check, formatter::format_bib)?;
    }

    if check && unformatted > 0 {
        anyhow::bail!(
            "{} file(s) need formatting — run 'texforge fmt'",
            unformatted
        );
    } else if check {
        println!("  ◇ All files formatted correctly");
    } else {
        println!("  ◇ {} file(s) checked", total);
    }

    Ok(())
}

/// Format a single file, returning 1 if it needed formatting (else 0).
fn format_one(file: &Path, root: &Path, check: bool, fmt: fn(&str) -> String) -> Result<usize> {
    let content = std::fs::read_to_string(file)?;
    let formatted = fmt(&content);

    if content == formatted {
        return Ok(0);
    }

    let rel = file.strip_prefix(root).unwrap_or(file).display();
    if check {
        println!("  ✗ {}", rel);
    } else {
        std::fs::write(file, &formatted)?;
        println!("  formatted {}", rel);
    }
    Ok(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn format_one_no_change_returns_zero() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let file = root.join("test.tex");
        fs::write(&file, "hello\n").unwrap();
        let count = format_one(&file, root, false, |s| s.to_string()).unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn format_one_needs_formatting_returns_one() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let file = root.join("test.tex");
        fs::write(&file, "hello  \n").unwrap();
        let count = format_one(&file, root, false, |s| format!("{}\n", s.trim_end())).unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn format_one_check_mode_does_not_write() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let file = root.join("test.tex");
        fs::write(&file, "hello  \n").unwrap();
        let count = format_one(&file, root, true, |s| format!("{}\n", s.trim_end())).unwrap();
        assert_eq!(count, 1);
        // File should be unchanged
        assert_eq!(fs::read_to_string(&file).unwrap(), "hello  \n");
    }

    #[test]
    fn format_one_check_mode_writes_when_not_checking() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let file = root.join("test.tex");
        fs::write(&file, "hello  \n").unwrap();
        let count = format_one(&file, root, false, |s| format!("{}\n", s.trim_end())).unwrap();
        assert_eq!(count, 1);
        assert_eq!(fs::read_to_string(&file).unwrap(), "hello\n");
    }

    #[test]
    fn execute_no_files_returns_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        // Write a project.toml so Project::load() works
        fs::write(
            root.join("project.toml"),
            "[document]\ntitle = \"T\"\nauthor = \"A\"\ntemplate = \"general\"\n\n[build]\nentry = \"main.tex\"\n",
        )
        .unwrap();
        let orig = std::env::current_dir().unwrap();
        std::env::set_current_dir(root).unwrap();
        let result = execute(false);
        std::env::set_current_dir(&orig).unwrap();
        result.unwrap();
    }
}
