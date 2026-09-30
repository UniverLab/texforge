//! Section-title resolution: macros stripped, prose kept, escapes decoded.

/// Macros that contribute no text to a resolved section title: pure spacing
/// or rule constructs, typically used to build dot leaders.
const TITLE_VISUAL_MACROS: &[&str] = &["leaders", "hfill", "hbox"];

/// Macros whose braced argument is the title text itself: everything but the
/// last argument is dropped (a URL, a color name, ...), and the kept argument
/// is resolved recursively so nesting works.
const TITLE_TEXT_MACROS: &[&str] = &["textit", "textbf", "emph"];

/// The standard LaTeX escaped specials: `\&`, `\%`, `\$`, `\#`, `\_`, `\{`,
/// `\}` each resolve to their literal character in a title. A backslash
/// followed by anything else still introduces a command.
const ESCAPED_SPECIALS: &[char] = &['&', '%', '$', '#', '_', '{', '}'];

/// Resolves a raw section title — the literal source text of a `\section`-like
/// command's braced argument — into human-readable prose.
///
/// This is the single place that understands macro-wrapped titles (`\href`,
/// `\textit`, `\textcolor`, dot-leader constructs, ...), so `outline` and the
/// PDF page mapper both see the same resolved string instead of drifting
/// title cleaners. See the module's [DECISIONS] in the originating spec for
/// the resolution rule per macro class.
///
/// The resolver never panics: unbalanced braces yield a best-effort string
/// built from whatever was scanned before the input ran out.
pub(crate) fn resolve_section_title(raw: &str) -> String {
    let chars: Vec<char> = raw.chars().collect();
    let resolved = TitleResolver::new(&chars).resolve();
    resolved.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Recursive-descent scanner over a title's raw source, mirroring the
/// tokenizer's own brace handling but standalone: it only needs to tell
/// prose apart from the handful of macros a title can be wrapped in.
struct TitleResolver<'a> {
    chars: &'a [char],
    pos: usize,
}

impl<'a> TitleResolver<'a> {
    fn new(chars: &'a [char]) -> Self {
        Self { chars, pos: 0 }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek();
        if c.is_some() {
            self.pos += 1;
        }
        c
    }

    /// Reads a balanced `{...}` group's inner content. On unbalanced input,
    /// returns everything scanned up to the end of input (best effort).
    fn read_group(&mut self) -> Option<Vec<char>> {
        if self.peek() != Some('{') {
            return None;
        }
        self.bump();
        let mut depth = 1usize;
        let mut out = Vec::new();
        while let Some(c) = self.peek() {
            match c {
                '\\' => {
                    out.push(c);
                    self.bump();
                    if let Some(next) = self.bump() {
                        out.push(next);
                    }
                }
                '{' => {
                    depth += 1;
                    out.push(c);
                    self.bump();
                }
                '}' => {
                    self.bump();
                    depth -= 1;
                    if depth == 0 {
                        return Some(out);
                    }
                    out.push(c);
                }
                _ => {
                    out.push(c);
                    self.bump();
                }
            }
        }
        Some(out)
    }

    fn read_command_name(&mut self) -> String {
        let mut name = String::new();
        match self.peek() {
            Some(c) if c.is_ascii_alphabetic() => {
                while let Some(c) = self.peek() {
                    if c.is_ascii_alphabetic() {
                        name.push(c);
                        self.bump();
                    } else {
                        break;
                    }
                }
            }
            Some(c) => {
                name.push(c);
                self.bump();
            }
            None => {}
        }
        name
    }

    fn resolve(&mut self) -> String {
        let mut out = String::new();
        while let Some(c) = self.peek() {
            match c {
                '\\' => self.resolve_backslash_macro(&mut out),
                '{' => self.resolve_bare_group(&mut out),
                '}' => {
                    // Stray closing brace from malformed input: skip it.
                    self.bump();
                }
                _ => {
                    out.push(c);
                    self.bump();
                }
            }
        }
        out
    }

    /// A `\...` sequence inside a title: escaped special, visual macro,
    /// text macro, two-group macro, or unknown macro.
    fn resolve_backslash_macro(&mut self, out: &mut String) {
        self.bump();
        if let Some(special) = self.peek().filter(|c| ESCAPED_SPECIALS.contains(c)) {
            // Escaped special: the backslash just protects the
            // character, it does not introduce a command.
            self.bump();
            out.push(special);
            return;
        }
        let name = self.read_command_name();
        if TITLE_VISUAL_MACROS.contains(&name.as_str()) {
            // Purely visual: drop the macro and, if present, the
            // one group it dresses (e.g. `\hbox{.}`).
            let _ = self.read_group();
        } else if TITLE_TEXT_MACROS.contains(&name.as_str()) {
            self.resolve_text_macro(out);
        } else if name == "textcolor" || name == "href" {
            self.resolve_textcolor_or_href(out);
        } else {
            self.resolve_unknown_macro(out);
        }
    }

    /// A text macro (`\textit`, `\textbf`, `\emph`): resolve its group.
    fn resolve_text_macro(&mut self, out: &mut String) {
        let Some(group) = self.read_group() else {
            return;
        };
        out.push_str(&TitleResolver::new(&group).resolve());
    }

