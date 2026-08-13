//! `texforge check` command implementation.

use anyhow::Result;

use crate::domain::project::Project;
use crate::linter::{self, Severity};

/// Lint project without compiling.
///
/// Exits non-zero when any `Error`-severity finding is present, or when
/// `deny_warnings` is `true` and any finding (including `Warning`) is present.
pub fn execute(deny_warnings: bool) -> Result<()> {
    let project = Project::load()?;

    println!("Checking project: {}", project.config.document.title);

    let findings = linter::lint(
        &project.root,
        &project.config.build.entry,
        project.config.build.bibliography.as_deref(),
    )?;

    if findings.is_empty() {
        println!("  ◇ No issues found");
        return Ok(());
    }

    // Partition by severity; errors first.
    let errors: Vec<_> = findings
        .iter()
        .filter(|f| f.severity == Severity::Error)
        .collect();
    let warnings: Vec<_> = findings
        .iter()
        .filter(|f| f.severity == Severity::Warning)
        .collect();

    println!();

    if !errors.is_empty() {
        println!("ERRORS ({})", errors.len());
        for f in &errors {
            println!("  ERROR [{}:{}]", f.file, f.line);
            println!("    {}", f.message);
            if let Some(ref s) = f.suggestion {
                println!("    suggestion: {}", s);
            }
            println!();
        }
    }

    if !warnings.is_empty() {
        println!("WARNINGS ({})", warnings.len());
        for f in &warnings {
            println!("  WARNING [{}:{}]", f.file, f.line);
            println!("    {}", f.message);
            if let Some(ref s) = f.suggestion {
                println!("    suggestion: {}", s);
            }
            println!();
        }
    }

    let should_fail = !errors.is_empty() || (deny_warnings && !warnings.is_empty());

    if should_fail {
        let total = if deny_warnings {
            findings.len()
        } else {
            errors.len()
        };
        anyhow::bail!("{} issue(s) found", total);
    }

    Ok(())
}
