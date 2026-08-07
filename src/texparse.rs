//! A context-aware, single-pass LaTeX tokenizer.
//!
//! [`tokenize`] turns a LaTeX source buffer into a flat [`Token`] stream that
//! answers the one question every later text feature shares: *what is real
//! prose and what is not*. Word counting, spell checking and the glyph linter
//! consume this stream instead of each growing their own parser.
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

// This module is the shared parsing contract for the upcoming word-count,
// spell-check and glyph-linter features and is deliberately not yet wired into
// the binary; its whole API surface is exercised by the tests below.
#![allow(dead_code)]

use std::path::{Path, PathBuf};

use crate::texutil;

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
    /// 6. `title` is the raw text of the first braced argument.
    Section { level: u8, title: String },

    /// Beginning of a math region: `$...$`, `$$...$$`, `\(...\)`, `\[...\]`,
    /// or a math environment (`equation`, `align`, `gather`, `multline`,
    /// `flalign`, `eqnarray`, `displaymath`).
    BeginMath,

    /// End of a math region.
    EndMath,

    /// Beginning of a verbatim region (`verbatim`, `Verbatim`, `lstlisting`,
    /// `minted`).
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

/// Math environments handled by the tokenizer.
const MATH_ENVIRONMENTS: &[&str] = &[
    "equation",
    "align",
    "gather",
    "multline",
    "flalign",
    "eqnarray",
    "displaymath",
];

/// Verbatim environments handled by the tokenizer.
const VERBATIM_ENVIRONMENTS: &[&str] = &["verbatim", "Verbatim", "lstlisting", "minted"];

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

/// Tokenize a single LaTeX source buffer.
pub fn tokenize(source: &str) -> Vec<Token> {
    Parser::new(source).run()
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

/// Nesting level for a sectioning command name (a trailing `*` is ignored).
fn section_level(name: &str) -> Option<u8> {
    match name.trim_end_matches('*') {
        "part" => Some(0),
        "chapter" => Some(1),
        "section" => Some(2),
        "subsection" => Some(3),
        "subsubsection" => Some(4),
        "paragraph" => Some(5),
        "subparagraph" => Some(6),
        _ => None,
    }
}

/// Single-pass state machine over the character stream.
struct Parser<'a> {
    src: &'a str,
    pos: usize,
}

impl<'a> Parser<'a> {
    fn new(src: &'a str) -> Self {
        Parser { src, pos: 0 }
    }

    fn peek(&self) -> Option<char> {
        self.src[self.pos..].chars().next()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += c.len_utf8();
        Some(c)
    }

