//! Source-to-PDF fidelity: every significant source word must survive
//! into the rendered PDF, or the miss is reported (never auto-fixed).

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::Result;

use crate::linter::{LintFinding, Severity};
use crate::texparse::{tokenize, tokenize_document, Token, TokenizedFile};
use crate::texutil::strip_empty_groups;

use super::extract::{extract_text, normalize_pdf_text};
use super::MissingWord;

/// Significant prose words from a tokenized document (TF3), excluding labels,
/// refs and math — the tokenizer already dropped those from [`Token::Text`].
pub fn significant_words(files: &[TokenizedFile]) -> BTreeMap<String, usize> {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut in_document = false;

    for file in files {
        for token in &file.tokens {
            match token {
                Token::BeginDocument => in_document = true,
                Token::EndDocument => in_document = false,
                Token::Section { title, .. } if in_document => {
                    count_title_words(title, &mut counts);
                }
                Token::Text(text) if in_document => {
                    count_text_words(text, &mut counts);
                }
                _ => {}
            }
        }
    }
    counts
}

/// Count the prose words inside a section title (re-tokenized so
/// math/commands in the title are excluded).
fn count_title_words(title: &str, counts: &mut BTreeMap<String, usize>) {
    for title_token in tokenize(title) {
        if let Token::Text(text) = title_token {
            count_text_words(&text, counts);
        }
    }
}

/// Count the prose words inside a single text run.
fn count_text_words(text: &str, counts: &mut BTreeMap<String, usize>) {
    for word in words_in_text(text) {
        *counts.entry(word).or_insert(0) += 1;
    }
}

/// Words from a prose run: non-space spans with ≥1 alphabetic character,
/// with leading/trailing punctuation stripped and empty groups removed for
/// PDF matching.
fn words_in_text(text: &str) -> impl Iterator<Item = String> + '_ {
    text.split_whitespace().filter_map(|raw| {
        let trimmed = strip_empty_groups(trim_punct(raw));
        if trimmed.chars().any(char::is_alphabetic) {
            Some(trimmed)
        } else {
            None
        }
    })
}

fn trim_punct(word: &str) -> &str {
    word.trim_matches(|c: char| !c.is_alphanumeric() && c != '-' && c != '\'')
}

/// Compare significant source words against normalized PDF text.
///
/// Returns the distinct missing words with how often they appear in the source.
pub fn fidelity_missing_words(
    source_words: &BTreeMap<String, usize>,
    pdf_text_normalized: &str,
) -> Vec<MissingWord> {
    // Build a searchable haystack of PDF words (normalized, punct-trimmed).
    let pdf_words: std::collections::HashSet<String> = pdf_text_normalized
        .split_whitespace()
        .filter_map(|w| {
            let t = trim_punct(w);
            if t.is_empty() {
                None
            } else {
                Some(t.to_string())
            }
        })
        .collect();

    // Also keep the full text for substring fallback (handles hyphenated
    // compounds already rejoined into a single token).
    let mut missing = Vec::new();
    for (word, count) in source_words {
        if pdf_words.contains(word) || pdf_text_normalized.contains(word.as_str()) {
            continue;
        }
        missing.push(MissingWord {
            word: word.clone(),
            count: *count,
        });
    }
    missing
}

/// Turn missing words into Warning findings. Suggestion breaks the first
/// ligature pair with an empty group (`Artif{}icial`) — the portable fix on
/// the Tectonic stack.
pub fn fidelity_findings(missing: &[MissingWord]) -> Vec<LintFinding> {
    missing
        .iter()
        .map(|m| {
            let suggestion = ligature_break_suggestion(&m.word);
            let message = if m.count == 1 {
                format!(
                    "source word `{}` not found in PDF text (ligature, hyphenation, or encoding)",
                    m.word
                )
            } else {
                format!(
                    "source word `{}` not found in PDF text ({} occurrences in source; ligature, hyphenation, or encoding)",
                    m.word, m.count
                )
            };
            LintFinding {
                file: "pdf".into(),
                line: 0,
                severity: Severity::Warning,
                message,
                suggestion,
            }
        })
        .collect()
}

/// Suggest breaking the first `fi`/`fl`/`ff`/`ffi`/`ffl` pair with `{}`.
fn ligature_break_suggestion(word: &str) -> Option<String> {
    // Longest pairs first so `ffi` wins over `fi`.
    const PAIRS: &[&str] = &["ffi", "ffl", "ff", "fi", "fl"];
    let lower = word.to_ascii_lowercase();
    for pair in PAIRS {
        if let Some(idx) = lower.find(pair) {
            // Split after the first letter of the pair: `Artif{}icial`.
            let split_at = idx + 1;
            let (before, after) = word.split_at(split_at);
            return Some(format!(
                "Break the ligature in the source with an empty group: `{before}{{}}{after}` \
                 (microtype/\\DisableLigatures and fontspec Ligatures=NoCommon do not work under Tectonic)"
            ));
        }
    }
    Some(
        "Ensure the word survives compilation; if a ligature is involved, break it with \
         an empty group (e.g. `Artif{}icial`)"
            .into(),
    )
}

