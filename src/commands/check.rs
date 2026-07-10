//! `texforge check` command implementation.

use anyhow::Result;

use crate::domain::project::Project;
use crate::linter;

/// Lint project without compiling.
pub fn execute() -> Result<()> {
    let project = Project::load()?;

    println!("Checking project: {}", project.config.document.title);

    let errors = linter::lint(
        &project.root,
        &project.config.build.entry,
        project.config.build.bibliography.as_deref(),
    )?;

    if errors.is_empty() {
        println!("  ◇ No issues found");
    } else {
        println!();
        for e in &errors {
            println!("ERROR [{}:{}]", e.file, e.line);
            println!("  {}", e.message);
            if let Some(ref s) = e.suggestion {
                println!("  suggestion: {}", s);
            }
            println!();
        }
        anyhow::bail!("{} issue(s) found", errors.len());
    }

    Ok(())
}
