//! `texforge outline` command implementation.
//!
//! Prints the document's section tree, or emits it as JSON for agents that
//! want the skeleton of a large document without reading the source.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::Serialize;

use crate::domain::project::Project;
use crate::texparse::{self, SectionTracker, SpannedToken, Token};
use crate::texutil;

/// Deepest level shown in the outline: `part` through `subsubsection`.
const MAX_LEVEL: u8 = 4;

/// One entry in the document outline.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct OutlineSection {
    /// Section level (`0` = part, ..., `4` = subsubsection).
    pub level: u8,
    /// Dotted section number (`1`, `1.1`, `2.1.1`).
    pub number: String,
    /// Raw section title.
    pub title: String,
    /// `.tex` file containing the section, relative to the project root.
    pub file: String,
    /// 1-based line of the section command within `file`.
    pub line: usize,
}

/// The document's section tree.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct DocumentOutline {
    /// Document name (the project title).
    pub document: String,
    /// Sections in `\input` traversal order.
    pub sections: Vec<OutlineSection>,
}

/// Build the outline for the document rooted at `root` with entry `entry`.
///
/// Reads source only — the document does not need to compile. Sections from
/// `\input`-ed files appear at the position their `\input` occupies, not
/// grouped by file.
pub fn build_outline(document: &str, root: &Path, entry: &str) -> DocumentOutline {
    let collection = texutil::collect_tex_files(root, entry);

    let mut sources: HashMap<PathBuf, String> = HashMap::new();
    let mut tokenized: HashMap<PathBuf, Vec<SpannedToken>> = HashMap::new();
    for path in &collection.files {
        if let Ok(source) = std::fs::read_to_string(path) {
            let tokens = texparse::tokenize_with_spans(&source).tokens;
            sources.insert(path.clone(), source);
            tokenized.insert(path.clone(), tokens);
        }
    }

    let entry_path = texutil::resolve_tex_path(root, entry);
    let mut tracker = SectionTracker::new(6);
    let mut sections = Vec::new();
    let mut stack = Vec::new();
    walk(
        &entry_path,
        root,
        &tokenized,
        &sources,
        &mut tracker,
        &mut sections,
        &mut stack,
    );

    DocumentOutline {
        document: document.to_string(),
        sections,
    }
}

/// Walk one file's token stream, recursing into `\input` targets in place.
fn walk(
    file: &Path,
    root: &Path,
    tokenized: &HashMap<PathBuf, Vec<SpannedToken>>,
    sources: &HashMap<PathBuf, String>,
    tracker: &mut SectionTracker,
    sections: &mut Vec<OutlineSection>,
    stack: &mut Vec<PathBuf>,
) {
    if stack.iter().any(|path| path == file) {
        return;
    }
    let Some(tokens) = tokenized.get(file) else {
        return;
    };
    stack.push(file.to_path_buf());
    let source = &sources[file];

    for spanned in tokens {
        match &spanned.token {
            Token::Section { level, title } => {
                let number = tracker.enter(*level);
                if *level <= MAX_LEVEL {
                    sections.push(OutlineSection {
                        level: *level,
                        number,
                        title: title.clone(),
                        file: relative_file(file, root),
                        line: line_at(source, spanned.start),
                    });
                }
            }
            Token::Command { name, args } if name == "input" => {
                if let Some(input) = args.last() {
                    let child = texutil::resolve_tex_path(root, input);
                    if tokenized.contains_key(&child) {
                        walk(&child, root, tokenized, sources, tracker, sections, stack);
                    }
                }
            }
            _ => {}
        }
    }

    stack.pop();
}

/// The section command's line number, computed from its byte offset.
fn line_at(source: &str, offset: usize) -> usize {
    source[..offset]
        .bytes()
        .filter(|byte| *byte == b'\n')
        .count()
        + 1
}

/// `path` relative to `root`, falling back to the file name outside `root`.
fn relative_file(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .map(|rel| rel.to_string_lossy().into_owned())
        .unwrap_or_else(|_| {
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default()
        })
}

/// Emit the outline for the current project.
pub fn execute(json: bool) -> Result<()> {
    let project = Project::load()?;

    let entry = project.root.join(&project.config.build.entry);
    if !entry.exists() {
        anyhow::bail!("Entry point file does not exist: {}", entry.display());
    }

    let outline = build_outline(
        &project.config.document.title,
        &project.root,
        &project.config.build.entry,
    );

    if json {
        println!("{}", serde_json::to_string_pretty(&outline)?);
    } else {
        print_human(&outline);
    }

    Ok(())
}

