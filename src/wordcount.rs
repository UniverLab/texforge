//! Document word counts built on the [`crate::texparse`] tokenizer.
//!
//! This is the first production consumer of the tokenizer. It turns a flat
//! [`Token`] stream into per-section and per-file word counts in a single pass.
//!
//! # Word definition
//!
//! A word is a maximal run of non-space characters containing at least one
//! alphabetic character ([`char::is_alphabetic`]). `Introduccion` counts,
//! `50` does not, and `fig:cap` does (a mild false positive, accepted for v1).
//!
//! # What is counted
//!
//! The tokenizer has already stripped math, verbatim regions and comments from
//! the stream and demoted non-prose command arguments to [`Token::Command`], so
//! the counter only ever sees prose: body text, section titles, prose-command
//! arguments (`\textit`, `\textbf`, `\emph`, `\footnote`, `\caption`, and the
//! link text of `\href`), and text inside environments such as `figure`,
//! `table`, `tabular` and `thebibliography`.
//!
//! The preamble (everything before the first [`Token::BeginDocument`]) is not
//! counted. Text between `\begin{document}` and the first section is reported
//! as [`DocumentStats::preamble_words`] and attributed to no section.
//!
//! # Limitations
//!
//! * A document's files are consumed in `\input` traversal order as one logical
//!   stream, so `\begin{document}` is a one-way switch: text in a file whose
//!   `\input` appears in the preamble is still counted. A mild inflation, in
//!   the same accepted family as `\tableofcontents` title duplication.
//! * Words in a section title are counted after re-tokenizing the title, so
//!   math inside a title is excluded.
//! * `--by file` mode reports one entry per `.tex` file, whether or not it
//!   contains counted text.

use std::path::Path;

use serde::Serialize;

use crate::texparse::{tokenize, Token, TokenizedFile};

/// One counted section, or one `.tex` file in `--by file` mode.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SectionStat {
    /// Dotted section number (`1`, `1.1`, `2.1.1`) in section mode; the
    /// `.tex` file name in `--by file` mode.
    pub path: String,
    /// Section level (`0` = part, ..., `6` = subparagraph) in section mode.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub level: Option<u8>,
    /// Raw section title, kept out of the JSON contract.
    #[serde(skip)]
    pub title: Option<String>,
    /// Word count for this section or file.
    pub words: usize,
}

/// Complete statistics for a document.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct DocumentStats {
    /// Document name (the project title).
    pub document: String,
    /// Total counted words across all files.
    pub total_words: usize,
    /// Number of `.tex` files scanned.
    pub file_count: usize,
    /// Words between `\begin{document}` and the first section.
    pub preamble_words: usize,
    /// Per-section or per-file breakdown.
    pub sections: Vec<SectionStat>,
}

/// Count the words in a text run.
///
/// A word is a maximal run of non-space characters containing at least one
/// alphabetic character.
pub fn count_words(text: &str) -> usize {
    text.split_whitespace()
        .filter(|word| word.chars().any(char::is_alphabetic))
        .count()
}

/// Count the words of a section title after re-tokenizing it, so math and
/// commands inside the title do not count.
fn count_title_words(title: &str) -> usize {
    tokenize(title)
        .into_iter()
        .filter_map(|token| match token {
            Token::Text(text) => Some(count_words(&text)),
            _ => None,
        })
        .sum()
}

/// Produces dotted section numbers such as `1`, `1.1` and `2.1.1`.
///
/// One counter per level, mirroring the tokenizer's section hierarchy
/// (`part` = 0, `chapter` = 1, `section` = 2, ...). Entering a level bumps its
/// counter and resets every deeper one. Leading zero counters are dropped from
/// the printed number, so a document that only uses `\section` numbers its
/// sections `1`, `2`, ... rather than `0.0.1`.
pub struct SectionTracker {
    counters: Vec<usize>,
}

impl SectionTracker {
    /// Create a tracker covering levels `0..=max_level`.
    pub fn new(max_level: u8) -> Self {
        Self {
            counters: vec![0; max_level as usize + 1],
        }
    }

