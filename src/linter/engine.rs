//! Engine-compatibility lint rules (TF7).
//!
//! texforge always compiles with Tectonic, which is a `XeTeX` engine. A generic
//! LaTeX linter cannot warn about constructs that behave badly specifically
//! under `XeTeX`, because it does not know the engine; texforge does. These
//! rules flag the packages and commands that Tectonic ignores, and the ones
//! that break the build outright.
//!
//! | Rule | Fires when | Severity |
//! |---|---|---|
//! | `inputenc` loaded | `\usepackage[utf8]{inputenc}` | [`Severity::Warning`] — ignored |
//! | `epstopdf` loaded | `\usepackage{epstopdf}` | [`Severity::Warning`] — no EPS conversion |
//! | `\DisableLigatures` with microtype | `\usepackage{microtype}` + `\DisableLigatures` | [`Severity::Error`] — build fails |
//! | `\setmainfont{Latin Modern Roman}` with fontspec | `\usepackage{fontspec}` + `\setmainfont{Latin Modern Roman}` | [`Severity::Error`] — build fails |
//!
//! The rules are data, not code branches: each entry in [`ENGINE_RULES`]
//! lists the [`TriggerKind`]s that must all be observed, a severity, and a
//! message that names the engine and the consequence. Detection runs over the
//! token stream of every file the `\input` traversal reaches, restricted to
//! the preamble, so comments and verbatim text can never fire a rule.

use crate::texparse::{tokenize_with_spans, Token};

use super::{LintFinding, Severity};