/// Human-readable tree, indented by level.
fn print_human(outline: &DocumentOutline) {
    println!("{}", outline.document);
    for section in &outline.sections {
        let indent = "  ".repeat(section.level as usize);
        println!("{}{}  {}", indent, section.number, section.title);
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    fn write(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (name, source) in files {
            let path = dir.path().join(name);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(path, source).unwrap();
        }
        dir
    }

    fn numbers(outline: &DocumentOutline) -> Vec<String> {
        outline
            .sections
            .iter()
            .map(|section| section.number.clone())
            .collect()
    }

    fn titles(outline: &DocumentOutline) -> Vec<String> {
        outline
            .sections
            .iter()
            .map(|section| section.title.clone())
            .collect()
    }

    #[test]
    fn numbering_follows_the_hierarchy() {
        let dir = write(&[(
            "main.tex",
            "\\documentclass{book}\n\\begin{document}\n\\part{One}\n\\chapter{Intro}\n\\section{First}\n\\subsection{Sub}\n\\subsubsection{Subsub}\n\\section{Second}\n\\end{document}",
        )]);
        let outline = build_outline("Doc", dir.path(), "main.tex");
        assert_eq!(
            numbers(&outline),
            vec!["1", "1.1", "1.1.1", "1.1.1.1", "1.1.1.1.1", "1.1.2"]
        );
    }

    #[test]
    fn sections_from_input_files_appear_at_their_input_position() {
        let dir = write(&[
            (
                "main.tex",
                "\\begin{document}\n\\part{Part}\n\\input{ch1}\n\\section{After Ch1}\n\\input{chapters/ch2}\n\\section{After Ch2}\n\\end{document}",
            ),
            ("ch1.tex", "\\chapter{Chapter One}\n\\input{ch1_sec}\n"),
            ("ch1_sec.tex", "\\section{Ch1 Section}\n"),
            (
                "chapters/ch2.tex",
                "\\chapter{Chapter Two}\n\\section{Ch2 Section}\n",
            ),
        ]);
        let outline = build_outline("Doc", dir.path(), "main.tex");
        assert_eq!(
            numbers(&outline),
            vec!["1", "1.1", "1.1.1", "1.1.2", "1.2", "1.2.1", "1.2.2"]
        );
        assert_eq!(
            titles(&outline),
            vec![
                "Part",
                "Chapter One",
                "Ch1 Section",
                "After Ch1",
                "Chapter Two",
                "Ch2 Section",
                "After Ch2"
            ]
        );
    }

    #[test]
    fn paragraphs_are_not_included() {
        let dir = write(&[(
            "main.tex",
            "\\begin{document}\n\\section{One}\n\\paragraph{Note}\n\\subparagraph{Detail}\n\\section{Two}\n\\end{document}",
        )]);
        let outline = build_outline("Doc", dir.path(), "main.tex");
        assert_eq!(numbers(&outline), vec!["1", "2"]);
        assert!(outline.sections.iter().all(|section| section.level <= 4));
    }

    #[test]
    fn reports_source_file_and_line() {
        let dir = write(&[(
            "main.tex",
            "\\begin{document}\n\n\\section{Intro}\n\\end{document}",
        )]);
        let outline = build_outline("Doc", dir.path(), "main.tex");
        let section = &outline.sections[0];
        assert_eq!(section.file, "main.tex");
        assert_eq!(section.line, 3);
    }

    #[test]
    fn file_paths_are_relative_to_root() {
        let dir = write(&[("chapters/intro.tex", "\\section{Intro}\n")]);
        let outline = build_outline("Doc", dir.path(), "chapters/intro.tex");
        assert_eq!(outline.sections[0].file, "chapters/intro.tex");
    }

    #[test]
    fn circular_inputs_do_not_duplicate_sections() {
        let dir = write(&[
            (
                "main.tex",
                "\\begin{document}\n\\section{A}\n\\input{loop}\n\\end{document}",
            ),
            ("loop.tex", "\\input{main}\n\\section{B}\n"),
        ]);
        let outline = build_outline("Doc", dir.path(), "main.tex");
        assert_eq!(titles(&outline), vec!["A", "B"]);
    }

    #[test]
    fn json_contract_keys_are_stable() {
        let dir = write(&[(
            "main.tex",
            "\\begin{document}\n\\section{S}\n\\end{document}",
        )]);
        let outline = build_outline("Doc", dir.path(), "main.tex");
        let value = serde_json::to_value(&outline).unwrap();
        let object = value.as_object().unwrap();
        let mut top_keys: Vec<&str> = object.keys().map(String::as_str).collect();
        top_keys.sort_unstable();
        assert_eq!(top_keys, vec!["document", "sections"]);
        let section = object["sections"][0].as_object().unwrap();
        let mut section_keys: Vec<&str> = section.keys().map(String::as_str).collect();
        section_keys.sort_unstable();
        assert_eq!(
            section_keys,
            vec!["file", "level", "line", "number", "title"]
        );
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
            "\\begin{document}\n\\section{Intro}\ntext\n\\end{document}",
        )
        .unwrap();
        let orig = std::env::current_dir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();
        let result = execute(true);
        std::env::set_current_dir(&orig).unwrap();
        result.unwrap();
    }

    #[test]
    fn execute_missing_entry_errors() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("project.toml"),
            "[document]\ntitle = \"T\"\nauthor = \"A\"\ntemplate = \"general\"\n\n[build]\nentry = \"missing.tex\"\n",
        )
        .unwrap();
        let orig = std::env::current_dir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();
        let result = execute(false);
        std::env::set_current_dir(&orig).unwrap();
        assert!(result.is_err());
    }
}
