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
    /// Dotted section number (`1`, `1.1`, `2.1.1`); empty for a starred
    /// section, which LaTeX leaves unnumbered.
    pub number: String,
    /// Resolved section title: wrapping macros (`\textit`, `\href`,
    /// `\textcolor`, ...) are stripped down to their text.
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
    tracker.set_top_level(texparse::appendix_top_level(
        tokenized
            .values()
            .flat_map(|tokens| tokens.iter())
            .map(|s| &s.token),
    ));
    let class = tokenized
        .get(&entry_path)
        .and_then(|tokens| texparse::document_class(tokens.iter().map(|s| &s.token)));
    tracker.set_secnumdepth(texparse::default_secnumdepth(class.as_deref()));
    let mut state = WalkState {
        tracker,
        sections: Vec::new(),
        stack: Vec::new(),
        in_document: false,
    };
    walk(&entry_path, root, &tokenized, &sources, &mut state);

    DocumentOutline {
        document: document.to_string(),
        sections: state.sections,
    }
}

/// The outline number for a section: starred sections are unnumbered in
/// LaTeX, so they keep an empty number and consume none.
fn section_number(tracker: &mut SectionTracker, level: u8, starred: bool) -> String {
    if starred {
        String::new()
    } else {
        tracker.enter(level)
    }
}

/// Mutable numbering and output state threaded through the `\input` walk.
struct WalkState {
    tracker: SectionTracker,
    sections: Vec<OutlineSection>,
    stack: Vec<PathBuf>,
    in_document: bool,
}