    fn eat(&mut self, expected: char) -> bool {
        if self.peek() == Some(expected) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn run(mut self) -> Vec<Token> {
        let mut tokens = Vec::new();
        let mut text = String::new();

        while let Some(c) = self.peek() {
            match c {
                '%' => {
                    self.flush_text(&mut tokens, &mut text);
                    tokens.push(Token::Comment(self.read_comment()));
                }
                '\\' => {
                    self.flush_text(&mut tokens, &mut text);
                    self.handle_backslash(&mut tokens);
                }
                '$' => {
                    self.flush_text(&mut tokens, &mut text);
                    self.read_dollar_math(&mut tokens);
                }
                _ => {
                    text.push(c);
                    self.bump();
                }
            }
        }
        self.flush_text(&mut tokens, &mut text);
        tokens
    }

    fn flush_text(&self, tokens: &mut Vec<Token>, text: &mut String) {
        if !text.is_empty() {
            tokens.push(Token::Text(std::mem::take(text)));
        }
    }

    /// Read a comment: `%` through end of line (the newline is not consumed).
    fn read_comment(&mut self) -> String {
        let start = self.pos;
        while let Some(c) = self.peek() {
            if c == '\n' {
                break;
            }
            self.bump();
        }
        self.src[start..self.pos].to_string()
    }

    fn handle_backslash(&mut self, tokens: &mut Vec<Token>) {
        self.bump(); // consume the backslash
        let Some(c) = self.peek() else {
            tokens.push(Token::Command {
                name: "\\".to_string(),
                args: Vec::new(),
            });
            return;
        };

        match c {
            '[' => {
                self.bump();
                tokens.push(Token::BeginMath);
                self.skip_to_control_close(']');
                tokens.push(Token::EndMath);
            }
            ']' => {
                self.bump();
                tokens.push(Token::EndMath);
            }
            '(' => {
                self.bump();
                tokens.push(Token::BeginMath);
                self.skip_to_control_close(')');
                tokens.push(Token::EndMath);
            }
            ')' => {
                self.bump();
                tokens.push(Token::EndMath);
            }
            _ if is_command_char(c) => {
                let name = self.read_command_name();
                self.handle_command(&name, tokens);
            }
            _ => {
                // Control symbol: `\\`, `\$`, `\%`, `\{`, `\&`, ...
                self.bump();
                tokens.push(Token::Command {
                    name: c.to_string(),
                    args: Vec::new(),
                });
            }
        }
    }

    fn read_command_name(&mut self) -> String {
        let start = self.pos;
        while let Some(c) = self.peek() {
            if is_command_char(c) {
                self.bump();
            } else {
                break;
            }
        }
        self.src[start..self.pos].to_string()
    }

    fn handle_command(&mut self, name: &str, tokens: &mut Vec<Token>) {
        match name {
            "begin" => self.handle_begin(tokens),
            "end" => self.handle_end(tokens),
            _ => {
                if let Some(level) = section_level(name) {
                    self.eat('*');
                    let _ = self.read_bracket_group();
                    let title = self.read_braced_group().unwrap_or_default();
                    tokens.push(Token::Section { level, title });
                } else if name == "href" {
                    self.handle_href(tokens);
                } else if PROSE_COMMANDS.contains(&name) {
                    self.handle_prose_command(name, tokens);
                } else {
                    self.handle_plain_command(name, tokens);
                }
            }
        }
    }

    fn handle_begin(&mut self, tokens: &mut Vec<Token>) {
        let env = self.read_braced_group().unwrap_or_default();
        // Discard float placement etc.: `\begin{figure}[htbp]`.
        let _ = self.read_bracket_group();

        if env == "document" {
            tokens.push(Token::BeginDocument);
            return;
        }
        if MATH_ENVIRONMENTS.contains(&env.as_str()) {
            tokens.push(Token::BeginMath);
            self.skip_to_env_end(&env);
            tokens.push(Token::EndMath);
            return;
        }
        if VERBATIM_ENVIRONMENTS.contains(&env.as_str()) {
            tokens.push(Token::BeginVerbatim { env: env.clone() });
            self.skip_to_env_end(&env);
            tokens.push(Token::EndVerbatim { env });
            return;
        }
        tokens.push(Token::Environment { name: env });
    }

    fn handle_end(&mut self, tokens: &mut Vec<Token>) {
        let env = self.read_braced_group().unwrap_or_default();
        if env == "document" {
            tokens.push(Token::EndDocument);
        } else if MATH_ENVIRONMENTS.contains(&env.as_str()) {
            tokens.push(Token::EndMath);
        } else if VERBATIM_ENVIRONMENTS.contains(&env.as_str()) {
            tokens.push(Token::EndVerbatim { env });
        } else {
            tokens.push(Token::Environment { name: env });
        }
    }

    /// `\href{url}{text}`: the URL is a non-prose argument, the link text is prose.
    fn handle_href(&mut self, tokens: &mut Vec<Token>) {
        let mut args = Vec::new();
        while let Some(optional) = self.read_bracket_group() {
            args.push(optional);
        }
        if let Some(url) = self.read_braced_group() {
            args.push(url);
        }
        tokens.push(Token::Command {
            name: "href".to_string(),
            args,
        });
        if let Some(text) = self.read_braced_group() {
            tokens.extend(tokenize(&text));
        }
    }

    /// A command whose braced argument is prose, emitted as [`Token::Text`].
    fn handle_prose_command(&mut self, name: &str, tokens: &mut Vec<Token>) {
        let mut args = Vec::new();
        while let Some(optional) = self.read_bracket_group() {
            args.push(optional);
        }
        tokens.push(Token::Command {
            name: name.to_string(),
            args,
        });
        if let Some(prose) = self.read_braced_group() {
            tokens.extend(tokenize(&prose));
        }
    }

    /// Any other command: greedily collect optional and braced arguments.
    fn handle_plain_command(&mut self, name: &str, tokens: &mut Vec<Token>) {
        let mut args = Vec::new();
        loop {
            if let Some(optional) = self.read_bracket_group() {
                args.push(optional);
                continue;
            }
            if let Some(braced) = self.read_braced_group() {
                args.push(braced);
                continue;
            }
            break;
        }
        tokens.push(Token::Command {
            name: name.to_string(),
            args,
        });
    }

    /// Read a balanced `{...}` group, returning its raw content (escaped
    /// braces `\{`/`\}` do not affect nesting).
    fn read_braced_group(&mut self) -> Option<String> {
        if !self.eat('{') {
            return None;
        }
        let mut depth = 1usize;
        let mut out = String::new();
        while let Some(c) = self.peek() {
            match c {
                '\\' => {
                    out.push('\\');
                    self.bump();
                    if let Some(next) = self.bump() {
                        out.push(next);
                    }
                }
                '{' => {
                    depth += 1;
                    out.push('{');
                    self.bump();
                }
                '}' => {
                    depth -= 1;
                    self.bump();
                    if depth == 0 {
                        break;
                    }
                    out.push('}');
                }
                _ => {
                    out.push(c);
                    self.bump();
                }
            }
        }
        Some(out)
    }

    /// Read a balanced `[...]` optional-argument group.
    fn read_bracket_group(&mut self) -> Option<String> {
        if !self.eat('[') {
            return None;
        }
        let mut depth = 1usize;
        let mut out = String::new();
        while let Some(c) = self.peek() {
            match c {
                '\\' => {
                    out.push('\\');
                    self.bump();
                    if let Some(next) = self.bump() {
                        out.push(next);
                    }
                }
                '[' => {
                    depth += 1;
                    out.push('[');
                    self.bump();
                }
                ']' => {
                    depth -= 1;
                    self.bump();
                    if depth == 0 {
                        break;
                    }
                    out.push(']');
                }
                _ => {
                    out.push(c);
                    self.bump();
                }
            }
        }
        Some(out)
    }

    fn read_dollar_math(&mut self, tokens: &mut Vec<Token>) {
        self.bump(); // consume the opening `$`
        let display = self.eat('$');
        tokens.push(Token::BeginMath);
        if display {
            self.skip_to_display_close();
        } else {
            self.skip_to_inline_dollar();
        }
        tokens.push(Token::EndMath);
    }

    /// Advance past a `$$` close (or to end of input).
    fn skip_to_display_close(&mut self) {
        if let Some(idx) = self.src[self.pos..].find("$$") {
            self.pos += idx + 2;
        } else {
            self.pos = self.src.len();
        }
    }

    /// Advance past a `\X` close such as `\]` or `\)` (or to end of input).
    fn skip_to_control_close(&mut self, close: char) {
        let needle = format!("\\{}", close);
        if let Some(idx) = self.src[self.pos..].find(&needle) {
            self.pos += idx + needle.len();
        } else {
            self.pos = self.src.len();
        }
    }

    /// Advance past the closing inline `$`, honoring escaped `\$`.
    fn skip_to_inline_dollar(&mut self) {
        let bytes = self.src.as_bytes();
        let mut i = self.pos;
        let mut backslashes = 0usize;
        while i < bytes.len() {
            match bytes[i] {
                b'\\' => {
                    backslashes += 1;
                    i += 1;
                }
                b'$' => {
                    if backslashes % 2 == 0 {
                        self.pos = i + 1;
                        return;
                    }
                    backslashes = 0;
                    i += 1;
                }
                _ => {
                    backslashes = 0;
                    i += 1;
                }
            }
        }
        self.pos = self.src.len();
    }

    /// Advance past the matching `\end{env}` terminator (or to end of input,
    /// closing the region synthetically at EOF).
    fn skip_to_env_end(&mut self, env: &str) {
        let terminator = format!("\\end{{{}}}", env);
        if let Some(idx) = self.src[self.pos..].find(&terminator) {
            self.pos += idx + terminator.len();
        } else {
            self.pos = self.src.len();
        }
    }
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
}