    /// Enter a section at `level`, returning its dotted number.
    pub fn enter(&mut self, level: u8) -> String {
        let level = level as usize;
        self.counters[level] += 1;
        for counter in &mut self.counters[level + 1..] {
            *counter = 0;
        }
        let parts: Vec<String> = self.counters[..=level]
            .iter()
            .map(|counter| counter.to_string())
            .collect();
        let start = parts
            .iter()
            .position(|part| part != "0")
            .unwrap_or(parts.len() - 1);
        parts[start..].join(".")
    }
}

/// A finished section's attributes: level, number, title, word count.
type Section = (u8, String, String, usize);

/// Word counts collected in one pass over every token stream.
struct RawCounts {
    preamble_words: usize,
    sections: Vec<Section>,
    file_words: Vec<usize>,
}

/// Single-pass counting over all token streams, in `\input` traversal order.
fn count_files(files: &[TokenizedFile]) -> RawCounts {
    let mut tracker = SectionTracker::new(6);
    let mut preamble_words = 0usize;
    let mut sections: Vec<Section> = Vec::new();
    let mut file_words = vec![0usize; files.len()];

    let mut in_document = false;
    let mut current: Option<Section> = None;

    for (file_idx, file) in files.iter().enumerate() {
        for token in &file.tokens {
            match token {
                Token::BeginDocument => in_document = true,
                Token::Section { level, title } => {
                    if !in_document {
                        continue;
                    }
                    if let Some(finished) = current.take() {
                        sections.push(finished);
                    }
                    let number = tracker.enter(*level);
                    let title_words = count_title_words(title);
                    file_words[file_idx] += title_words;
                    current = Some((*level, number, title.clone(), title_words));
                }
                Token::Text(text) => {
                    if !in_document {
                        continue;
                    }
                    let words = count_words(text);
                    if words == 0 {
                        continue;
                    }
                    file_words[file_idx] += words;
                    match &mut current {
                        Some((_, _, _, words_here)) => *words_here += words,
                        None => preamble_words += words,
                    }
                }
                _ => {}
            }
        }
    }
    if let Some(finished) = current {
        sections.push(finished);
    }

    RawCounts {
        preamble_words,
        sections,
        file_words,
    }
}

/// Count words per section for a document.
pub fn count_document(document: &str, files: &[TokenizedFile]) -> DocumentStats {
    let raw = count_files(files);
    let sections: Vec<SectionStat> = raw
        .sections
        .into_iter()
        .map(|(level, number, title, words)| SectionStat {
            path: number,
            level: Some(level),
            title: Some(title),
            words,
        })
        .collect();
    let total_words = raw.preamble_words + sections.iter().map(|s| s.words).sum::<usize>();
    DocumentStats {
        document: document.to_string(),
        total_words,
        file_count: files.len(),
        preamble_words: raw.preamble_words,
        sections,
    }
}

/// Count words per `.tex` file for a document.
pub fn count_by_file(document: &str, files: &[TokenizedFile]) -> DocumentStats {
    let raw = count_files(files);
    let sections: Vec<SectionStat> = raw
        .file_words
        .iter()
        .enumerate()
        .map(|(file_idx, words)| SectionStat {
            path: file_name(&files[file_idx].path),
            level: None,
            title: None,
            words: *words,
        })
        .collect();
    let total_words = raw.file_words.iter().sum();
    DocumentStats {
        document: document.to_string(),
        total_words,
        file_count: files.len(),
        preamble_words: raw.preamble_words,
        sections,
    }
}