    /// `\textcolor{color}{text}` and `\href{url}{text}`: drop the first
    /// (non-textual) group, keep the second.
    fn resolve_textcolor_or_href(&mut self, out: &mut String) {
        let _dropped = self.read_group();
        let Some(text) = self.read_group() else {
            return;
        };
        out.push_str(&TitleResolver::new(&text).resolve());
    }

    /// Unknown macro: keep its textual content rather than dropping the
    /// title or emitting raw source. With no braced argument, drop silently.
    fn resolve_unknown_macro(&mut self, out: &mut String) {
        let Some(group) = self.read_group() else {
            return;
        };
        out.push_str(&TitleResolver::new(&group).resolve());
    }

    /// A bare group not attached to a command still groups prose
    /// (`{Emphasis}`); keep its resolved content.
    fn resolve_bare_group(&mut self, out: &mut String) {
        let Some(group) = self.read_group() else {
            return;
        };
        out.push_str(&TitleResolver::new(&group).resolve());
    }
}

#[cfg(test)]
mod tests {
    use super::super::{tokenize, Token};
    use super::*;

    #[test]
    fn plain_title_is_unchanged() {
        assert_eq!(resolve_section_title("Introduction"), "Introduction");
    }

    #[test]
    fn href_title_keeps_link_text_not_url() {
        assert_eq!(
            resolve_section_title(r"\href{https://univerlab.org}{UniverLab.org}"),
            "UniverLab.org"
        );
    }

    #[test]
    fn textit_and_textbf_keep_their_argument() {
        assert_eq!(resolve_section_title(r"\textit{Hello}"), "Hello");
        assert_eq!(resolve_section_title(r"\textbf{Hello}"), "Hello");
    }

    #[test]
    fn textcolor_drops_color_keeps_text() {
        assert_eq!(
            resolve_section_title(r"\textcolor{lightgray}{Hello}"),
            "Hello"
        );
    }

    #[test]
    fn visual_macros_are_dropped_entirely() {
        assert_eq!(resolve_section_title(r"\leaders\hbox{.}\hfill"), "");
        assert_eq!(
            resolve_section_title(r"Before \leaders\hbox{.}\hfill{}After"),
            "Before After"
        );
    }

    #[test]
    fn unknown_macro_keeps_textual_content() {
        assert_eq!(resolve_section_title(r"\foo{bar}"), "bar");
    }

    #[test]
    fn nested_macros_resolve_to_innermost_text() {
        assert_eq!(resolve_section_title(r"\textbf{\href{u}{X}}"), "X");
    }

    #[test]
    fn whitespace_and_newlines_collapse_to_single_spaces() {
        assert_eq!(
            resolve_section_title("AI Engineer en Accenture\n\\textcolor{lightgray}{\\leaders\\hbox{.}\\hfill}\n\\textit{Julio 2026 -- Actual}"),
            "AI Engineer en Accenture Julio 2026 -- Actual"
        );
    }

    #[test]
    fn title_resolution_survives_unbalanced_braces() {
        assert_eq!(resolve_section_title(r"\textit{Hello"), "Hello");
        assert_eq!(resolve_section_title("Hello}"), "Hello");
    }

    #[test]
    fn evidence_heading_resolves_via_tokenize() {
        let tokens = tokenize(r"\section{\href{https://univerlab.org}{UniverLab.org}}");
        assert_eq!(
            tokens,
            vec![Token::Section {
                level: 2,
                title: "UniverLab.org".to_string(),
                raw_title: r"\href{https://univerlab.org}{UniverLab.org}".to_string(),
            }]
        );
    }

    #[test]
    fn escaped_ampersand_resolves_to_literal_character() {
        assert_eq!(resolve_section_title(r"\&"), "&");
    }

    #[test]
    fn escaped_percent_resolves_to_literal_character() {
        assert_eq!(resolve_section_title(r"\%"), "%");
    }

    #[test]
    fn escaped_dollar_resolves_to_literal_character() {
        assert_eq!(resolve_section_title(r"\$"), "$");
    }

    #[test]
    fn escaped_hash_resolves_to_literal_character() {
        assert_eq!(resolve_section_title(r"\#"), "#");
    }

    #[test]
    fn escaped_underscore_resolves_to_literal_character() {
        assert_eq!(resolve_section_title(r"\_"), "_");
    }

    #[test]
    fn escaped_open_brace_resolves_to_literal_character() {
        assert_eq!(resolve_section_title(r"\{"), "{");
    }

    #[test]
    fn escaped_close_brace_resolves_to_literal_character() {
        assert_eq!(resolve_section_title(r"\}"), "}");
    }

    #[test]
    fn escaped_specials_survive_inside_wrapping_macros() {
        assert_eq!(
            resolve_section_title(r"\textit{Fundador \& Lead Engineer}"),
            "Fundador & Lead Engineer"
        );
    }

    #[test]
    fn evidence_escaped_ampersand_resolves_via_tokenize() {
        let tokens = tokenize(r"\subsection*{\textit{Fundador \& Lead Engineer}}");
        assert_eq!(
            tokens,
            vec![Token::Section {
                level: 3,
                title: "Fundador & Lead Engineer".to_string(),
                raw_title: r"\textit{Fundador \& Lead Engineer}".to_string(),
            }]
        );
    }
}
