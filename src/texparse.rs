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

/// Produces dotted section numbers such as `1`, `1.1` and `2.1.1`.
///
/// One counter per level, mirroring the tokenizer's section hierarchy
/// ([`section_level`]: `part` = 0, `chapter` = 1, `section` = 2, ...). Entering
/// a level bumps its counter and resets every deeper one. Leading zero counters
/// are dropped from the printed number, so a document that only uses
/// `\section` numbers its sections `1`, `2`, ... rather than `0.0.1`.
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

/// Single-pass state machine over the character stream.
struct Parser<'a> {
    src: &'a str,
    pos: usize,
    /// Byte offsets of `$`/`$$` math delimiters that were never closed.
    unclosed_math: Vec<usize>,
}

impl<'a> Parser<'a> {
    fn new(src: &'a str) -> Self {
        Parser {
            src,
            pos: 0,
            unclosed_math: Vec::new(),
        }
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

    fn push(&self, tokens: &mut Vec<SpannedToken>, token: Token, start: usize, end: usize) {
        tokens.push(SpannedToken { token, start, end });
    }

    fn run(mut self) -> TokenizedSource {
        let mut tokens = Vec::new();
        let mut text = String::new();
        let mut text_start = 0usize;

        while let Some(c) = self.peek() {
            match c {
                '%' => {
                    self.flush_text(&mut tokens, &mut text, text_start);
                    let start = self.pos;
                    let comment = self.read_comment();
                    self.push(&mut tokens, Token::Comment(comment), start, self.pos);
                }
                '\\' => {
                    self.flush_text(&mut tokens, &mut text, text_start);
                    self.handle_backslash(&mut tokens);
                }
                '$' => {
                    self.flush_text(&mut tokens, &mut text, text_start);
                    self.read_dollar_math(&mut tokens);
                }
                _ => {
                    if text.is_empty() {
                        text_start = self.pos;
                    }
                    text.push(c);
                    self.bump();
                }
            }
        }
        self.flush_text(&mut tokens, &mut text, text_start);
        TokenizedSource {
            tokens,
            unclosed_math: self.unclosed_math,
        }
    }

    fn flush_text(&self, tokens: &mut Vec<SpannedToken>, text: &mut String, start: usize) {
        if !text.is_empty() {
            let end = self.pos;
            self.push(tokens, Token::Text(std::mem::take(text)), start, end);
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

    fn handle_backslash(&mut self, tokens: &mut Vec<SpannedToken>) {
        let start = self.pos;
        self.bump(); // consume the backslash
        let Some(c) = self.peek() else {
            self.push(
                tokens,
                Token::Command {
                    name: "\\".to_string(),
                    args: Vec::new(),
                },
                start,
                self.pos,
            );
            return;
        };

        match c {
            '[' => {
                self.bump();
                self.push(tokens, Token::BeginMath, start, start + 2);
                let close_start = self.pos;
                self.skip_to_control_close(']');
                self.push(tokens, Token::EndMath, close_start, close_start + 2);
            }
            ']' => {
                self.bump();
                self.push(tokens, Token::EndMath, start, start + 2);
            }
            '(' => {
                self.bump();
                self.push(tokens, Token::BeginMath, start, start + 2);
                let close_start = self.pos;
                self.skip_to_control_close(')');
                self.push(tokens, Token::EndMath, close_start, close_start + 2);
            }
            ')' => {
                self.bump();
                self.push(tokens, Token::EndMath, start, start + 2);
            }
            _ if is_command_char(c) => {
                let name = self.read_command_name();
                self.handle_command(&name, start, tokens);
            }
            _ => {
                // Control symbol: `\\`, `\$`, `\%`, `\{`, `\&`, ...
                self.bump();
                self.push(
                    tokens,
                    Token::Command {
                        name: c.to_string(),
                        args: Vec::new(),
                    },
                    start,
                    self.pos,
                );
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

    fn handle_command(&mut self, name: &str, start: usize, tokens: &mut Vec<SpannedToken>) {
        match name {
            "begin" => self.handle_begin(start, tokens),
            "end" => self.handle_end(start, tokens),
            "verb" | "lstinline" => self.handle_verb_command(name, start, tokens),
            _ => {
                if let Some(level) = section_level(name) {
                    self.eat('*');
                    let _ = self.read_bracket_group();
                    let title = self.read_braced_group().unwrap_or_default();
                    self.push(tokens, Token::Section { level, title }, start, self.pos);
                } else if name == "href" {
                    self.handle_href(start, tokens);
                } else if PROSE_COMMANDS.contains(&name) {
                    self.handle_prose_command(name, start, tokens);
                } else {
                    self.handle_plain_command(name, start, tokens);
                }
            }
        }
    }

    /// `\verb[char][...]` and `\lstinline[opts][char][...]`: the delimiter is
    /// the first non-letter after the name (and any options); everything up to
    /// the next occurrence of that delimiter is verbatim and never prose.
    fn handle_verb_command(&mut self, name: &str, start: usize, tokens: &mut Vec<SpannedToken>) {
        let mut args = Vec::new();
        if name == "lstinline" {
            while let Some(optional) = self.read_bracket_group() {
                args.push(optional);
            }
        }
        if self.peek() == Some('*') {
            self.bump();
        }
        let Some(delim) = self.bump() else {
            self.push(
                tokens,
                Token::Command {
                    name: name.to_string(),
                    args,
                },
                start,
                self.pos,
            );
            return;
        };
        let mut content = String::new();
        while let Some(c) = self.peek() {
            if c == delim || c == '\n' {
                break;
            }
            content.push(c);
            self.bump();
        }
        args.push(content);
        self.push(
            tokens,
            Token::Command {
                name: name.to_string(),
                args,
            },
            start,
            self.pos,
        );
    }

    fn handle_begin(&mut self, start: usize, tokens: &mut Vec<SpannedToken>) {
        let env = self.read_braced_group().unwrap_or_default();
        // Discard float placement etc.: `\begin{figure}[htbp]`.
        let _ = self.read_bracket_group();
        let mid = self.pos;

        if env == "document" {
            self.push(tokens, Token::BeginDocument, start, mid);
            return;
        }
        if MATH_ENVIRONMENTS.contains(&env.as_str()) {
            self.push(tokens, Token::BeginMath, start, mid);
            self.skip_to_env_end(&env);
            self.push(tokens, Token::EndMath, mid, self.pos);
            return;
        }
        if VERBATIM_ENVIRONMENTS.contains(&env.as_str()) {
            self.push(
                tokens,
                Token::BeginVerbatim { env: env.clone() },
                start,
                mid,
            );
            self.skip_to_env_end(&env);
            self.push(tokens, Token::EndVerbatim { env }, mid, self.pos);
            return;
        }
        self.push(tokens, Token::Environment { name: env }, start, self.pos);
    }

    fn handle_end(&mut self, start: usize, tokens: &mut Vec<SpannedToken>) {
        let env = self.read_braced_group().unwrap_or_default();
        if env == "document" {
            self.push(tokens, Token::EndDocument, start, self.pos);
        } else if MATH_ENVIRONMENTS.contains(&env.as_str()) {
            self.push(tokens, Token::EndMath, start, self.pos);
        } else if VERBATIM_ENVIRONMENTS.contains(&env.as_str()) {
            self.push(tokens, Token::EndVerbatim { env }, start, self.pos);
        } else {
            self.push(tokens, Token::Environment { name: env }, start, self.pos);
        }
    }

    /// `\href{url}{text}`: the URL is a non-prose argument, the link text is prose.
    fn handle_href(&mut self, start: usize, tokens: &mut Vec<SpannedToken>) {
        let mut args = Vec::new();
        while let Some(optional) = self.read_bracket_group() {
            args.push(optional);
        }
        if let Some(url) = self.read_braced_group() {
            args.push(url);
        }
        self.push(
            tokens,
            Token::Command {
                name: "href".to_string(),
                args,
            },
            start,
            self.pos,
        );
        if let Some((text, text_start, _text_end)) = self.read_braced_group_spanned() {
            let sub = tokenize_with_spans(&text);
            self.unclosed_math
                .extend(sub.unclosed_math.iter().map(|offset| offset + text_start));
            tokens.extend(offset_tokens(sub.tokens, text_start));
        }
    }

    /// A command whose braced argument is prose, emitted as [`Token::Text`].
    fn handle_prose_command(&mut self, name: &str, start: usize, tokens: &mut Vec<SpannedToken>) {
        let mut args = Vec::new();
        while let Some(optional) = self.read_bracket_group() {
            args.push(optional);
        }
        self.push(
            tokens,
            Token::Command {
                name: name.to_string(),
                args,
            },
            start,
            self.pos,
        );
        if let Some((prose, prose_start, _prose_end)) = self.read_braced_group_spanned() {
            let sub = tokenize_with_spans(&prose);
            self.unclosed_math
                .extend(sub.unclosed_math.iter().map(|offset| offset + prose_start));
            tokens.extend(offset_tokens(sub.tokens, prose_start));
        }
    }

    /// Any other command: greedily collect optional and braced arguments.
    fn handle_plain_command(&mut self, name: &str, start: usize, tokens: &mut Vec<SpannedToken>) {
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
        self.push(
            tokens,
            Token::Command {
                name: name.to_string(),
                args,
            },
            start,
            self.pos,
        );
    }

    /// Read a balanced `{...}` group, returning its raw content (escaped
    /// braces `\{`/`\}` do not affect nesting).
    fn read_braced_group(&mut self) -> Option<String> {
        self.read_braced_group_spanned().map(|(text, _, _)| text)
    }

    /// Like [`Parser::read_braced_group`], also returning the byte span of the
    /// inner content so consumers can map re-tokenized prose back to the source.
    fn read_braced_group_spanned(&mut self) -> Option<(String, usize, usize)> {
        if !self.eat('{') {
            return None;
        }
        let content_start = self.pos;
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
                        return Some((out, content_start, self.pos - 1));
                    }
                    out.push('}');
                }
                _ => {
                    out.push(c);
                    self.bump();
                }
            }
        }
        Some((out, content_start, self.pos))
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

    fn read_dollar_math(&mut self, tokens: &mut Vec<SpannedToken>) {
        let start = self.pos; // at the opening `$`
        self.bump(); // consume the opening `$`
        let display = self.eat('$');
        let delim_len = if display { 2 } else { 1 };
        self.push(tokens, Token::BeginMath, start, start + delim_len);
        let closed = if display {
            self.skip_to_display_close()
        } else {
            self.skip_to_inline_dollar()
        };
        if !closed {
            self.unclosed_math.push(start);
        }
        let end = self.pos;
        self.push(tokens, Token::EndMath, end.saturating_sub(delim_len), end);
    }

    /// Advance past a `$$` close (or to end of input). Returns whether a close
    /// was actually found.
    fn skip_to_display_close(&mut self) -> bool {
        if let Some(idx) = self.src[self.pos..].find("$$") {
            self.pos += idx + 2;
            true
        } else {
            self.pos = self.src.len();
            false
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

    /// Advance past the closing inline `$`, honoring escaped `\$`. Returns
    /// whether a close was actually found.
    fn skip_to_inline_dollar(&mut self) -> bool {
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
                        return true;
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
        false
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
}
