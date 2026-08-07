//! Silent-divergence glyph rules (TF9).
//!
//! The most expensive class of LaTeX defect is the construct that compiles
//! cleanly and produces something other than what the author meant: TeX emits
//! no warning because, to TeX, nothing is wrong. The archetype is `~60M COP`
//! in text mode, where `~` is a non-breaking space — not "approximately" —
//! and the figure silently goes from approximate to exact.
//!
//! The six rules below are deliberately narrow. The discriminating heuristics
//! are the whole point: almost every glyph in the table has a legitimate use,
//! so a rule fires only on the pattern that cannot be that legitimate use, and
//! the tokenizer ([`crate::texparse`]) has already stripped math, verbatim,
//! comments and non-prose command arguments from the stream before these rules
//! see any text.
//!
//! | Rule | Fires on | Does NOT fire on | Severity |
//! |---|---|---|---|
//! | `~` as "approximately" | `~60M`, `~5 anos` (adjacent to a digit) | `Figura~\ref{}`, `Dr.~Smith` | [`Severity::Warning`] |
//! | unescaped `%` | `50%` (digit before, unescaped) | `% a real comment` | [`Severity::Warning`] |
//! | literal `...` | three dots in prose | `\ldots`, `\dots` | [`Severity::Warning`] |
//! | straight quotes | `"text"` | `` `` `` / `''` | [`Severity::Warning`] |
//! | hyphen as range | `2020-2024` | `well-written` | [`Severity::Warning`] |
//! | unescaped `&`, `#`, `_`, `$` | outside math and tables | inside math or tables | [`Severity::Error`] |

use crate::texparse::{tokenize_with_spans, Token};

use super::{LintFinding, Severity};

/// Alignment environments where `&` is the column separator rather than prose.
const TABLE_ENVIRONMENTS: &[&str] = &[
    "tabular",
    "tabular*",
    "tabularx",
    "longtable",
    "supertabular",
    "array",
];

/// Run the six glyph rules over one `.tex` file's source.
///
/// `rel` is the project-relative path used in every finding, matching the rest
/// of the linter's output.
pub fn lint_file(rel: &str, source: &str) -> Vec<LintFinding> {
    let tokenized = tokenize_with_spans(source);
    let mut findings = Vec::new();
    let mut in_math = 0usize;
    let mut table_stack: Vec<&str> = Vec::new();
    let mut in_quote = false;

    for spanned in &tokenized.tokens {
        match &spanned.token {
            Token::BeginMath => in_math += 1,
            Token::EndMath => in_math = in_math.saturating_sub(1),
            Token::Environment { name } => {
                if table_stack.last() == Some(&name.as_str()) {
                    table_stack.pop();
                } else if TABLE_ENVIRONMENTS.contains(&name.as_str()) {
                    table_stack.push(name);
                }
            }
            Token::Text(text) => {
                let in_prose = in_math == 0 && table_stack.is_empty();
                check_prose(
                    rel,
                    source,
                    text,
                    spanned.start,
                    in_prose,
                    &mut in_quote,
                    &mut findings,
                );
            }
            Token::Section { title, .. } => {
                check_section_title(rel, source, title, spanned.start, &mut findings);
            }
            Token::Comment(_) => check_percent(rel, source, spanned.start, &mut findings),
            _ => {}
        }
    }

    for offset in &tokenized.unclosed_math {
        findings.push(LintFinding {
            file: rel.to_string(),
            line: line_of(source, *offset),
            severity: Severity::Error,
            message: "Unescaped `$` opens a math region that is never closed — \
                      everything after it is typeset as math, not as prose"
                .into(),
            suggestion: Some("Escape the dollar sign (\\$) for a literal one, or close the math region with a matching $".into()),
        });
    }

    findings
}

