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
