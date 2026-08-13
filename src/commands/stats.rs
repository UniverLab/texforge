//! `texforge stats` command implementation.

use anyhow::Result;
use clap::ValueEnum;

use crate::domain::project::Project;
use crate::texparse;
use crate::wordcount::{self, DocumentStats};

/// Breakdown mode for `texforge stats`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ByMode {
    /// Report per section.
    Section,
    /// Report per `.tex` file.
    File,
}

/// Count words and report document statistics.
pub fn execute(json: bool, by: ByMode) -> Result<()> {
    let project = Project::load()?;

    let entry = project.root.join(&project.config.build.entry);
    if !entry.exists() {
        anyhow::bail!("Entry point file does not exist: {}", entry.display());
    }

    let files = texparse::tokenize_document(&project.root, &project.config.build.entry);
    let stats = match by {
        ByMode::Section => wordcount::count_document(&project.config.document.title, &files),
        ByMode::File => wordcount::count_by_file(&project.config.document.title, &files),
    };

    if json {
        println!("{}", serde_json::to_string_pretty(&stats)?);
    } else {
        print_human(&stats, by);
    }

    Ok(())
}

/// Human-readable breakdown with dotted leaders and a total.
fn print_human(stats: &DocumentStats, by: ByMode) {
    println!("{}", stats.document);

    if stats.preamble_words > 0 {
        println!(
            "  {}",
            dotted_line("preamble", &stats.preamble_words.to_string())
        );
    }
    for section in &stats.sections {
        let left = match by {
            ByMode::Section => format!(
                "{} {}",
                section.path,
                section.title.as_deref().unwrap_or("")
            ),
            ByMode::File => section.path.clone(),
        };
        println!("  {}", dotted_line(&left, &section.words.to_string()));
    }
    if stats.preamble_words > 0 || !stats.sections.is_empty() {
        println!("  {}", dotted_line("Total", &stats.total_words.to_string()));
    }
}

/// Join `left` and `right` with a run of dots, padded to `WIDTH`.
fn dotted_line(left: &str, right: &str) -> String {
    const WIDTH: usize = 60;
    let dots = WIDTH.saturating_sub(left.chars().count() + right.chars().count());
    format!("{left}{dots}{right}", dots = ".".repeat(dots))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dotted_line_pads_with_dots() {
        let line = dotted_line("Total", "12");
        assert!(line.starts_with("Total"));
        assert!(line.ends_with("12"));
        assert_eq!(line.chars().count(), 60);
        assert!(line.contains("...."));
    }

    #[test]
    fn dotted_line_tolerates_overlong_left() {
        let line = dotted_line(&"x".repeat(70), "1");
        assert!(line.ends_with("1"));
    }

    #[test]
    fn execute_json_on_small_project() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("project.toml"),
            "[document]\ntitle = \"T\"\nauthor = \"A\"\ntemplate = \"general\"\n\n[build]\nentry = \"main.tex\"\n",
        )
        .unwrap();
        std::fs::write(
            tmp.path().join("main.tex"),
            "\\begin{document}\nhello world\n\\section{S}\ntext\n\\end{document}",
        )
        .unwrap();
        let orig = std::env::current_dir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();
        let result = execute(true, ByMode::Section);
        std::env::set_current_dir(&orig).unwrap();
        result.unwrap();
    }

    #[test]
    fn execute_missing_entry_errors() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("project.toml"),
            "[document]\ntitle = \"T\"\nauthor = \"A\"\ntemplate = \"general\"\n\n[build]\nentry = \"main.tex\"\n",
        )
        .unwrap();
        let orig = std::env::current_dir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();
        let result = execute(false, ByMode::Section);
        std::env::set_current_dir(&orig).unwrap();
        assert!(result.is_err());
    }
}
