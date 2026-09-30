//! Braced and bracketed argument groups.

use super::Parser;

impl<'a> Parser<'a> {
    /// Read a balanced `{...}` group, returning its raw content (escaped
    /// braces `\{`/`\}` do not affect nesting).
    pub(super) fn read_braced_group(&mut self) -> Option<String> {
        self.read_braced_group_spanned().map(|(text, _, _)| text)
    }

    /// Like [`Parser::read_braced_group`], also returning the byte span of the
    /// inner content so consumers can map re-tokenized prose back to the source.
    pub(super) fn read_braced_group_spanned(&mut self) -> Option<(String, usize, usize)> {
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
    pub(super) fn read_bracket_group(&mut self) -> Option<String> {
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
}
