//! A context-aware, single-pass LaTeX tokenizer.
//!
//! [`tokenize`] turns a LaTeX source buffer into a flat [`Token`] stream that
//! answers the one question every later text feature shares: *what is real
//! prose and what is not*. Word counting, spell checking and the glyph linter
//! consume this stream instead of each growing their own parser.
//!
//! [`tokenize_with_spans`] is the position-aware sibling: it pairs every token
//! with the byte range it covers in the source, and reports the byte offsets of
//! `$`-delimited math regions that were never closed. The glyph linter needs
//! both to map findings back to a `file:line`.
//!
//! # Contract
//!
//! * Prose appears as [`Token::Text`]. Text inside math regions, verbatim
//!   regions and comments is never emitted.
//! * [`Token::Command`] carries only the *non-prose* arguments. Arguments that
//!   are prose — `\textit`, `\textbf`, `\emph`, `\footnote`, `\caption`, and
//!   the link text of `\href` — are emitted as [`Token::Text`] immediately
//!   after their command token.
//! * Everything before [`Token::BeginDocument`] is the preamble.
//! * The tokenizer never panics: unbalanced braces, an unterminated
//!   environment or a stray backslash yield tokens and scanning continues.
//! * Comments are recognized at the top level of the stream; a `%` inside a
//!   command argument is kept as literal text.
//!
//! # Limitations
//!
//! * A `%` comment inside a command argument is not stripped.
//! * Math content between `BeginMath`/`EndMath` and verbatim content between
//!   `BeginVerbatim`/`EndVerbatim` is not emitted as tokens.

// This module is the shared parsing contract consumed by word counting
// ([`crate::wordcount`]) and the glyph linter ([`crate::linter::glyphs`]).

mod parser;
mod sections;
mod title;
pub mod verbatim;

use parser::Parser;

use std::path::{Path, PathBuf};

use crate::texutil;

pub use sections::SectionTracker;
pub use verbatim::{verbatim_blocks, verbatim_body_lines};

/// A single token produced by the tokenizer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Token {
    /// Literal prose text outside math, verbatim, and comment regions.
    ///
    /// This is what consumers count and spell-check. The prose argument of a
    /// prose command follows that command's [`Token::Command`] as [`Token::Text`].
    Text(String),

    /// A command and its non-prose arguments.
    ///
    /// `name` is the command name without the leading backslash; control
    /// symbols (`\$`, `\%`, `\{`, `\\`) use the raw character as their name.
    /// `args` holds the raw text of the optional/braced arguments that are not
    /// prose. Prose arguments are emitted as [`Token::Text`] instead.
    Command { name: String, args: Vec<String> },

    /// A sectioning command.
    ///
    /// `level` follows the standard hierarchy: part = 0, chapter = 1, section
    /// = 2, subsection = 3, subsubsection = 4, paragraph = 5, subparagraph =
    /// 6.
    ///
    /// Two consumers need opposite forms of the title, so it is carried
    /// twice:
    ///
    /// * `title` is the first braced argument with any wrapping macros
    ///   (`\textit`, `\href`, `\textcolor`, dot-leader constructs, ...)
    ///   resolved to plain text and escaped specials (`\&`, `\%`, ...)
    ///   unescaped to their literal character — see
    ///   [`resolve_section_title`]. This is what `outline`, `wordcount` and
    ///   `pdftext` display: human-readable prose.
    /// * `raw_title` is the verbatim source text of that same braced
    ///   argument, escapes and macros intact. This is what the glyph linter
    ///   reads: it needs to see `\&` as still escaped to judge whether a
    ///   special character is protected.
    Section {
        level: u8,
        title: String,
        raw_title: String,
    },

    /// Beginning of a math region: `$...$`, `$$...$$`, `\(...\)`, `\[...\]`,
    /// or a math environment (`equation`, `align`, `gather`, `multline`,
    /// `flalign`, `eqnarray`, `displaymath`).
    BeginMath,

    /// End of a math region.
    EndMath,

    /// Beginning of a verbatim region (`verbatim`, `Verbatim`, `lstlisting`,
    /// `minted`, `code`).
    BeginVerbatim { env: String },

    /// End of a verbatim region.
    EndVerbatim { env: String },

    /// A comment: `%` through end of line, including the leading `%`.
    Comment(String),

    /// `\begin{document}`. Everything before this token is the preamble.
    BeginDocument,

    /// `\end{document}`.
    EndDocument,

    /// Any other environment, emitted for both `\begin{env}` and `\end{env}`
    /// in nesting order.
    Environment { name: String },
}

