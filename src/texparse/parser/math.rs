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
    pub(super) fn skip_to_env_end(&mut self, env: &str) {
        let terminator = format!("\\end{{{}}}", env);
        if let Some(idx) = self.src[self.pos..].find(&terminator) {
            self.pos += idx + terminator.len();
        } else {
            self.pos = self.src.len();
        }
    }
}
