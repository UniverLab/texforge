//! `texforge skill` command implementation.
//!
//! The agent skill travels inside the binary: `SKILL.md` and every file under
//! `skill/references/` are embedded at build time, so `texforge skill` always
//! prints the skill that matches the installed version. stdout carries the
//! skill text and nothing else; errors surface through stderr.

use anyhow::Result;

const SKILL: &str = include_str!("../../skill/SKILL.md");

/// Reference files, sorted by name: `(file name, contents)`.
///
/// One `include_str!` per file: adding a file without an entry here leaves it
/// out of `--references`, and removing a listed file fails the build instead
/// of the run.
const REFERENCES: &[(&str, &str)] = &[(
    "cli-cheatsheet.md",
    include_str!("../../skill/references/cli-cheatsheet.md"),
)];

/// Print the embedded skill, its reference files, or just the reference names.
pub fn execute(list: bool, references: bool) -> Result<()> {
    if list {
        print_reference_names();
    } else if references {
        print_skill_with_references();
    } else {
        print!("{}", rendered_skill());
    }
    Ok(())
}

/// The embedded skill with the version line inserted right after the
/// frontmatter, so the text still begins with the YAML block the skill format
/// requires while naming the binary that printed it.
fn rendered_skill() -> String {
    match frontmatter_end(SKILL) {
        Some(end) => format!(
            "{}\nThis skill was printed by texforge v{}\n{}",
            &SKILL[..end],
            env!("CARGO_PKG_VERSION"),
            &SKILL[end..],
        ),
        None => SKILL.to_string(),
    }
}

/// Byte offset just past the newline that closes the opening frontmatter, or
/// `None` when the text does not start with a `---` block.
fn frontmatter_end(text: &str) -> Option<usize> {
    let mut offset = 0;
    let mut opened = false;
    for line in text.split_inclusive('\n') {
        offset += line.len();
        if line.trim_end() != "---" {
            if !opened {
                return None;
            }
            continue;
        }
        if opened {
            return Some(offset);
        }
        opened = true;
    }
    None
}

fn print_reference_names() {
    for (name, _) in REFERENCES {
        println!("{name}");
    }
}

fn print_skill_with_references() {
    print!("{}", rendered_skill());
    for (name, body) in REFERENCES {
        print!("\n=== references/{name} ===\n{body}");
        if !body.ends_with('\n') {
            println!();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frontmatter_end_finds_the_closing_fence() {
        let text = "---\nname: x\n---\n\n# body\n";
        let end = frontmatter_end(text).unwrap();
        assert_eq!(&text[..end], "---\nname: x\n---\n");
    }

    #[test]
    fn frontmatter_end_rejects_text_without_a_fence() {
        assert_eq!(frontmatter_end("# body\n"), None);
        assert_eq!(frontmatter_end("---\nname: x\n"), None);
    }

    #[test]
    fn rendered_skill_keeps_the_frontmatter_first_and_adds_the_version() {
        let rendered = rendered_skill();
        assert!(rendered.starts_with("---\nname: texforge\n"));
        let version = format!(
            "This skill was printed by texforge v{}",
            env!("CARGO_PKG_VERSION")
        );
        let version_at = rendered
            .find(version.as_str())
            .expect("version line is present");
        let closing = rendered[4..].find("\n---\n").expect("closing fence") + 4;
        let heading = rendered.find("\n# texforge CLI").expect("body heading");
        assert!(
            closing < version_at,
            "version line must follow the frontmatter"
        );
        assert!(version_at < heading, "version line must precede the body");
    }

    #[test]
    fn references_are_sorted_unique_and_non_empty() {
        assert!(!REFERENCES.is_empty(), "ship at least one reference file");
        let names: Vec<&str> = REFERENCES.iter().map(|(name, _)| *name).collect();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(names, sorted, "REFERENCES must be sorted and unique");
    }
}