/// Math environments handled by the tokenizer (including the `*`-variants,
/// which are the same environments in unnumbered form).
const MATH_ENVIRONMENTS: &[&str] = &[
    "equation",
    "equation*",
    "align",
    "align*",
    "alignat",
    "alignat*",
    "gather",
    "gather*",
    "multline",
    "multline*",
    "flalign",
    "flalign*",
    "eqnarray",
    "eqnarray*",
    "displaymath",
];

/// Verbatim environments handled by the tokenizer. `code` is texforge's own
/// highlighting environment (see `crate::highlight`): its body is source code
/// and must no more reach the spell/glyph/package checks than `lstlisting`'s
/// does — the same reason the formatter keeps it in its verbatim list.
const VERBATIM_ENVIRONMENTS: &[&str] = &["verbatim", "Verbatim", "lstlisting", "minted", "code"];

/// Table environments whose `\begin{...}` is followed by a column
/// specification (and, for the starred/extended forms, a width) rather than
/// content. These arguments are typography, not prose — e.g.
/// `\begin{tabular}{@{}>{\bfseries}p{3cm}...}` — and must never reach the
/// significant-word stream that the fidelity check draws from.
const TABLE_PREAMBLE_ENVIRONMENTS: &[&str] = &[
    "tabular",
    "tabular*",
    "longtable",
    "tabularx",
    "tabulary",
    "array",
];

/// Commands whose braced arguments are prose.
const PROSE_COMMANDS: &[&str] = &["textit", "textbf", "emph", "footnote", "caption"];

/// One tokenized file, paired with the file it came from.
#[derive(Debug)]
pub struct TokenizedFile {
    /// Absolute path of the tokenized `.tex` file.
    pub path: PathBuf,
    /// The token stream for that file.
    pub tokens: Vec<Token>,
}

/// A token paired with the byte span it covers in the source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpannedToken {
    /// The token itself.
    pub token: Token,
    /// Byte offset of the first character of the construct, inclusive.
    pub start: usize,
    /// Byte offset just past the last character of the construct, exclusive.
    pub end: usize,
}

/// The outcome of a position-aware tokenization.
#[derive(Debug)]
pub struct TokenizedSource {
    /// The flat token stream, each with its source span.
    pub tokens: Vec<SpannedToken>,
    /// Byte offset of every `$`/`$$` delimiter that opened a math region with
    /// no matching close in the source (the tokenizer synthesized the closing
    /// delimiter at end of input).
    pub unclosed_math: Vec<usize>,
}

/// Tokenize a single LaTeX source buffer.
pub fn tokenize(source: &str) -> Vec<Token> {
    tokenize_with_spans(source)
        .tokens
        .into_iter()
        .map(|spanned| spanned.token)
        .collect()
}

/// Tokenize a single LaTeX source buffer, pairing each token with its span.
///
/// The spans of prose-command arguments (`\textit{...}` and friends) and of
/// `\href` link text are absolute offsets into `source`, so consumers can map
/// any finding back to a `file:line` without re-scanning the buffer.
pub fn tokenize_with_spans(source: &str) -> TokenizedSource {
    Parser::new(source).run()
}

/// Shift a tokenized buffer by `offset` bytes, as used when prose-command
/// arguments are re-tokenized inside the enclosing source.
fn offset_tokens(tokens: Vec<SpannedToken>, offset: usize) -> Vec<SpannedToken> {
    tokens
        .into_iter()
        .map(|mut spanned| {
            spanned.start += offset;
            spanned.end += offset;
            spanned
        })
        .collect()
}

