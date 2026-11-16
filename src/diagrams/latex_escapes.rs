//! Translate LaTeX accent escapes in a diagram source to UTF-8.
//!
//! Mermaid, D2 and Graphviz do not read LaTeX, so an author writing
//! `M\'etodos` inside a diagram block gets a diagram labelled `M\'etodos`:
//! the backslash is rendered. The escape is normal in the surrounding
//! document, so [`check`](crate::linter) does not flag it; the pre-processor
//! translates it once, before the source reaches any renderer.
//!
//! Only escapes that compose to a single Unicode character are rewritten.
//! Everything else — Graphviz `\n`/`\l`/`\r` line controls, unknown commands,
//! a backslash that starts no accent — is passed through unchanged. Letter
//! form accents (`\c`, `\v`, `\u`, `\H`, `\r`, `\k`) are only recognised with
//! a braced argument, which keeps a Graphviz `\r` line-justification marker
//! from being mistaken for a ring accent.

use crate::texutil::compose_accent;

/// Rewrite LaTeX accent escapes in `src` as the UTF-8 characters they stand
/// for. Already-UTF-8 text and unrecognised escapes come back untouched.
pub(crate) fn translate_latex_escapes(src: &str) -> String {
    if !src.contains('\\') {
        return src.to_string();
    }

    let mut out = String::with_capacity(src.len());
    let mut rest = src;
    while let Some(c) = rest.chars().next() {
        if c == '{' {
            if let Some((composed, used)) = braced_escape(&rest[1..]) {
                out.push(composed);
                rest = &rest[used + 1..];
                continue;
            }
        }
        if c == '\\' {
            if let Some((replacement, used)) = escaped(&rest[1..]) {
                out.extend(replacement);
                rest = &rest[used + 1..];
                continue;
            }
        }
        out.push(c);
        rest = &rest[c.len_utf8()..];
    }
    out
}

/// Translate one escape, given the text just after its backslash. Returns the
/// replacement (`None` for a command that renders no glyph, like `\-`) and
/// how many bytes after the backslash it consumed. `None` means "not an
/// escape this pass knows": the backslash is kept as written.
fn escaped(after: &str) -> Option<(Option<char>, usize)> {
    let accent = after.chars().next()?;
    if accent == '-' {
        return Some((None, 1));
    }
    if matches!(accent, '&' | '%' | '_') {
        return Some((Some(accent), 1));
    }

    let (base, base_len) = if is_symbol_accent(accent) {
        symbol_base(&after[1..])?
    } else if is_letter_accent(accent) {
        braced_base(&after[1..])?
    } else {
        return None;
    };

    let composed = compose_accent(accent, base)?;
    Some((Some(composed), 1 + base_len))
}

/// A whole group around an escape (`{\'e}`, `{\'{e}}`) collapses to the
/// composed character: the braces only group the accent, so all three brace
/// forms produce the same text — and therefore the same cached image.
fn braced_escape(inside: &str) -> Option<(char, usize)> {
    let (replacement, used) = escaped(inside.strip_prefix('\\')?)?;
    let composed = replacement?;
    let consumed = 1 + used;
    inside[consumed..]
        .starts_with('}')
        .then_some((composed, consumed + 1))
}

fn is_symbol_accent(accent: char) -> bool {
    matches!(accent, '\'' | '`' | '^' | '"' | '~' | '=' | '.')
}

fn is_letter_accent(accent: char) -> bool {
    matches!(accent, 'c' | 'v' | 'u' | 'H' | 'r' | 'k')
}

/// Base of a symbol-form accent: `e`, `{e}`, or the spaced `\' e` form LaTeX
/// also accepts. Leading spaces/tabs are skipped, never newlines, so an
/// escape at the end of a line cannot reach across it.
fn symbol_base(s: &str) -> Option<(char, usize)> {
    let trimmed = s.trim_start_matches([' ', '\t']);
    let skipped = s.len() - trimmed.len();
    if let Some(inner) = trimmed.strip_prefix('{') {
        let (base, used) = single_letter(inner)?;
        return inner[used..]
            .starts_with('}')
            .then_some((base, skipped + 1 + used + 1));
    }
    let base = trimmed.chars().next()?;
    base.is_ascii_alphabetic()
        .then_some((base, skipped + base.len_utf8()))
}