/// Scan one prose text run for the rules that live entirely inside a single
/// `Token::Text` (`~`, `...`, straight quotes, hyphen ranges) and, when the
/// run is outside math and tables, the unescaped special characters.
///
/// `in_quote` toggles per straight-quote pair, so `"text"` is one construct
/// (one finding) rather than two.
fn check_prose(
    rel: &str,
    source: &str,
    text: &str,
    start: usize,
    in_prose: bool,
    in_quote: &mut bool,
    findings: &mut Vec<LintFinding>,
) {
    let bytes = text.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        let c = bytes[i] as char;
        match c {
            '~' => {
                if bytes.get(i + 1).is_some_and(|b| b.is_ascii_digit()) {
                    let value = &text[i + 1..].split_whitespace().next().unwrap_or("");
                    findings.push(LintFinding {
                        file: rel.to_string(),
                        line: line_of(source, start + i),
                        severity: Severity::Warning,
                        message: format!(
                            "`~{value}` — `~` in text mode is a non-breaking space, not \"approximately\"; \
                             the PDF will show a plain space and the figure becomes exact"
                        ),
                        suggestion: Some("Use \\textasciitilde{} (e.g. \\textasciitilde{}60M) or spell the word out".into()),
                    });
                }
                i += 1;
            }
            '.' => {
                if bytes.get(i + 1) == Some(&b'.') && bytes.get(i + 2) == Some(&b'.') {
                    findings.push(LintFinding {
                        file: rel.to_string(),
                        line: line_of(source, start + i),
                        severity: Severity::Warning,
                        message: "Literal \"...\" renders as three periods, not an ellipsis".into(),
                        suggestion: Some("Use \\ldots or \\dots".into()),
                    });
                    i += 3;
                } else {
                    i += 1;
                }
            }
            '"' => {
                if !*in_quote {
                    findings.push(LintFinding {
                        file: rel.to_string(),
                        line: line_of(source, start + i),
                        severity: Severity::Warning,
                        message: "Straight double quote renders as a plain \" glyph, not LaTeX quotation marks".into(),
                        suggestion: Some("Use `` (open) and '' (close) around quoted text".into()),
                    });
                }
                *in_quote = !*in_quote;
                i += 1;
            }
            '-' => {
                if i > 0
                    && bytes.get(i - 1).is_some_and(|b| b.is_ascii_digit())
                    && bytes.get(i + 1).is_some_and(|b| b.is_ascii_digit())
                {
                    let mut left = i - 1;
                    while left > 0 && bytes[left - 1].is_ascii_digit() {
                        left -= 1;
                    }
                    let mut right = i + 1;
                    while right < bytes.len() && bytes[right].is_ascii_digit() {
                        right += 1;
                    }
                    let construct = &text[left..right];
                    findings.push(LintFinding {
                        file: rel.to_string(),
                        line: line_of(source, start + i),
                        severity: Severity::Warning,
                        message: format!(
                            "\"{construct}\" renders as a plain hyphen, not a range dash"
                        ),
                        suggestion: Some("Use an en-dash for numeric ranges: 2020--2024".into()),
                    });
                }
                i += 1;
            }
            '&' | '#' | '_' if in_prose => {
                let (meaning, fix) = match c {
                    '&' => ("an alignment tab character, not a literal ampersand", "\\&"),
                    '#' => ("a macro parameter character, not a literal hash", "\\#"),
                    _ => (
                        "the math subscript operator, not a literal underscore",
                        "\\_",
                    ),
                };
                findings.push(LintFinding {
                    file: rel.to_string(),
                    line: line_of(source, start + i),
                    severity: Severity::Error,
                    message: format!(
                        "Unescaped `{c}` is {meaning} and will not typeset as a literal {c}"
                    ),
                    suggestion: Some(format!("Escape it as {fix}")),
                });
                i += 1;
            }
            _ => i += 1,
        }
    }
}

/// Lint a section title, which is prose but carries commands and math of its
/// own; re-tokenize it so `\ldots`, math and the like stay inert.
fn check_section_title(
    rel: &str,
    source: &str,
    title: &str,
    command_start: usize,
    findings: &mut Vec<LintFinding>,
) {
    let tokenized = tokenize_with_spans(title);
    let mut in_math = 0usize;
    let mut in_quote = false;
    for spanned in &tokenized.tokens {
        match &spanned.token {
            Token::BeginMath => in_math += 1,
            Token::EndMath => in_math = in_math.saturating_sub(1),
            Token::Text(text) => {
                check_prose(
                    rel,
                    source,
                    text,
                    command_start + spanned.start,
                    in_math == 0,
                    &mut in_quote,
                    findings,
                );
            }
            Token::Comment(_) => {
                check_percent(rel, source, command_start + spanned.start, findings);
            }
            _ => {}
        }
    }
}

