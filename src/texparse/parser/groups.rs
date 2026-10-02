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
        // Capped loop: see `Parser::run` for why the bound is behaviour-neutral.
        for _ in 0..self.src.len() {
            let Some(c) = self.peek() else {
                break;
            };
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
        // Capped loop: see `Parser::run` for why the bound is behaviour-neutral.
        for _ in 0..self.src.len() {
            let Some(c) = self.peek() else {
                break;
            };
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

#[cfg(test)]
mod tests {
    use super::Parser;
    use crate::texparse::{tokenize, Token};

    /// The spanned reader reports the inner content's extent: it starts just
    /// after the opening `{` and ends on the `}` byte, never past it.
    #[test]
    fn braced_group_span_reports_the_inner_extent() {
        let mut parser = Parser::new("{abc}");
        assert_eq!(
            parser.read_braced_group_spanned(),
            Some(("abc".to_string(), 1, 4))
        );
    }

    /// `\{` and `\}` are literal braces: they must not change nesting, so the
    /// group closes at the real `}` and the text after it stays outside.
    #[test]
    fn escaped_braces_do_not_change_group_nesting() {
        let tokens = tokenize(r"\textbf{a \{ b} rest");
        assert_eq!(
            tokens,
            vec![
                Token::Command {
                    name: "textbf".to_string(),
                    args: vec![],
                },
                Token::Text("a ".to_string()),
                Token::Command {
                    name: "{".to_string(),
                    args: Vec::new(),
                },
                Token::Text(" b".to_string()),
                Token::Text(" rest".to_string()),
            ]
        );
    }

    /// An escaped `\]` inside an optional argument is literal: the bracket
    /// group must swallow it instead of closing on it, so the `{x}` after it
    /// is still read as the next argument.
    #[test]
    fn escaped_bracket_does_not_close_the_optional_argument() {
        let tokens = tokenize(r"\label[a\]b]{x}");
        assert_eq!(
            tokens,
            vec![Token::Command {
                name: "label".to_string(),
                args: vec!["a\\]b".to_string(), "x".to_string()],
            }]
        );
    }

    /// Nested `[...]` inside an optional argument increments the depth, so
    /// only the outermost `]` closes the group.
    #[test]
    fn nested_brackets_stay_inside_one_optional_argument() {
        let tokens = tokenize(r"\label[a[b]c]{x}");
        assert_eq!(
            tokens,
            vec![Token::Command {
                name: "label".to_string(),
                args: vec!["a[b]c".to_string(), "x".to_string()],
            }]
        );
    }
}
