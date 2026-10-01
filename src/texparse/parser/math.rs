//! `$`/`$$`-delimited math, `\[..\]` display math and environment skipping.

use super::{Parser, SpannedToken, Token};

impl<'a> Parser<'a> {
    pub(super) fn read_dollar_math(&mut self, tokens: &mut Vec<SpannedToken>) {
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
    pub(super) fn skip_to_display_close(&mut self) -> bool {
        if let Some(idx) = self.src[self.pos..].find("$$") {
            self.pos += idx + 2;
            true
        } else {
            self.pos = self.src.len();
            false
        }
    }

    /// Advance past a `\X` close such as `\]` or `\)` (or to end of input).
    pub(super) fn skip_to_control_close(&mut self, close: char) {
        let needle = format!("\\{}", close);
        if let Some(idx) = self.src[self.pos..].find(&needle) {
            self.pos += idx + needle.len();
        } else {
            self.pos = self.src.len();
        }
    }

    /// Advance past the closing inline `$`, honoring escaped `\$`. Returns
    /// whether a close was actually found.
    pub(super) fn skip_to_inline_dollar(&mut self) -> bool {
        let bytes = self.src.as_bytes();
        let mut backslashes = 0usize;
        // Index-driven scan: the counter is carried across iterations while
        // the position itself moves with the `for`, so no per-byte arithmetic
        // can leave the scanner parked (or spinning) inside the buffer.
        for (i, &byte) in bytes.iter().enumerate().skip(self.pos) {
            match byte {
                b'\\' => backslashes += 1,
                // An unescaped `$` closes; an escaped `\$` (odd backslash
                // run) falls through with the counter reset, exactly like any
                // other literal byte.
                b'$' if backslashes % 2 == 0 => {
                    self.pos = i + 1;
                    return true;
                }
                _ => backslashes = 0,
            }
        }
        self.pos = self.src.len();
        false
    }

    /// Advance past the matching `\end{env}` terminator (or to end of input,
    /// closing the region synthetically at EOF).
    pub(super) fn skip_to_env_end(&mut self, env: &str) {
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
    use crate::texparse::{tokenize_with_spans, SpannedToken, Token};

    /// An escaped `\$` inside `$...$` is content, not a delimiter: the math
    /// region runs to the *real* closing `$`, so both delimiters keep their
    /// own one-byte spans and nothing is reported as unclosed. A backslash
    /// run that stops being counted would close the region on the escaped
    /// dollar instead, moving every span after it.
    #[test]
    fn escaped_dollar_does_not_close_inline_math() {
        let src = r"$a \$ b$ tail";
        let tokenized = tokenize_with_spans(src);
        assert_eq!(
            tokenized.tokens,
            vec![
                SpannedToken {
                    token: Token::BeginMath,
                    start: 0,
                    end: 1,
                },
                SpannedToken {
                    token: Token::EndMath,
                    start: 7,
                    end: 8,
                },
                SpannedToken {
                    token: Token::Text(" tail".to_string()),
                    start: 8,
                    end: 13,
                },
            ]
        );
        assert_eq!(&src[7..8], "$");
        assert!(tokenized.unclosed_math.is_empty());
    }
}