/// Tokenize every `.tex` file reachable from `entry` via `\input{}`.
///
/// Traversal reuses [`texutil::collect_tex_files`], so the file set matches
/// what the linter inspects.
pub fn tokenize_document(root: &Path, entry: &str) -> Vec<TokenizedFile> {
    texutil::collect_tex_files(root, entry)
        .files
        .into_iter()
        .filter_map(|path| {
            let source = std::fs::read_to_string(&path).ok()?;
            Some(TokenizedFile {
                path,
                tokens: tokenize(&source),
            })
        })
        .collect()
}

/// Command-name characters beyond plain ASCII letters (internal commands use `@`).
fn is_command_char(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '@'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_prose_is_text() {
        let tokens = tokenize("Hello world, this is prose.");
        assert_eq!(
            tokens,
            vec![Token::Text("Hello world, this is prose.".to_string())]
        );
    }

    #[test]
    fn command_names_distinct_from_arguments() {
        let tokens = tokenize(r"\label{sec:intro} and \ref{fig:x}");
        assert_eq!(
            tokens,
            vec![
                Token::Command {
                    name: "label".to_string(),
                    args: vec!["sec:intro".to_string()],
                },
                Token::Text(" and ".to_string()),
                Token::Command {
                    name: "ref".to_string(),
                    args: vec!["fig:x".to_string()],
                },
            ]
        );
    }

    #[test]
    fn cite_is_command() {
        let tokens = tokenize(r"\cite{knuth1984}");
        assert_eq!(
            tokens,
            vec![Token::Command {
                name: "cite".to_string(),
                args: vec!["knuth1984".to_string()],
            }]
        );
    }

    #[test]
    fn input_is_command() {
        let tokens = tokenize(r"\input{chapter1}");
        assert_eq!(
            tokens,
            vec![Token::Command {
                name: "input".to_string(),
                args: vec!["chapter1".to_string()],
            }]
        );
    }

    #[test]
    fn includegraphics_with_options_is_command() {
        let tokens = tokenize(r"\includegraphics[width=0.5\textwidth]{img.png}");
        assert_eq!(
            tokens,
            vec![Token::Command {
                name: "includegraphics".to_string(),
                args: vec![r"width=0.5\textwidth".to_string(), "img.png".to_string()],
            }]
        );
    }

    #[test]
    fn index_is_command() {
        let tokens = tokenize(r"\index{LaTeX}");
        assert_eq!(
            tokens,
            vec![Token::Command {
                name: "index".to_string(),
                args: vec!["LaTeX".to_string()],
            }]
        );
    }

    #[test]
    fn inline_math_is_marked() {
        let tokens = tokenize("The value is $x+1$ here.");
        assert_eq!(
            tokens,
            vec![
                Token::Text("The value is ".to_string()),
                Token::BeginMath,
                Token::EndMath,
                Token::Text(" here.".to_string()),
            ]
        );
    }

    #[test]
    fn display_math_dollars_is_marked() {
        let tokens = tokenize("Before $$\\int f dx$$ after.");
        assert_eq!(
            tokens,
            vec![
                Token::Text("Before ".to_string()),
                Token::BeginMath,
                Token::EndMath,
                Token::Text(" after.".to_string()),
            ]
        );
    }

    #[test]
    fn bracket_display_math_is_marked() {
        let tokens = tokenize(r"Before \[a+b\] after.");
        assert_eq!(
            tokens,
            vec![
                Token::Text("Before ".to_string()),
                Token::BeginMath,
                Token::EndMath,
                Token::Text(" after.".to_string()),
            ]
        );
    }

    #[test]
    fn paren_inline_math_is_marked() {
        let tokens = tokenize(r"Before \(x\) after.");
        assert_eq!(
            tokens,
            vec![
                Token::Text("Before ".to_string()),
                Token::BeginMath,
                Token::EndMath,
                Token::Text(" after.".to_string()),
            ]
        );
    }

    #[test]
    fn math_environments_are_marked() {
        for env in MATH_ENVIRONMENTS {
            let src = format!("Before \\begin{{{env}}}a+b\\end{{{env}}} after.");
            let tokens = tokenize(&src);
            assert_eq!(
                tokens,
                vec![
                    Token::Text("Before ".to_string()),
                    Token::BeginMath,
                    Token::EndMath,
                    Token::Text(" after.".to_string()),
                ],
                "math env: {env}"
            );
        }
    }

    #[test]
    fn math_content_is_not_prose() {
        let tokens = tokenize("$\\text{alpha} + \\beta$");
        assert_eq!(tokens, vec![Token::BeginMath, Token::EndMath]);
    }

    #[test]
    fn verbatim_environments_are_marked() {
        for env in VERBATIM_ENVIRONMENTS {
            let src = format!("\\begin{{{env}}}% $nonsense$ 100% \\end{{{env}}} after.");
            let tokens = tokenize(&src);
            assert_eq!(
                tokens,
                vec![
                    Token::BeginVerbatim {
                        env: env.to_string(),
                    },
                    Token::EndVerbatim {
                        env: env.to_string(),
                    },
                    Token::Text(" after.".to_string()),
                ],
                "verbatim env: {env}"
            );
        }
    }

    #[test]
    fn comments_are_tokens() {
        let tokens = tokenize("Text % a comment\nmore");
        assert_eq!(
            tokens,
            vec![
                Token::Text("Text ".to_string()),
                Token::Comment("% a comment".to_string()),
                Token::Text("\nmore".to_string()),
            ]
        );
    }

    #[test]
    fn escaped_percent_is_not_a_comment() {
        let tokens = tokenize(r"50\% off");
        assert_eq!(
            tokens,
            vec![
                Token::Text("50".to_string()),
                Token::Command {
                    name: "%".to_string(),
                    args: Vec::new(),
                },
                Token::Text(" off".to_string()),
            ]
        );
    }

    #[test]
    fn urls_are_not_prose() {
        let tokens = tokenize(r"\url{https://example.com/x}");
        assert_eq!(
            tokens,
            vec![Token::Command {
                name: "url".to_string(),
                args: vec!["https://example.com/x".to_string()],
            }]
        );
    }

    #[test]
    fn href_splits_url_from_link_text() {
        let tokens = tokenize(r"\href{https://example.com}{Example}");
        assert_eq!(
            tokens,
            vec![
                Token::Command {
                    name: "href".to_string(),
                    args: vec!["https://example.com".to_string()],
                },
                Token::Text("Example".to_string()),
            ]
        );
    }

    #[test]
    fn sectioning_commands_are_marked() {
        let tokens = tokenize(
            r"\part{One}\chapter{Two}\section{Three}\subsection{Four}\subsubsection{Five}\paragraph{Six}\subparagraph{Seven}",
        );
        let levels: Vec<u8> = tokens
            .iter()
            .filter_map(|t| match t {
                Token::Section { level, .. } => Some(*level),
                _ => None,
            })
            .collect();
        assert_eq!(levels, vec![0, 1, 2, 3, 4, 5, 6]);
        assert_eq!(
            tokens[2],
            Token::Section {
                level: 2,
                title: "Three".to_string(),
                raw_title: "Three".to_string(),
            }
        );
    }

    #[test]
    fn starred_section_is_marked() {
        let tokens = tokenize(r"\section*{Intro}");
        assert_eq!(
            tokens,
            vec![Token::Section {
                level: 2,
                title: "Intro".to_string(),
                raw_title: "Intro".to_string(),
            }]
        );
    }

    #[test]
    fn section_with_optional_toc_title_uses_required_title() {
        let tokens = tokenize(r"\section[Short]{Full title}");
        assert_eq!(
            tokens,
            vec![Token::Section {
                level: 2,
                title: "Full title".to_string(),
                raw_title: "Full title".to_string(),
            }]
        );
    }

    #[test]
    fn prose_commands_emit_text() {
        for cmd in PROSE_COMMANDS {
            let src = format!("\\{cmd}{{Hello world}}");
            let tokens = tokenize(&src);
            assert_eq!(
                tokens,
                vec![
                    Token::Command {
                        name: cmd.to_string(),
                        args: Vec::new(),
                    },
                    Token::Text("Hello world".to_string()),
                ],
                "prose cmd: {cmd}"
            );
        }
    }

    #[test]
    fn caption_optional_argument_is_non_prose() {
        let tokens = tokenize(r"\caption[Short]{Long caption}");
        assert_eq!(
            tokens,
            vec![
                Token::Command {
                    name: "caption".to_string(),
                    args: vec!["Short".to_string()],
                },
                Token::Text("Long caption".to_string()),
            ]
        );
    }

    #[test]
    fn math_inside_caption_is_handled() {
        let tokens = tokenize(r"\caption{The value is $x$}");
        assert_eq!(
            tokens,
            vec![
                Token::Command {
                    name: "caption".to_string(),
                    args: Vec::new(),
                },
                Token::Text("The value is ".to_string()),
                Token::BeginMath,
                Token::EndMath,
            ]
        );
    }

    #[test]
    fn nested_prose_command_in_braces_is_handled() {
        let tokens = tokenize(r"\textbf{see \ref{fig:1}}");
        assert_eq!(
            tokens,
            vec![
                Token::Command {
                    name: "textbf".to_string(),
                    args: Vec::new(),
                },
                Token::Text("see ".to_string()),
                Token::Command {
                    name: "ref".to_string(),
                    args: vec!["fig:1".to_string()],
                },
            ]
        );
    }

    #[test]
    fn verbatim_inside_figure_is_handled() {
        let tokens = tokenize(
            "\\begin{figure}\n\\begin{verbatim}\nx = $y$ 100%\n\\end{verbatim}\n\\end{figure}",
        );
        assert_eq!(
            tokens,
            vec![
                Token::Environment {
                    name: "figure".to_string(),
                },
                Token::Text("\n".to_string()),
                Token::BeginVerbatim {
                    env: "verbatim".to_string(),
                },
                Token::EndVerbatim {
                    env: "verbatim".to_string(),
                },
                Token::Text("\n".to_string()),
                Token::Environment {
                    name: "figure".to_string(),
                },
            ]
        );
    }

    #[test]
    fn tabular_column_spec_is_not_prose() {
        let tokens = tokenize(
            r"\begin{tabular}{@{}>{\bfseries}p{3cm}>{\raggedright\arraybackslash}p{5.5cm}@{}}Name & Alice \\\end{tabular}",
        );
        for t in &tokens {
            if let Token::Text(text) = t {
                assert!(!text.contains("p{3cm"), "column spec leaked: {text:?}");
                assert!(!text.contains("p{5.5cm"), "column spec leaked: {text:?}");
            }
        }
        assert!(tokens.iter().any(
            |t| matches!(t, Token::Text(text) if text.contains("Name") || text.contains("Alice"))
        ));
    }

    #[test]
    fn tabular_star_and_x_and_y_column_specs_are_not_prose() {
        for source in [
            r"\begin{tabular*}{\textwidth}[t]{lcr}x\end{tabular*}",
            r"\begin{tabularx}{\textwidth}{lX}x\end{tabularx}",
            r"\begin{tabulary}{\textwidth}{lC}x\end{tabulary}",
            r"\begin{longtable}[c]{ll}x\end{longtable}",
            r"\begin{array}{cc}x\end{array}",
        ] {
            let tokens = tokenize(source);
            for t in &tokens {
                if let Token::Text(text) = t {
                    assert!(
                        !text.contains('{') && !text.contains('}'),
                        "preamble leaked from {source:?}: {text:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn generic_environments_preserve_order() {
        let tokens = tokenize("\\begin{figure}\\begin{tabular}c\\end{tabular}\\end{figure}");
        assert_eq!(
            tokens,
            vec![
                Token::Environment {
                    name: "figure".to_string(),
                },
                Token::Environment {
                    name: "tabular".to_string(),
                },
                Token::Text("c".to_string()),
                Token::Environment {
                    name: "tabular".to_string(),
                },
                Token::Environment {
                    name: "figure".to_string(),
                },
            ]
        );
    }

    #[test]
    fn unterminated_environment_does_not_panic() {
        let tokens = tokenize("\\begin{verbatim}\nraw $ content %");
        assert_eq!(
            tokens,
            vec![
                Token::BeginVerbatim {
                    env: "verbatim".to_string(),
                },
                Token::EndVerbatim {
                    env: "verbatim".to_string(),
                },
            ]
        );
    }

    #[test]
    fn unbalanced_braces_do_not_panic() {
        let tokens = tokenize(r"\textbf{unclosed and }}}} stray");
        assert!(tokens.iter().any(|t| matches!(
            t,
            Token::Command { name, .. } if name == "textbf"
        )));
    }

    #[test]
    fn stray_backslash_does_not_panic() {
        let tokens = tokenize("trailing \\");
        assert_eq!(
            tokens,
            vec![
                Token::Text("trailing ".to_string()),
                Token::Command {
                    name: "\\".to_string(),
                    args: Vec::new(),
                },
            ]
        );
    }

    #[test]
    fn escaped_dollar_is_not_math() {
        let tokens = tokenize(r"cost: \$5");
        assert_eq!(
            tokens,
            vec![
                Token::Text("cost: ".to_string()),
                Token::Command {
                    name: "$".to_string(),
                    args: Vec::new(),
                },
                Token::Text("5".to_string()),
            ]
        );
    }

    #[test]
    fn preamble_precedes_begin_document() {
        let tokens = tokenize(
            "\\documentclass{article}\n\\usepackage{amsmath}\n\\begin{document}\nHello world\n\\end{document}",
        );
        let doc_pos = tokens
            .iter()
            .position(|t| matches!(t, Token::BeginDocument))
            .unwrap();
        assert!(tokens[..doc_pos]
            .iter()
            .all(|t| matches!(t, Token::Command { .. } | Token::Text(_))));
        assert!(tokens[doc_pos..]
            .iter()
            .any(|t| matches!(t, Token::Text(s) if s.contains("Hello"))));
        assert!(tokens.iter().any(|t| matches!(t, Token::EndDocument)));
    }

    #[test]
    fn trailing_empty_group_is_an_argument() {
        let tokens = tokenize(r"\LaTeX{}");
        assert_eq!(
            tokens,
            vec![Token::Command {
                name: "LaTeX".to_string(),
                args: vec!["".to_string()],
            }]
        );
    }

    #[test]
    fn tokenize_document_traverses_input_files() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("main.tex"),
            "\\input{ch1}\n\\begin{document}\nMain\\end{document}",
        )
        .unwrap();
        std::fs::write(dir.path().join("ch1.tex"), "% ch1\nText in chapter").unwrap();

        let files = tokenize_document(dir.path(), "main.tex");
        assert_eq!(files.len(), 2);

        let ch1 = files.iter().find(|f| f.path.ends_with("ch1.tex")).unwrap();
        assert!(ch1
            .tokens
            .iter()
            .any(|t| matches!(t, Token::Comment(c) if c == "% ch1")));

        let main = files.iter().find(|f| f.path.ends_with("main.tex")).unwrap();
        assert!(main
            .tokens
            .iter()
            .any(|t| matches!(t, Token::BeginDocument)));
    }

    #[test]
    fn tokenize_agrees_with_tokenize_with_spans() {
        let src = r"before $x+1$ after \textit{emph} 50% done";
        let plain = tokenize(src);
        let spanned = tokenize_with_spans(src);
        let projected: Vec<Token> = spanned.tokens.iter().map(|s| s.token.clone()).collect();
        assert_eq!(plain, projected);
    }

    #[test]
    fn spans_cover_text_comments_and_commands() {
        let src = "hola % c\n\\ref{fig:1}";
        let spanned = tokenize_with_spans(src).tokens;
        assert_eq!(
            spanned,
            vec![
                SpannedToken {
                    token: Token::Text("hola ".to_string()),
                    start: 0,
                    end: 5,
                },
                SpannedToken {
                    token: Token::Comment("% c".to_string()),
                    start: 5,
                    end: 8,
                },
                SpannedToken {
                    token: Token::Text("\n".to_string()),
                    start: 8,
                    end: 9,
                },
                SpannedToken {
                    token: Token::Command {
                        name: "ref".to_string(),
                        args: vec!["fig:1".to_string()],
                    },
                    start: 9,
                    end: 20,
                },
            ]
        );
    }

    #[test]
    fn prose_command_args_get_absolute_spans() {
        let src = "\\textit{hello world}";
        let spanned = tokenize_with_spans(src).tokens;
        let text = spanned
            .iter()
            .find(|s| matches!(s.token, Token::Text(_)))
            .unwrap();
        assert_eq!(
            *text,
            SpannedToken {
                token: Token::Text("hello world".to_string()),
                start: 8,
                end: 19,
            }
        );
        assert_eq!(&src[text.start..text.end], "hello world");
    }

    #[test]
    fn href_link_text_gets_absolute_spans() {
        let src = r"\href{https://example.com}{Example}";
        let spanned = tokenize_with_spans(src).tokens;
        let text = spanned
            .iter()
            .find(|s| matches!(s.token, Token::Text(_)))
            .unwrap();
        assert_eq!(&src[text.start..text.end], "Example");
    }

    #[test]
    fn verb_commands_are_not_prose() {
        let src = r"a \verb|b_c & d| e \verb*#$x$# f \lstinline|l_m| g";
        let tokens = tokenize(src);
        let has_prose = tokens
            .iter()
            .any(|t| matches!(t, Token::Text(s) if s.contains('_')));
        assert!(!has_prose);
        assert!(tokens.iter().any(|t| matches!(
            t,
            Token::Command { name, args } if name == "verb" && args.contains(&"b_c & d".to_string())
        )));
        assert!(tokens.iter().any(|t| matches!(
            t,
            Token::Command { name, args } if name == "verb" && args.contains(&"$x$".to_string())
        )));
        assert!(tokens.iter().any(|t| matches!(
            t,
            Token::Command { name, args } if name == "lstinline" && args.contains(&"l_m".to_string())
        )));
    }

    #[test]
    fn starred_math_environments_are_stripped() {
        for env in [
            "equation*",
            "align*",
            "alignat*",
            "gather*",
            "multline*",
            "flalign*",
        ] {
            let src = format!("before \\begin{{{env}}}a &= b_{{0}}\\end{{{env}}} after");
            let tokens = tokenize(&src);
            assert_eq!(
                tokens,
                vec![
                    Token::Text("before ".to_string()),
                    Token::BeginMath,
                    Token::EndMath,
                    Token::Text(" after".to_string()),
                ],
                "starred math env: {env}"
            );
        }
    }

    #[test]
    fn unclosed_dollar_math_is_reported() {
        let src = "costo: $5 dolares\nfin";
        let tokenized = tokenize_with_spans(src);
        assert_eq!(tokenized.unclosed_math, vec![7]);
        assert_eq!(&src[7..8], "$");
    }

    #[test]
    fn closed_dollar_math_is_not_reported() {
        let tokenized = tokenize_with_spans(r"El valor es $x^2$");
        assert!(tokenized.unclosed_math.is_empty());
    }

    #[test]
    fn unclosed_dollar_in_prose_command_is_reported_absolutely() {
        let src = r"\textit{costo $5}";
        let tokenized = tokenize_with_spans(src);
        assert_eq!(tokenized.unclosed_math, vec![14]);
        assert_eq!(&src[14..15], "$");
    }

    /// `\[ .. \]` display math: the open span is exactly the two bytes of
    /// `\[`, the synthesized close span is pinned where the content began
    /// (the position captured before skipping ahead), and the tail text
    /// resumes after the real `\]`.
    #[test]
    fn bracket_display_math_delimiters_carry_exact_spans() {
        let src = r"x\[abc\] tail";
        let spanned = tokenize_with_spans(src).tokens;
        assert_eq!(
            spanned,
            vec![
                SpannedToken {
                    token: Token::Text("x".to_string()),
                    start: 0,
                    end: 1,
                },
                SpannedToken {
                    token: Token::BeginMath,
                    start: 1,
                    end: 3,
                },
                SpannedToken {
                    token: Token::EndMath,
                    start: 3,
                    end: 5,
                },
                SpannedToken {
                    token: Token::Text(" tail".to_string()),
                    start: 8,
                    end: 13,
                },
            ]
        );
        assert_eq!(&src[1..3], r"\[");
    }

    /// The paren form behaves byte-for-byte like the bracket form.
    #[test]
    fn paren_inline_math_delimiters_carry_exact_spans() {
        let src = r"x\(abc\) tail";
        let spanned = tokenize_with_spans(src).tokens;
        assert_eq!(
            spanned,
            vec![
                SpannedToken {
                    token: Token::Text("x".to_string()),
                    start: 0,
                    end: 1,
                },
                SpannedToken {
                    token: Token::BeginMath,
                    start: 1,
                    end: 3,
                },
                SpannedToken {
                    token: Token::EndMath,
                    start: 3,
                    end: 5,
                },
                SpannedToken {
                    token: Token::Text(" tail".to_string()),
                    start: 8,
                    end: 13,
                },
            ]
        );
        assert_eq!(&src[1..3], r"\(");
    }

    /// A stray close delimiter (no matching open) still marks an `EndMath`
    /// over its own two bytes instead of falling through as a control symbol.
    #[test]
    fn stray_control_math_closes_are_end_math_tokens() {
        for (src, close) in [(r"x\] tail", "]"), (r"x\) tail", ")")] {
            let spanned = tokenize_with_spans(src).tokens;
            assert_eq!(
                spanned,
                vec![
                    SpannedToken {
                        token: Token::Text("x".to_string()),
                        start: 0,
                        end: 1,
                    },
                    SpannedToken {
                        token: Token::EndMath,
                        start: 1,
                        end: 3,
                    },
                    SpannedToken {
                        token: Token::Text(" tail".to_string()),
                        start: 3,
                        end: 8,
                    },
                ],
                "stray close: {close}"
            );
            assert_eq!(src[1..3].to_string(), format!("\\{close}"));
        }
    }

    /// `$$` display math: the begin span covers both dollar bytes and the end
    /// span covers the closing pair, so a span multiplied by the delimiter
    /// length cannot masquerade as the right boundary.
    #[test]
    fn display_dollar_delimiters_carry_exact_spans() {
        let src = "x$$ab$$ y";
        let spanned = tokenize_with_spans(src).tokens;
        assert_eq!(
            spanned,
            vec![
                SpannedToken {
                    token: Token::Text("x".to_string()),
                    start: 0,
                    end: 1,
                },
                SpannedToken {
                    token: Token::BeginMath,
                    start: 1,
                    end: 3,
                },
                SpannedToken {
                    token: Token::EndMath,
                    start: 5,
                    end: 7,
                },
                SpannedToken {
                    token: Token::Text(" y".to_string()),
                    start: 7,
                    end: 9,
                },
            ]
        );
        assert_eq!(&src[1..3], "$$");
        assert_eq!(&src[5..7], "$$");
    }

    /// An unclosed `$` inside `\href` link text must be reported at its
    /// absolute offset in the outer source, not at the sub-buffer offset.
    #[test]
    fn unclosed_math_inside_href_is_offset_to_the_outer_source() {
        let src = r"\href{url}{$x}";
        let tokenized = tokenize_with_spans(src);
        assert_eq!(tokenized.unclosed_math, vec![11]);
        assert_eq!(&src[11..12], "$");
    }

    /// `\verb` with a bracket delimiter: the delimiter is the first non-letter
    /// after the name, so everything after it — brackets included — is the
    /// verbatim body, never an option group.
    #[test]
    fn verb_with_bracket_delimiter_reads_the_rest_as_verbatim() {
        let tokens = tokenize(r"\verb[abc]def");
        assert_eq!(
            tokens,
            vec![Token::Command {
                name: "verb".to_string(),
                args: vec!["abc]def".to_string()],
            }]
        );
    }

    /// `\lstinline` collects its option groups first, then reads the body up
    /// to the verbatim delimiter.
    #[test]
    fn lstinline_collects_options_then_the_verbatim_body() {
        let tokens = tokenize(r"\lstinline[opts]|x|");
        assert_eq!(
            tokens,
            vec![
                Token::Command {
                    name: "lstinline".to_string(),
                    args: vec!["opts".to_string(), "x".to_string()],
                },
                Token::Text("|".to_string()),
            ]
        );
    }
}
