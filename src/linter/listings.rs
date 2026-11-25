//! `check` findings for `code` blocks that read from a file.
//!
//! Mirrors the build: the same [`crate::highlight::source::resolve`] the
//! rewrite runs, so `check` reports the same problems (missing file, bad
//! range, range past the end, escaping path, `lines=` without `file=`) as
//! findings without building — plus a warning when a block carries both
//! `file=` and a body (the build ignores the body).

use std::path::Path;

use super::{LintFinding, Severity};
use crate::highlight::{source, CODE_OPTION_KEYS};
use crate::texparse;
use crate::texutil;

/// Check every `code` block in one file, returning its findings.
pub(crate) fn check_code_blocks(root: &Path, rel: &str, content: &str) -> Vec<LintFinding> {
    let mut findings = Vec::new();
    for block in texparse::verbatim_blocks(content) {
        if block.env != "code" {
            continue;
        }
        let line = 1 + content[..block.begin_start].matches('\n').count();
        let Some(opts_text) =
            content[block.begin_start..block.body_start].strip_prefix("\\begin{code}")
        else {
            continue;
        };
        let opts = match texutil::parse_opts(opts_text, "code", CODE_OPTION_KEYS) {
            Ok((opts, _)) => opts,
            Err(error) => {
                findings.push(error_finding(rel, line, error.to_string()));
                continue;
            }
        };
        match source::resolve(root, rel, line, &opts, "") {
            Ok(resolved) => {
                let body = content[block.body_start..block.end_start].trim();
                if resolved.external && !body.is_empty() {
                    findings.push(LintFinding {
                        file: rel.to_string(),
                        line,
                        severity: Severity::Warning,
                        message: "file= ignores the block body".to_string(),
                        suggestion: Some("remove the body or drop file= to render it".to_string()),
                    });
                }
            }
            Err(error) => findings.push(error_finding(rel, line, error.to_string())),
        }
    }
    findings
}

fn error_finding(rel: &str, line: usize, message: String) -> LintFinding {
    LintFinding {
        file: rel.to_string(),
        line,
        severity: Severity::Error,
        message,
        suggestion: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(files: &[(&str, &str)], entry: &str) -> Vec<LintFinding> {
        let dir = tempfile::tempdir().unwrap();
        for (name, content) in files {
            std::fs::write(dir.path().join(name), content).unwrap();
        }
        let rel = entry.to_string();
        let content = std::fs::read_to_string(dir.path().join(entry)).unwrap();
        check_code_blocks(dir.path(), &rel, &content)
    }

    #[test]
    fn missing_file_is_an_error() {
        let findings = check(
            &[("main.tex", "\\begin{code}[file=gone.py]\nx\n\\end{code}\n")],
            "main.tex",
        );
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, Severity::Error);
        assert!(
            findings[0].message.contains("gone.py"),
            "{}",
            findings[0].message
        );
    }

    #[test]
    fn malformed_range_is_an_error() {
        let findings = check(
            &[
                ("s.py", "x = 1\n"),
                (
                    "main.tex",
                    "\\begin{code}[file=s.py, lines=1-2-3]\n\\end{code}\n",
                ),
            ],
            "main.tex",
        );
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, Severity::Error);
        assert!(
            findings[0].message.contains("malformed"),
            "{}",
            findings[0].message
        );
    }

    #[test]
    fn range_past_the_end_is_an_error() {
        let findings = check(
            &[
                ("s.py", "x = 1\n"),
                (
                    "main.tex",
                    "\\begin{code}[file=s.py, lines=9-12]\n\\end{code}\n",
                ),
            ],
            "main.tex",
        );
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, Severity::Error);
        assert!(
            findings[0].message.contains("past the end"),
            "{}",
            findings[0].message
        );
    }

    #[test]
    fn lines_without_file_is_an_error() {
        let findings = check(
            &[("main.tex", "\\begin{code}[lines=1-2]\nx\n\\end{code}\n")],
            "main.tex",
        );
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, Severity::Error);
        assert!(
            findings[0].message.contains("lines= requires file="),
            "{}",
            findings[0].message
        );
    }

    #[test]
    fn escaping_path_is_an_error() {
        let findings = check(
            &[(
                "main.tex",
                "\\begin{code}[file=../outside.py]\n\\end{code}\n",
            )],
            "main.tex",
        );
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, Severity::Error);
        assert!(
            findings[0].message.contains("inside the project root"),
            "{}",
            findings[0].message
        );
    }

    #[test]
    fn body_beside_file_warns() {
        let findings = check(
            &[
                ("s.py", "x = 1\n"),
                ("main.tex", "\\begin{code}[file=s.py]\nx = 2\n\\end{code}\n"),
            ],
            "main.tex",
        );
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, Severity::Warning);
        assert!(
            findings[0].message.contains("ignores the block body"),
            "{}",
            findings[0].message
        );
    }

    #[test]
    fn clean_blocks_report_nothing() {
        let findings = check(
            &[
                ("s.py", "x = 1\ny = 2\n"),
                (
                    "main.tex",
                    "\\begin{code}[lang=python]\nx = 1\n\\end{code}\n\
                     \\begin{code}[file=s.py, lines=1-2]\n\\end{code}\n",
                ),
            ],
            "main.tex",
        );
        assert!(findings.is_empty(), "{findings:?}");
    }
}