/// Run the fidelity check for a project: tokenize source, extract+normalize
/// PDF text, report distinct missing words as warnings.
pub fn check_fidelity(root: &Path, entry: &str, pdf_path: &Path) -> Result<Vec<LintFinding>> {
    let files = tokenize_document(root, entry);
    let source_words = significant_words(&files);
    let raw = extract_text(pdf_path)?;
    let normalized = normalize_pdf_text(&raw);
    let missing = fidelity_missing_words(&source_words, &normalized);
    Ok(fidelity_findings(&missing))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    use crate::texparse::TokenizedFile;

    use super::super::extract::extract_text_from_bytes;
    use super::super::fixtures::LIGATURES_PDF;

    #[test]
    fn fidelity_flags_missing_words_without_flooding() {
        let mut source = BTreeMap::new();
        source.insert("Artificial".into(), 3);
        source.insert("MLflow".into(), 1);
        source.insert("present".into(), 1);
        // Raw ligature text — without normalize, words are missing.
        let raw = "Arti\u{FB01}cial ML\u{FB02}ow present";
        let missing_raw = fidelity_missing_words(&source, raw);
        assert!(
            missing_raw
                .iter()
                .any(|m| m.word == "Artificial" && m.count == 3),
            "{missing_raw:?}"
        );
        // After normalize, only nothing missing for these.
        let missing_norm = fidelity_missing_words(&source, &normalize_pdf_text(raw));
        assert!(missing_norm.is_empty(), "{missing_norm:?}");
    }

    #[test]
    fn fidelity_findings_are_warnings_with_empty_group_suggestion() {
        let missing = vec![MissingWord {
            word: "Artificial".into(),
            count: 3,
        }];
        let findings = fidelity_findings(&missing);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, Severity::Warning);
        let suggestion = findings[0].suggestion.as_deref().unwrap();
        assert!(
            suggestion.contains("Artif{}icial"),
            "suggestion was {suggestion}"
        );
        assert!(
            !suggestion.to_lowercase().contains("disableligatures")
                || suggestion.contains("do not work")
        );
    }

    #[test]
    fn significant_words_skip_math_and_preamble() {
        let files = vec![TokenizedFile {
            path: PathBuf::from("main.tex"),
            tokens: vec![
                Token::Command {
                    name: "documentclass".into(),
                    args: vec!["article".into()],
                },
                Token::Text("IgnorePreamble".into()),
                Token::BeginDocument,
                Token::Text("Hello $x$ world Artificial".into()),
                Token::BeginMath,
                Token::EndMath,
                Token::Text(" and MLflow.".into()),
            ],
        }];
        // Note: tokenize would strip math; here we simulate post-tokenizer stream.
        let words = significant_words(&files);
        assert!(words.contains_key("Hello"));
        assert!(words.contains_key("world"));
        assert!(words.contains_key("Artificial"));
        assert!(words.contains_key("MLflow"));
        assert!(!words.contains_key("IgnorePreamble"));
    }

    #[test]
    fn tabular_column_spec_is_excluded_from_significant_words() {
        let source = "\\begin{document}\n\\begin{tabular}{@{}>{\\bfseries}p{3cm}>{\\raggedright\\arraybackslash}p{5.5cm}@{}}\nName & Alice \\\\\n\\end{tabular}\n\\end{document}\n";
        let files = vec![TokenizedFile {
            path: PathBuf::from("main.tex"),
            tokens: crate::texparse::tokenize(source),
        }];
        let words = significant_words(&files);
        assert!(!words.contains_key("p{3cm"), "{words:?}");
        assert!(!words.contains_key("p{5.5cm"), "{words:?}");
        assert!(words.contains_key("Name"), "{words:?}");
        assert!(words.contains_key("Alice"), "{words:?}");
    }

    #[test]
    fn ligature_workaround_empty_group_matches_rendered_word() {
        let source =
            "\\begin{document}\nWe streamlined the workf{}lows for the team.\n\\end{document}\n";
        let files = vec![TokenizedFile {
            path: PathBuf::from("main.tex"),
            tokens: crate::texparse::tokenize(source),
        }];
        let words = significant_words(&files);
        assert!(words.contains_key("workflows"), "{words:?}");
        assert!(!words.contains_key("workf{}lows"), "{words:?}");

        let missing = fidelity_missing_words(&words, "We streamlined the workflows for the team.");
        assert!(missing.is_empty(), "{missing:?}");
    }

    #[test]
    fn genuinely_missing_word_still_warns() {
        let mut source = BTreeMap::new();
        source.insert("Nonexistent".into(), 1);
        let missing = fidelity_missing_words(&source, "some other text entirely");
        assert_eq!(missing.len(), 1);
        assert_eq!(missing[0].word, "Nonexistent");
    }

    #[test]
    fn package_options_and_hypersetup_keys_are_excluded_from_significant_words() {
        let source = "\\usepackage[hyphens]{url}\n\\hypersetup{pdfcreationdate={\\today}, colorlinks=true}\n\\setlength{\\parindent}{0pt}\n\\begin{document}\nHello world.\n\\end{document}\n";
        let files = vec![TokenizedFile {
            path: PathBuf::from("main.tex"),
            tokens: crate::texparse::tokenize(source),
        }];
        let words = significant_words(&files);
        assert!(words.contains_key("Hello"), "{words:?}");
        assert!(words.contains_key("world"), "{words:?}");
        assert!(!words.contains_key("hyphens"), "{words:?}");
        assert!(!words.contains_key("pdfcreationdate"), "{words:?}");
        assert!(!words.contains_key("colorlinks"), "{words:?}");
        assert!(!words.contains_key("parindent"), "{words:?}");
    }

    #[test]
    fn fixture_fidelity_passes_after_normalize() {
        let raw = extract_text_from_bytes(LIGATURES_PDF).unwrap();
        let mut source = BTreeMap::new();
        for w in [
            "Artificial",
            "MLflow",
            "workflows",
            "local-first",
            "Learning",
        ] {
            source.insert(w.into(), 1);
        }
        let missing = fidelity_missing_words(&source, &normalize_pdf_text(&raw));
        assert!(missing.is_empty(), "unexpected missing: {missing:?}");
    }
}