/// The file name of a tokenized file.
fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn files_from(fixtures: &[(&str, &str)]) -> Vec<TokenizedFile> {
        fixtures
            .iter()
            .map(|(name, source)| TokenizedFile {
                path: PathBuf::from(name),
                tokens: tokenize(source),
            })
            .collect()
    }

    fn paths_and_words(stats: &DocumentStats) -> Vec<(String, usize)> {
        stats
            .sections
            .iter()
            .map(|section| (section.path.clone(), section.words))
            .collect()
    }

    #[test]
    fn word_is_a_run_of_non_space_containing_an_alphabetic_character() {
        assert_eq!(count_words("Introduccion"), 1);
        assert_eq!(count_words("50"), 0);
        assert_eq!(count_words("fig:cap"), 1);
        assert_eq!(count_words("Hello world, this is prose."), 5);
        assert_eq!(count_words(""), 0);
        assert_eq!(count_words("   \t\n"), 0);
    }

    #[test]
    fn comments_are_not_counted() {
        let files = files_from(&[(
            "main.tex",
            "\\begin{document}\nbody words\n% hidden words here\nmore words\n\\end{document}",
        )]);
        let stats = count_document("Doc", &files);
        assert_eq!(stats.preamble_words, 4);
        assert_eq!(stats.total_words, 4);
        assert!(stats.sections.is_empty());
    }

    #[test]
    fn math_is_not_counted_in_any_form() {
        let src = "\\begin{document}\nbefore $x + y$ after\n$$\\int f dx$$\n\\[a+b\\]\n\\(c+d\\)\n\\begin{equation}e=mc^2\\end{equation}\nmore\n\\end{document}";
        let stats = count_document("Doc", &files_from(&[("main.tex", src)]));
        assert_eq!(stats.preamble_words, 3);
        assert_eq!(stats.total_words, 3);
    }

    #[test]
    fn verbatim_content_is_not_counted() {
        let src = "\\begin{document}\nwords \\begin{verbatim}not words here $math$\\end{verbatim} after\n\\end{document}";
        let stats = count_document("Doc", &files_from(&[("main.tex", src)]));
        assert_eq!(stats.preamble_words, 2);
    }

    #[test]
    fn non_prose_command_arguments_are_not_counted() {
        let src = r"\begin{document}
\label{sec:intro} \ref{fig:x} \cite{key} \input{ch1} \includegraphics{img.png} \url{https://example.com/x} \href{https://example.com}{} \index{LaTeX}
body
\end{document}";
        let stats = count_document("Doc", &files_from(&[("main.tex", src)]));
        assert_eq!(stats.preamble_words, 1);
        assert_eq!(stats.total_words, 1);
    }

    #[test]
    fn prose_command_text_is_counted() {
        let src = r"\begin{document}\section{S}\textit{italic words}\textbf{bold words}\emph{emph words}\footnote{foot words}\caption{cap words}\end{document}";
        let stats = count_document("Doc", &files_from(&[("main.tex", src)]));
        assert_eq!(stats.sections[0].words, 11);
    }

    #[test]
    fn figure_table_and_tabular_text_is_counted() {
        let src = r"\begin{document}\section{S}
\begin{figure}\caption{A cat}\includegraphics{cat.png}\end{figure}
\begin{table}\caption{Timeline}\end{table}
\begin{tabular}{cc}cell one & cell two\end{tabular}
\end{document}";
        let stats = count_document("Doc", &files_from(&[("main.tex", src)]));
        assert_eq!(stats.sections[0].words, 1 + 2 + 1 + 4);
    }

    #[test]
    fn bibliography_text_is_counted() {
        let src = r"\begin{document}\section{S}body
\begin{thebibliography}{9}
\bibitem{knuth} Knuth, The Art of Computer Programming.
\end{thebibliography}
\end{document}";
        let stats = count_document("Doc", &files_from(&[("main.tex", src)]));
        assert_eq!(stats.sections[0].words, 1 + 1 + 6);
    }

    #[test]
    fn preamble_is_not_counted_but_front_matter_is() {
        let src = "\\documentclass{article}\n% preamble prose not counted\n\\begin{document}\nfront matter words\n\\section{S}\nbody\n\\end{document}";
        let stats = count_document("Doc", &files_from(&[("main.tex", src)]));
        assert_eq!(stats.file_count, 1);
        assert_eq!(stats.preamble_words, 3);
        assert_eq!(paths_and_words(&stats), vec![("1".to_string(), 2)]);
        assert_eq!(stats.total_words, 5);
    }

    #[test]
    fn section_numbers_follow_the_hierarchy() {
        let src = r"\begin{document}
\section{One}
\subsection{One One}
\subsection{One Two}
\section{Two}
\subsection{Two One}
\subsubsection{Two One One}
\end{document}";
        let stats = count_document("Doc", &files_from(&[("main.tex", src)]));
        let paths: Vec<&str> = stats.sections.iter().map(|s| s.path.as_str()).collect();
        assert_eq!(paths, vec!["1", "1.1", "1.2", "2", "2.1", "2.1.1"]);
    }

    #[test]
    fn part_and_chapter_levels_number_correctly() {
        let src = "\\begin{document}\n\\part{One}\\chapter{Ch}\\section{Sec}\\section{Sec2}\\chapter{Ch2}\\section{Sec3}\n\\end{document}";
        let stats = count_document("Doc", &files_from(&[("main.tex", src)]));
        let paths: Vec<&str> = stats.sections.iter().map(|s| s.path.as_str()).collect();
        assert_eq!(paths, vec!["1", "1.1", "1.1.1", "1.1.2", "1.2", "1.2.1"]);
    }

    #[test]
    fn section_title_words_are_attributed_to_their_section() {
        let src = r"\begin{document}
\section{Introduction and Background}
text here
\section{Results}
more text
\end{document}";
        let stats = count_document("Doc", &files_from(&[("main.tex", src)]));
        assert_eq!(
            paths_and_words(&stats),
            vec![("1".to_string(), 5), ("2".to_string(), 3)]
        );
    }

    #[test]
    fn math_inside_a_section_title_is_not_counted() {
        let src = r"\begin{document}\section{Energy $E=mc^2$}body\end{document}";
        let stats = count_document("Doc", &files_from(&[("main.tex", src)]));
        assert_eq!(stats.sections[0].words, 2);
    }

    #[test]
    fn sections_from_input_files_are_attributed_to_the_document() {
        let files = files_from(&[
            (
                "main.tex",
                "\\begin{document}\nfront matter\n\\input{ch1}\n\\end{document}",
            ),
            ("ch1.tex", "\\section{One}\nbody"),
        ]);
        let stats = count_document("Doc", &files);
        assert_eq!(stats.preamble_words, 2);
        assert_eq!(paths_and_words(&stats), vec![("1".to_string(), 2)]);
        assert_eq!(stats.total_words, 4);
    }

    #[test]
    fn by_file_breaks_down_per_tex_file() {
        let files = files_from(&[
            (
                "main.tex",
                "\\begin{document}\nfront matter\n\\input{ch1}\n\\end{document}",
            ),
            ("ch1.tex", "\\section{One}\nbody"),
        ]);
        let stats = count_by_file("Doc", &files);
        assert_eq!(stats.total_words, 4);
        assert_eq!(stats.preamble_words, 2);
        assert_eq!(stats.file_count, 2);
        assert_eq!(
            paths_and_words(&stats),
            vec![("main.tex".to_string(), 2), ("ch1.tex".to_string(), 2)]
        );
    }

    #[test]
    fn json_contract_keys_are_stable() {
        let files = files_from(&[(
            "main.tex",
            "\\begin{document}\n\\section{S}\ntext\n\\end{document}",
        )]);
        let stats = count_document("Doc", &files);
        let value = serde_json::to_value(&stats).unwrap();
        let object = value.as_object().unwrap();
        let mut top_keys: Vec<&str> = object.keys().map(String::as_str).collect();
        top_keys.sort_unstable();
        assert_eq!(
            top_keys,
            vec![
                "document",
                "file_count",
                "preamble_words",
                "sections",
                "total_words"
            ]
        );
        let section = object["sections"][0].as_object().unwrap();
        let mut section_keys: Vec<&str> = section.keys().map(String::as_str).collect();
        section_keys.sort_unstable();
        assert_eq!(section_keys, vec!["level", "path", "words"]);
    }
}
