//! The single-pass parser behind [`super::tokenize`] and
//! [`super::tokenize_with_spans`]: the scanner loop plus backslash and
//! command dispatch.

mod commands;
mod groups;
mod math;

use super::sections::section_level;
use super::title::resolve_section_title;
use super::{is_command_char, SpannedToken, Token, TokenizedSource, PROSE_COMMANDS};

/// Single-pass state machine over the character stream.
pub(super) struct Parser<'a> {
    src: &'a str,
    pos: usize,
    /// Byte offsets of `$`/`$$` math delimiters that were never closed.
    unclosed_math: Vec<usize>,
}

impl<'a> Parser<'a> {
    pub(super) fn new(src: &'a str) -> Self {
        Parser {
            src,
            pos: 0,
            unclosed_math: Vec::new(),
        }
    }

    pub(super) fn peek(&self) -> Option<char> {
        self.src[self.pos..].chars().next()
    }

    pub(super) fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += c.len_utf8();
        Some(c)
    }

    pub(super) fn eat(&mut self, expected: char) -> bool {
        match self.peek() {
            Some(c) if c == expected => {
                self.bump();
                true
            }
            // EOF (or a mismatch) never consumes: every caller's argument
            // loop therefore terminates at end of input.
            _ => false,
        }
    }

    pub(super) fn push(
        &self,
        tokens: &mut Vec<SpannedToken>,
        token: Token,
        start: usize,
        end: usize,
    ) {
        tokens.push(SpannedToken { token, start, end });
    }

    pub(super) fn run(mut self) -> TokenizedSource {
        let mut tokens = Vec::new();
        let mut text = String::new();
        let mut text_start = 0usize;

        // Bounded scan: every arm consumes at least one byte, so a correct
        // parse always reaches EOF within `src.len()` iterations and the cap
        // never truncates; a regressed `peek`/`bump` can no longer spin
        // forever — it emits garbage the tests then reject.
        for _ in 0..self.src.len() {
            let Some(c) = self.peek() else {
                break;
            };
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

    pub(super) fn flush_text(
        &self,
        tokens: &mut Vec<SpannedToken>,
        text: &mut String,
        start: usize,
    ) {
        if !text.is_empty() {
            let end = self.pos;
            self.push(tokens, Token::Text(std::mem::take(text)), start, end);
        }
    }

    /// Read a comment: `%` through end of line (the newline is not consumed).
    pub(super) fn read_comment(&mut self) -> String {
        let start = self.pos;
        // Capped loop: a comment is at most the rest of the buffer, so a
        // correct pass never reaches the bound (see `Parser::run`).
        for _ in 0..self.src.len() {
            let Some(c) = self.peek() else {
                break;
            };
            if c == '\n' {
                break;
            }
            self.bump();
        }
        self.src[start..self.pos].to_string()
    }

    pub(super) fn handle_backslash(&mut self, tokens: &mut Vec<SpannedToken>) {
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

    pub(super) fn handle_command(
        &mut self,
        name: &str,
        start: usize,
        tokens: &mut Vec<SpannedToken>,
    ) {
        match name {
            "begin" => self.handle_begin(start, tokens),
            "end" => self.handle_end(start, tokens),
            "verb" | "lstinline" => self.handle_verb_command(name, start, tokens),
            _ => {
                if let Some(level) = section_level(name) {
                    self.eat('*');
                    let _ = self.read_bracket_group();
                    let raw_title = self.read_braced_group().unwrap_or_default();
                    let title = resolve_section_title(&raw_title);
                    self.push(
                        tokens,
                        Token::Section {
                            level,
                            title,
                            raw_title,
                        },
                        start,
                        self.pos,
                    );
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
}