/// An observable LaTeX construct a rule can require.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TriggerKind {
    /// `\usepackage{name}`, with any options.
    Package(&'static str),
    /// The command `\name`, with or without arguments.
    Command(&'static str),
    /// `\command{arg}` whose last braced argument equals `arg` exactly.
    CommandWithArg(&'static str, &'static str),
}

/// One engine-compatibility rule.
#[derive(Debug)]
pub struct EngineRule {
    /// Severity split: ignored constructs are [`Severity::Warning`];
    /// build-breaking constructs are [`Severity::Error`].
    pub severity: Severity,
    /// Constructs that must ALL be observed for the rule to fire. The last
    /// trigger is the actionable one and owns the finding's location.
    pub triggers: &'static [TriggerKind],
    /// Message naming the engine and stating the consequence in plain terms.
    pub message: &'static str,
    pub suggestion: &'static str,
}

/// The data table of engine-compatibility rules. Adding a construct is one
/// entry here; the scanning and reporting logic never grows.
pub const ENGINE_RULES: &[EngineRule] = &[
    EngineRule {
        severity: Severity::Warning,
        triggers: &[TriggerKind::Package("inputenc")],
        message: "\\usepackage{inputenc} is ignored under Tectonic (a XeTeX engine): it always \
                  reads UTF-8 source, so the package does nothing",
        suggestion: "Remove the \\usepackage{inputenc} line",
    },
    EngineRule {
        severity: Severity::Warning,
        triggers: &[TriggerKind::Package("epstopdf")],
        message: "\\usepackage{epstopdf} will not work under Tectonic (a XeTeX engine): epstopdf \
                  supports only the pdfTeX and LuaTeX drivers, so included .eps graphics are not \
                  converted to a renderable format",
        suggestion: "Convert the .eps files to PDF or PNG and \\includegraphics those instead",
    },
    EngineRule {
        severity: Severity::Error,
        triggers: &[
            TriggerKind::Package("microtype"),
            TriggerKind::Command("DisableLigatures"),
        ],
        message: "\\DisableLigatures breaks the build under Tectonic (a XeTeX engine): microtype \
                  cannot disable a font's ligatures there — XeTeX aborts with 'Disabling \
                  ligatures of a font is only possible'",
        suggestion: "Remove \\DisableLigatures from the preamble",
    },
    EngineRule {
        severity: Severity::Error,
        triggers: &[
            TriggerKind::Package("fontspec"),
            TriggerKind::CommandWithArg("setmainfont", "Latin Modern Roman"),
        ],
        message: "\\setmainfont{Latin Modern Roman} breaks the build under Tectonic (a XeTeX \
                  engine): Tectonic ships no system fonts, so XeTeX aborts with 'The font ... \
                  cannot be found'",
        suggestion: "Remove the \\setmainfont line and use the engine's bundled Latin Modern \
                     instead",
    },
];

/// Lint every file reachable from the entry point against the engine rules.
///
/// Each entry is a `(project-relative path, source)` pair; the caller already
/// resolved `\input` traversal ([`crate::texutil::collect_tex_files`]), so
/// packages loaded from included files are seen here. Compound rules
/// accumulate their triggers across files, and each rule fires at most once
/// per project, at the location of its actionable (last) trigger.
pub fn lint_files(files: &[(String, String)]) -> Vec<LintFinding> {
    let mut fired: Vec<Vec<Option<(String, usize)>>> = ENGINE_RULES
        .iter()
        .map(|rule| vec![None; rule.triggers.len()])
        .collect();

    for (rel, source) in files {
        let tokenized = tokenize_with_spans(source);
        for spanned in &tokenized.tokens {
            match &spanned.token {
                Token::BeginDocument => break,
                Token::Command { name, args } => {
                    let line = line_of(source, spanned.start);
                    for (rule_idx, rule) in ENGINE_RULES.iter().enumerate() {
                        for (trigger_idx, trigger) in rule.triggers.iter().enumerate() {
                            if fired[rule_idx][trigger_idx].is_none()
                                && trigger_matches(trigger, name, args)
                            {
                                fired[rule_idx][trigger_idx] = Some((rel.clone(), line));
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    let mut findings = Vec::new();
    for (rule_idx, rule) in ENGINE_RULES.iter().enumerate() {
        if fired[rule_idx].iter().all(Option::is_some) {
            let (file, line) = fired[rule_idx].last().unwrap().as_ref().unwrap();
            findings.push(LintFinding {
                file: file.clone(),
                line: *line,
                severity: rule.severity,
                message: rule.message.to_string(),
                suggestion: Some(rule.suggestion.to_string()),
            });
        }
    }
    findings
}

/// Whether one `\command` token satisfies a trigger.
fn trigger_matches(trigger: &TriggerKind, name: &str, args: &[String]) -> bool {
    match trigger {
        TriggerKind::Package(pkg) => {
            name == "usepackage" && args.last().is_some_and(|a| a.as_str() == *pkg)
        }
        TriggerKind::Command(cmd) => name == *cmd,
        TriggerKind::CommandWithArg(cmd, arg) => {
            name == *cmd && args.last().is_some_and(|a| a.as_str() == *arg)
        }
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
        lint_files(&[("main.tex".to_string(), source.to_string())])
    }

    fn has_severity_with(findings: &[LintFinding], fragment: &str, sev: Severity) -> bool {
        findings
            .iter()
            .any(|f| f.severity == sev && f.message.contains(fragment))
    }

    // --- the four constructs, severity + consequence naming ---

    #[test]
    fn inputenc_is_a_warning_naming_the_consequence() {
        let findings = lint(
            r"\documentclass{article}
\usepackage[utf8]{inputenc}
\begin{document}
\end{document}",
        );
        assert!(has_severity_with(
            &findings,
            "does nothing",
            Severity::Warning
        ));
        assert!(has_severity_with(&findings, "XeTeX", Severity::Warning));
    }

    #[test]
    fn epstopdf_is_a_warning_naming_the_consequence() {
        let findings = lint(
            r"\documentclass{article}
\usepackage{epstopdf}
\begin{document}
\end{document}",
        );
        assert!(has_severity_with(&findings, ".eps", Severity::Warning));
        assert!(has_severity_with(&findings, "XeTeX", Severity::Warning));
    }

    #[test]
    fn disable_ligatures_with_microtype_is_an_error_naming_the_consequence() {
        let findings = lint(
            r"\documentclass{article}
\usepackage{microtype}
\DisableLigatures
\begin{document}
\end{document}",
        );
        assert!(has_severity_with(
            &findings,
            "breaks the build",
            Severity::Error
        ));
        assert!(has_severity_with(&findings, "XeTeX", Severity::Error));
    }

    #[test]
    fn latin_modern_roman_setmainfont_with_fontspec_is_an_error_naming_the_consequence() {
        let findings = lint(
            r"\documentclass{article}
\usepackage{fontspec}
\setmainfont{Latin Modern Roman}
\begin{document}
\end{document}",
        );
        assert!(has_severity_with(
            &findings,
            "breaks the build",
            Severity::Error
        ));
        assert!(has_severity_with(
            &findings,
            "cannot be found",
            Severity::Error
        ));
    }

    // --- no false positives ---

    #[test]
    fn microtype_without_disable_ligatures_is_clean() {
        let findings = lint(
            r"\documentclass{article}
\usepackage{microtype}
\begin{document}
\end{document}",
        );
        assert!(!has_severity_with(&findings, "ligature", Severity::Error));
    }

    #[test]
    fn disable_ligatures_without_microtype_is_clean() {
        let findings = lint(
            r"\documentclass{article}
\DisableLigatures
\begin{document}
\end{document}",
        );
        assert!(!has_severity_with(&findings, "ligature", Severity::Error));
    }

    #[test]
    fn setmainfont_of_another_font_is_clean() {
        let findings = lint(
            r"\documentclass{article}
\usepackage{fontspec}
\setmainfont{TeX Gyre Termes}
\begin{document}
\end{document}",
        );
        assert!(!has_severity_with(
            &findings,
            "cannot be found",
            Severity::Error
        ));
    }

    #[test]
    fn setmainfont_without_fontspec_is_clean() {
        let findings = lint(
            r"\documentclass{article}
\setmainfont{Latin Modern Roman}
\begin{document}
\end{document}",
        );
        assert!(!has_severity_with(
            &findings,
            "cannot be found",
            Severity::Error
        ));
    }

    #[test]
    fn a_clean_document_has_no_engine_findings() {
        let findings = lint(
            r"\documentclass{article}
\begin{document}
Hello
\end{document}",
        );
        assert!(findings.is_empty());
    }

    #[test]
    fn constructs_after_begin_document_are_not_reported() {
        let findings = lint(
            r"\documentclass{article}
\begin{document}
\usepackage[utf8]{inputenc}
\setmainfont{Latin Modern Roman}
\end{document}",
        );
        assert!(findings.is_empty());
    }

    #[test]
    fn constructs_in_comments_are_not_reported() {
        let findings = lint(
            r"\documentclass{article}
% \usepackage[utf8]{inputenc}
% \usepackage{epstopdf}
\begin{document}
\end{document}",
        );
        assert!(findings.is_empty());
    }

    #[test]
    fn compound_rule_reports_the_command_location() {
        let findings = lint(
            "\\documentclass{article}\n\\usepackage{microtype}\n\\DisableLigatures\n\\begin{document}",
        );
        let finding = findings
            .iter()
            .find(|f| f.severity == Severity::Error && f.message.contains("DisableLigatures"))
            .unwrap();
        assert_eq!(finding.line, 3);
        assert_eq!(finding.file, "main.tex");
    }

    // --- wiring through \input traversal ---

    #[test]
    fn packages_loaded_from_input_files_are_seen() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("main.tex"),
            "\\documentclass{article}\n\\input{preamble}\n\\begin{document}\n\\end{document}",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("preamble.tex"),
            "\\usepackage[utf8]{inputenc}\n\\usepackage{epstopdf}",
        )
        .unwrap();
        let findings = super::super::lint(dir.path(), "main.tex", None).unwrap();
        assert!(has_severity_with(
            &findings,
            "does nothing",
            Severity::Warning
        ));
        assert!(has_severity_with(&findings, ".eps", Severity::Warning));
    }

    #[test]
    fn compound_rule_fires_across_input_files() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("main.tex"),
            "\\documentclass{article}\n\\usepackage{fontspec}\n\\input{preamble}\n\\begin{document}\n\\end{document}",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("preamble.tex"),
            "\\setmainfont{Latin Modern Roman}",
        )
        .unwrap();
        let findings = super::super::lint(dir.path(), "main.tex", None).unwrap();
        assert!(has_severity_with(
            &findings,
            "cannot be found",
            Severity::Error
        ));
    }
}