/// Base of a letter-form accent: only the braced `\c{c}` spelling, which is
/// what LaTeX users write for these and what keeps Graphviz's one-letter
/// line controls out of the accent table.
fn braced_base(s: &str) -> Option<(char, usize)> {
    let inner = s.strip_prefix('{')?;
    let (base, used) = single_letter(inner)?;
    if !inner[used..].starts_with('}') {
        return None;
    }
    Some((base, 1 + used + 1))
}

fn single_letter(s: &str) -> Option<(char, usize)> {
    let base = s.chars().next()?;
    base.is_ascii_alphabetic()
        .then_some((base, base.len_utf8()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translates_unbraced_symbol_accents() {
        assert_eq!(translate_latex_escapes("M\\'etodos"), "Métodos");
        assert_eq!(translate_latex_escapes("Evaluaci\\'on"), "Evaluación");
        assert_eq!(translate_latex_escapes("caf\\`e"), "cafè");
        assert_eq!(translate_latex_escapes("n\\^ucleo"), "nûcleo");
        assert_eq!(translate_latex_escapes("\\\"uber"), "über");
        assert_eq!(translate_latex_escapes("a\\~no"), "año");
    }

    #[test]
    fn translates_braced_symbol_accents() {
        assert_eq!(translate_latex_escapes("Evaluaci\\'{o}n"), "Evaluación");
        assert_eq!(translate_latex_escapes("a\\~{n}o"), "año");
    }

    #[test]
    fn translates_letter_form_accents_with_a_braced_argument() {
        assert_eq!(translate_latex_escapes("fran\\c{c}ais"), "français");
        assert_eq!(translate_latex_escapes("\\v{s}koda"), "škoda");
        assert_eq!(translate_latex_escapes("\\H{o}"), "ő");
        assert_eq!(translate_latex_escapes("\\k{a}"), "ą");
    }

    #[test]
    fn collapses_a_group_wrapped_accent() {
        assert_eq!(translate_latex_escapes("M{\\'e}todos"), "Métodos");
        assert_eq!(translate_latex_escapes("M{\\'{e}}todos"), "Métodos");
        assert_eq!(translate_latex_escapes("{\\c{c}}"), "ç");
    }

    #[test]
    fn strips_discretionary_hyphens() {
        assert_eq!(translate_latex_escapes("impor\\-tancia"), "importancia");
    }

    #[test]
    fn unescapes_literal_specials() {
        assert_eq!(translate_latex_escapes("A \\& B"), "A & B");
        assert_eq!(translate_latex_escapes("50\\%"), "50%");
        assert_eq!(translate_latex_escapes("\\_x"), "_x");
    }

    #[test]
    fn leaves_graphviz_line_controls_alone() {
        assert_eq!(
            translate_latex_escapes("one\\ntwo\\lthree\\rfour"),
            "one\\ntwo\\lthree\\rfour"
        );
    }

    #[test]
    fn leaves_unknown_escapes_and_non_composable_bases_alone() {
        assert_eq!(translate_latex_escapes("\\unknownmacro"), "\\unknownmacro");
        assert_eq!(translate_latex_escapes("\\'x"), "\\'x");
        assert_eq!(translate_latex_escapes("\\c c"), "\\c c");
        assert_eq!(translate_latex_escapes("\\'1"), "\\'1");
    }

    #[test]
    fn leaves_utf8_text_untouched() {
        assert_eq!(
            translate_latex_escapes("Métodos, Evaluación"),
            "Métodos, Evaluación"
        );
        assert_eq!(translate_latex_escapes("plain text"), "plain text");
    }

    #[test]
    fn leaves_non_escape_braces_untouched() {
        assert_eq!(translate_latex_escapes("A{?}"), "A{?}");
        assert_eq!(
            translate_latex_escapes("{not an escape}"),
            "{not an escape}"
        );
    }
}
