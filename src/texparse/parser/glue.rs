//! Unbraced TeX glue specification reader for spacing commands.
//!
//! Consumes a single glue value (`1.2cm`, `2em plus 1fill minus 0.5em`,
//! `\baselineskip`, `2\baselineskip`) and returns it as the raw source
//! string so the tokenizer can record it as a command argument instead of
//! leaking its parts into prose.

use super::Parser;

const UNITS: &[&str] = &[
    "pt", "bp", "mm", "cm", "in", "ex", "em", "mu", "sp", "dd", "cc", "nd", "nc", "pc", "px",
];

const FIL_UNITS: &[&str] = &["filll", "fill", "fil"];

impl<'a> Parser<'a> {
    /// Consume one TeX glue specification after a glue-taking command
    /// (`\vskip`, `\hskip`, `\kern`, `\vglue`, `\hglue`).
    ///
    /// Returns the consumed source text, or `None` when no glue follows (the
    /// caller then emits the command with no argument). `plus`/`minus` are
    /// recognised only immediately after a consumed dimension.
    pub(super) fn read_glue(&mut self) -> Option<String> {
        let start = self.pos;
        self.skip_glue_whitespace();
        if !self.read_dimension() {
            self.pos = start;
            return None;
        }
        // Capped loop: at most two stretch/shrink parts can be consumed.
        for _ in 0..2 {
            let before = self.pos;
            self.read_stretch_shrink("plus");
            self.read_stretch_shrink("minus");
            if self.pos == before {
                break;
            }
        }
        Some(self.src[start..self.pos].to_string())
    }

    fn skip_glue_whitespace(&mut self) {
        // Capped loop: each iteration consumes one whitespace char (see
        // `Parser::run` for why the bound is behaviour-neutral).
        for _ in 0..self.src.len() {
            match self.peek() {
                Some(c) if c.is_whitespace() => {
                    self.bump();
                }
                _ => break,
            }
        }
    }

    /// Read a dimension: `[+-]? (number unit | \cs | number \cs)`.
    fn read_dimension(&mut self) -> bool {
        let start = self.pos;
        self.skip_glue_whitespace();
        let _ = self.read_sign();
        self.skip_glue_whitespace();

        if self.read_control_sequence() {
            return true;
        }

        if !self.read_number() {
            self.pos = start;
            return false;
        }
        self.skip_glue_whitespace();
        if self.read_unit() || self.read_control_sequence() {
            return true;
        }

        self.pos = start;
        false
    }

    /// Read an optional `plus` or `minus` stretch/shrink part. The keyword is
    /// consumed only when a full stretch value follows; otherwise the position
    /// is restored so bare `plus`/`minus` stays prose.
    fn read_stretch_shrink(&mut self, keyword: &str) {
        let start = self.pos;
        self.skip_glue_whitespace();
        if !self.read_keyword(keyword) {
            self.pos = start;
            return;
        }
        self.skip_glue_whitespace();
        let _ = self.read_sign();
        self.skip_glue_whitespace();
        if self.read_stretch_value() {
            return;
        }
        self.pos = start;
    }

    /// Read the value of a stretch/shrink part: number + (unit | fil | \cs).
    fn read_stretch_value(&mut self) -> bool {
        let start = self.pos;
        if !self.read_number() {
            self.pos = start;
            return false;
        }
        self.skip_glue_whitespace();
        if self.read_fil_unit() || self.read_unit() || self.read_control_sequence() {
            return true;
        }
        self.pos = start;
        false
    }

    fn read_sign(&mut self) -> bool {
        matches!(self.peek(), Some('+' | '-')) && self.bump().is_some()
    }

    /// Read a number: digits with an optional `.` or `,` decimal part. Also
    /// accepts a leading decimal point/comma followed by digits.
    fn read_number(&mut self) -> bool {
        let start = self.pos;
        let mut saw_digit = false;
        for _ in 0..self.src.len() {
            match self.peek() {
                Some(c) if c.is_ascii_digit() => {
                    saw_digit = true;
                    self.bump();
                }
                _ => break,
            }
        }
        if matches!(self.peek(), Some('.' | ',')) {
            self.bump();
            for _ in 0..self.src.len() {
                match self.peek() {
                    Some(c) if c.is_ascii_digit() => {
                        saw_digit = true;
                        self.bump();
                    }
                    _ => break,
                }
            }
        }
        if saw_digit {
            true
        } else {
            self.pos = start;
            false
        }
    }

    fn read_unit(&mut self) -> bool {
        let start = self.pos;
        if self.read_keyword("true") {
            self.skip_glue_whitespace();
            if !self.read_unit_name(UNITS) {
                self.pos = start;
                return false;
            }
            return true;
        }
        self.read_unit_name(UNITS)
    }

    fn read_fil_unit(&mut self) -> bool {
        self.read_unit_name(FIL_UNITS)
    }

    /// Read one of the given unit names. A unit followed by ASCII letters is
    /// rejected unless the letters start a glue keyword (`plus`/`minus`), so
    /// `1cm plus 1fil` and `1cmplus1fil` both match while `1inches` does not.
    fn read_unit_name(&mut self, units: &[&str]) -> bool {
        let rest = &self.src[self.pos..];
        for unit in units {
            let Some(tail) = rest.strip_prefix(unit) else {
                continue;
            };
            if tail.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
                && !tail.starts_with("plus")
                && !tail.starts_with("minus")
            {
                continue;
            }
            self.pos += unit.len();
            return true;
        }
        false
    }

    fn read_keyword(&mut self, keyword: &str) -> bool {
        let rest = &self.src[self.pos..];
        let Some(tail) = rest.strip_prefix(keyword) else {
            return false;
        };
        if tail.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) {
            return false;
        }
        self.pos += keyword.len();
        true
    }

    /// Read a control sequence (`\name` or `\x`) using the same command-char
    /// rule as the tokenizer.
    fn read_control_sequence(&mut self) -> bool {
        if !self.eat('\\') {
            return false;
        }
        match self.peek() {
            Some(c) if super::super::is_command_char(c) => {
                for _ in 0..self.src.len() {
                    match self.peek() {
                        Some(ch) if super::super::is_command_char(ch) => {
                            self.bump();
                        }
                        _ => break,
                    }
                }
                true
            }
            Some(_) => {
                self.bump();
                true
            }
            None => true,
        }
    }
}