/// The `%` rule: a `%` that starts a comment immediately after a digit is the
/// gravest miss — `mejora del 20% en latencia` silently loses `en latencia`.
/// A `%` preceded by anything else (line start, space) is a real comment.
fn check_percent(rel: &str, source: &str, start: usize, findings: &mut Vec<LintFinding>) {
    let before = source[..start].chars().next_back();
    if before.is_some_and(|c| c.is_ascii_digit()) {
        let digits: String = source[..start]
            .chars()
            .rev()
            .take_while(|c| c.is_ascii_digit())
            .collect::<String>()
            .chars()
            .rev()
            .collect();
        findings.push(LintFinding {
            file: rel.to_string(),
            line: line_of(source, start),
            severity: Severity::Warning,
            message: format!(
                "\"{digits}%\" — unescaped `%` after a digit starts a comment and silently \
                 deletes the rest of the line"
            ),
            suggestion: Some("Escape the percent sign as \\%".into()),
        });
    }
}

/// 1-based line number of a byte offset.
fn line_of(source: &str, offset: usize) -> usize {
    let offset = offset.min(source.len());
    1 + source[..offset].matches('\n').count()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lint(source: &str) -> Vec<LintFinding> {
        lint_file("main.tex", source)
    }

    fn warning(source: &str, fragment: &str) -> bool {
        lint(source)
            .iter()
            .any(|f| f.severity == Severity::Warning && f.message.contains(fragment))
    }

    fn error(source: &str, fragment: &str) -> bool {
        lint(source)
            .iter()
            .any(|f| f.severity == Severity::Error && f.message.contains(fragment))
    }

    fn silent(source: &str, fragment: &str) -> bool {
        lint(source).iter().any(|f| f.message.contains(fragment))
    }

    // --- `~` as "approximately" ---

    #[test]
    fn tilde_before_digit_is_approximately() {
        assert!(warning(
            r"negociacion comercial, ~60M COP de ahorro.",
            "non-breaking space"
        ));
        assert!(warning(r"~5 anos de experiencia", "non-breaking space"));
    }

    #[test]
    fn tilde_before_command_or_letter_is_a_tie() {
        assert!(!silent(r"ver Figura~\ref{fig:x}", "non-breaking space"));
        assert!(!silent(r"Dr.~Smith", "non-breaking space"));
    }

    // --- unescaped `%` ---

    #[test]
    fn percent_after_digit_eats_the_rest_of_the_line() {
        assert!(warning(r"mejora del 20% en latencia", "silently deletes"));
        assert!(warning(r"descuento 50% aplicado", "silently deletes"));
    }

    #[test]
    fn percent_at_line_start_is_a_real_comment() {
        assert!(!silent("% a real comment", "silently deletes"));
        assert!(!silent("texto\n% a real comment", "silently deletes"));
    }

    #[test]
    fn escaped_percent_does_not_fire() {
        assert!(!silent(r"50\% de descuento", "silently deletes"));
    }

    #[test]
    fn percent_after_digit_inside_prose_command_fires() {
        assert!(warning(
            r"\textit{mejora del 20% en latencia}",
            "silently deletes"
        ));
    }

    // --- literal `...` ---

    #[test]
    fn three_dots_in_prose_are_not_an_ellipsis() {
        assert!(warning(r"y entonces ... sucede algo", "three periods"));
    }

    #[test]
    fn latex_ellipsis_commands_do_not_fire() {
        assert!(!silent(r"\ldots y luego \dots", "three periods"));
    }

    // --- straight quotes ---

    #[test]
    fn straight_double_quote_fires() {
        assert!(warning(
            r#"dijo "hola mundo" y se fue"#,
            "Straight double quote"
        ));
    }

    #[test]
    fn tex_quotes_do_not_fire() {
        assert!(!silent(
            "dijo ``hola mundo'' y se fue",
            "Straight double quote"
        ));
    }

    #[test]
    fn a_quote_pair_is_one_finding() {
        let findings = lint(r#"dijo "hola mundo" y se fue"#);
        let quote_findings: Vec<&LintFinding> = findings
            .iter()
            .filter(|f| f.message.contains("Straight double quote"))
            .collect();
        assert_eq!(quote_findings.len(), 1);
    }

    // --- hyphen as range ---

    #[test]
    fn hyphen_between_digits_is_a_range_dash() {
        assert!(warning(r"periodo 2020-2024", "not a range dash"));
    }

    #[test]
    fn hyphenated_word_does_not_fire() {
        assert!(!silent("well-written code", "not a range dash"));
    }

    // --- unescaped `&`, `#`, `_`, `$` ---

    #[test]
    fn unescaped_special_characters_are_errors() {
        assert!(error(r"a & b", "literal ampersand"));
        assert!(error(r"referencia #1", "literal hash"));
        assert!(error(r"foo_bar", "literal underscore"));
        assert!(error(r"costo $5 dolares", "never closed"));
    }

    #[test]
    fn math_and_tables_do_not_fire() {
        assert!(!silent(r"El valor es $x_{i}^2$ aqui", "literal underscore"));
        assert!(!silent(
            r"\begin{tabular}{cc} a & b \\ 1 & 2 \end{tabular}",
            "literal ampersand"
        ));
    }

    #[test]
    fn starred_math_does_not_fire() {
        assert!(!silent(
            r"\begin{align*}a &= b_0\end{align*}",
            "literal ampersand"
        ));
    }

    #[test]
    fn escaped_special_characters_do_not_fire() {
        assert!(!silent(r"a \& b \# 1 x\_y", "literal"));
    }

    #[test]
    fn verbatim_content_does_not_fire() {
        assert!(!silent(r"\verb|a_b & c|", "literal"));
        assert!(!silent(r"\lstinline|50% ...|", "silently deletes"));
    }

    #[test]
    fn url_arguments_do_not_fire() {
        assert!(!silent(
            r"\url{https://example.com/~user_a}",
            "non-breaking space"
        ));
        assert!(!silent(
            r"\url{https://example.com/a_b}",
            "literal underscore"
        ));
    }

    // --- locations ---

    #[test]
    fn findings_report_the_right_line() {
        let source = "primera linea\nsegunda con ~60M aqui\ntercera";
        let findings = lint(source);
        let tilde = findings
            .iter()
            .find(|f| f.message.contains("non-breaking space"))
            .unwrap();
        assert_eq!(tilde.line, 2);
        assert_eq!(tilde.file, "main.tex");
    }

    #[test]
    fn percent_finding_is_on_the_comment_line() {
        let source = "linea uno\nahorro 20% perdido\nlinea tres";
        let findings = lint(source);
        let finding = findings
            .iter()
            .find(|f| f.message.contains("silently deletes"))
            .unwrap();
        assert_eq!(finding.line, 2);
    }

    #[test]
    fn section_titles_are_prose() {
        assert!(warning(
            r"\section{Resultados 2020-2024}",
            "not a range dash"
        ));
    }

    #[test]
    fn section_title_latex_is_inert() {
        assert!(!silent(
            r"\section{Introduccion \ldots y \dots}",
            "three periods"
        ));
    }

    // --- wiring through the whole linter ---

    #[test]
    fn lint_entry_point_includes_glyph_findings() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("main.tex"),
            "\\documentclass{article}\n\\begin{document}\n~60M de ahorro\n\\end{document}",
        )
        .unwrap();
        let findings = super::super::lint(dir.path(), "main.tex", None).unwrap();
        assert!(findings
            .iter()
            .any(|f| f.severity == Severity::Warning && f.message.contains("non-breaking space")));
    }
}