/// Walk one file's token stream, recursing into `\input` targets in place.
fn walk(
    file: &Path,
    root: &Path,
    tokenized: &HashMap<PathBuf, Vec<SpannedToken>>,
    sources: &HashMap<PathBuf, String>,
    state: &mut WalkState,
) {
    if state.stack.iter().any(|path| path == file) {
        return;
    }
    let Some(tokens) = tokenized.get(file) else {
        return;
    };
    state.stack.push(file.to_path_buf());
    let source = &sources[file];

    for spanned in tokens {
        match &spanned.token {
            Token::Section {
                level,
                starred,
                title,
                ..
            } => {
                let number = section_number(&mut state.tracker, *level, *starred);
                if *level <= MAX_LEVEL {
                    state.sections.push(OutlineSection {
                        level: *level,
                        number,
                        title: title.clone(),
                        file: relative_file(file, root),
                        line: line_at(source, spanned.start),
                    });
                }
            }
            Token::BeginDocument => state.in_document = true,
            Token::Command { name, args } if name == "input" => {
                if let Some(input) = args.last() {
                    let child = texutil::resolve_tex_path(root, input);
                    if tokenized.contains_key(&child) {
                        walk(&child, root, tokenized, sources, state);
                    }
                }
            }
            Token::Command { name, .. } if name == "appendix" && state.in_document => {
                state.tracker.start_appendix();
            }
            Token::Command { name, args } if name == "setcounter" => {
                texparse::apply_secnumdepth(&mut state.tracker, args);
            }
            _ => {}
        }
    }

    state.stack.pop();
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

/// Human-readable tree lines: the document name, then one line per section,
/// indented by level and carrying its number followed by two spaces and the
/// title. A starred section's number is empty, so its title starts where the
/// number column ends and no number precedes it.
fn human_lines(outline: &DocumentOutline) -> Vec<String> {
    let mut lines = vec![outline.document.clone()];
    for section in &outline.sections {
        let indent = "  ".repeat(section.level as usize);
        lines.push(format!("{indent}{}  {}", section.number, section.title));
    }
    lines
}

/// Human-readable tree, indented by level.
fn print_human(outline: &DocumentOutline) {
    for line in human_lines(outline) {
        println!("{line}");
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
        // `book` numbers down to `secnumdepth` 2: the subsubsection keeps its
        // row (it is within `MAX_LEVEL`) but prints no number.
        assert_eq!(
            numbers(&outline),
            vec!["1", "1.1", "1.1.1", "1.1.1.1", "", "1.1.2"]
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
    fn macro_wrapped_titles_resolve_to_plain_text() {
        let dir = write(&[(
            "main.tex",
            "\\begin{document}\n\\section{\\href{https://univerlab.org}{UniverLab.org}}\n\\subsection{AI Engineer en Accenture\n\\textcolor{lightgray}{\\leaders\\hbox{.}\\hfill}\n\\textit{Julio 2026 -- Actual}}\n\\end{document}",
        )]);
        let outline = build_outline("Doc", dir.path(), "main.tex");
        assert_eq!(
            titles(&outline),
            vec![
                "UniverLab.org",
                "AI Engineer en Accenture Julio 2026 -- Actual"
            ]
        );
        for title in titles(&outline) {
            assert!(!title.contains('\\'), "title leaks markup: {title:?}");
            assert!(
                !title.contains('\n'),
                "title spans multiple lines: {title:?}"
            );
        }
    }

    #[test]
    fn evidence_escaped_ampersand_survives_in_outline() {
        let dir = write(&[(
            "main.tex",
            "\\begin{document}\n\\section{Team}\n\\subsection*{\\textit{Fundador \\& Lead Engineer}}\n\\end{document}",
        )]);
        let outline = build_outline("Doc", dir.path(), "main.tex");
        assert_eq!(titles(&outline), vec!["Team", "Fundador & Lead Engineer"]);

        let value = serde_json::to_value(&outline).unwrap();
        let json_title = value["sections"][1]["title"].as_str().unwrap();
        assert_eq!(json_title, "Fundador & Lead Engineer");
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
        let _cwd = crate::test_sync::CWD_LOCK.lock().unwrap();
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
        let _cwd = crate::test_sync::CWD_LOCK.lock().unwrap();
        let orig = std::env::current_dir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();
        let result = execute(false);
        std::env::set_current_dir(&orig).unwrap();
        assert!(result.is_err());
    }

    #[test]
    fn starred_sections_are_unnumbered() {
        let dir = write(&[(
            "main.tex",
            "\\begin{document}\n\\chapter*{\\abstractname}\n\\chapter{Intro}\n\\section{A}\n\\end{document}",
        )]);
        let outline = build_outline("Doc", dir.path(), "main.tex");
        assert_eq!(numbers(&outline), vec!["", "1", "1.1"]);
        assert_eq!(titles(&outline), vec!["Abstract", "Intro", "A"]);

        let json = serde_json::to_value(&outline).unwrap();
        assert_eq!(json["sections"][0]["number"], "", "starred number is empty");
        assert_eq!(json["sections"][0]["title"], "Abstract");
        assert_eq!(json["sections"][1]["number"], "1");
    }

    /// The human tree prints the starred title where the number column ends:
    /// no leading number, the same two spaces every other entry gets after its
    /// number, and the usual level indentation.
    #[test]
    fn human_output_prints_starred_sections_without_a_number() {
        let dir = write(&[(
            "main.tex",
            "\\begin{document}\n\\chapter*{\\abstractname}\n\\chapter{Intro}\n\\end{document}",
        )]);
        let outline = build_outline("Doc", dir.path(), "main.tex");
        assert_eq!(
            human_lines(&outline),
            vec!["Doc", "    Abstract", "  1  Intro"]
        );
    }

    #[test]
    fn starred_section_between_numbered_ones_consumes_no_number() {
        let dir = write(&[(
            "main.tex",
            "\\begin{document}\n\\section{A}\n\\subsection{A1}\n\\section*{Notes}\n\\section{B}\n\\subsection{B1}\n\\end{document}",
        )]);
        let outline = build_outline("Doc", dir.path(), "main.tex");
        assert_eq!(numbers(&outline), vec!["1", "1.1", "", "2", "2.1"]);
        assert_eq!(titles(&outline), vec!["A", "A1", "Notes", "B", "B1"]);
    }

    /// A starred section at one level leaves the counters at that level and
    /// below untouched, so the next numbered sibling continues the sequence
    /// instead of starting after the starred one.
    #[test]
    fn starred_section_does_not_reset_deeper_counters() {
        let dir = write(&[(
            "main.tex",
            "\\begin{document}\n\\section{A}\n\\subsection{A1}\n\\subsection*{Aside}\n\\subsection{A2}\n\\section{B}\n\\end{document}",
        )]);
        let outline = build_outline("Doc", dir.path(), "main.tex");
        assert_eq!(numbers(&outline), vec!["1", "1.1", "", "1.2", "2"]);
    }

    #[test]
    fn appendix_chapters_take_letters_and_deep_headings_stay_empty() {
        let dir = write(&[(
            "main.tex",
            "\\documentclass{report}\n\\begin{document}\n\\chapter*{Resumen}\n\\chapter{Uno}\n\\section{S}\n\\paragraph{P}\n\\appendix\n\\chapter{Anexo}\n\\section{T}\n\\end{document}",
        )]);
        let outline = build_outline("Doc", dir.path(), "main.tex");
        // The paragraph sits below `MAX_LEVEL`, so the outline skips its row.
        assert_eq!(numbers(&outline), vec!["", "1", "1.1", "A", "A.1"]);
    }

    #[test]
    fn article_appendix_letters_sections() {
        let dir = write(&[(
            "main.tex",
            "\\documentclass{article}\n\\begin{document}\n\\section{Uno}\n\\subsection{S}\n\\subsubsection{SS}\n\\paragraph{P}\n\\appendix\n\\section{Anexo}\n\\subsection{T}\n\\end{document}",
        )]);
        let outline = build_outline("Doc", dir.path(), "main.tex");
        assert_eq!(numbers(&outline), vec!["1", "1.1", "1.1.1", "A", "A.1"]);
    }

    #[test]
    fn setcounter_from_an_input_file_narrows_the_depth() {
        let dir = write(&[
            (
                "main.tex",
                "\\begin{document}\n\\section{A}\n\\input{part}\n\\subsection{B}\n\\end{document}",
            ),
            ("part.tex", "\\setcounter{secnumdepth}{1}\n\\section{C}\n"),
        ]);
        let outline = build_outline("Doc", dir.path(), "main.tex");
        assert_eq!(numbers(&outline), vec!["1", "2", ""]);
    }

    #[test]
    fn appendix_without_chapter_or_section_changes_nothing() {
        let dir = write(&[(
            "main.tex",
            "\\begin{document}\n\\subsection{A}\n\\appendix\n\\subsection{B}\n\\end{document}",
        )]);
        let outline = build_outline("Doc", dir.path(), "main.tex");
        assert_eq!(numbers(&outline), vec!["1", "2"]);
    }

    #[test]
    fn preamble_titlesec_adds_no_outline_sections() {
        let dir = write(&[(
            "main.tex",
            "\\titleformat{\\section}{}{}{}{}\n\\titleformat{\\subsection}{}{}{}{}\n\\titlespacing*{\\section}{0pt}{1ex}{1ex}\n\\begin{document}\n\\section{Introduction}\n\\subsection{Background}\n\\end{document}",
        )]);
        let outline = build_outline("Doc", dir.path(), "main.tex");
        assert_eq!(titles(&outline), vec!["Introduction", "Background"]);
        assert_eq!(numbers(&outline), vec!["1", "1.1"]);
    }
}
