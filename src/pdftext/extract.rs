//! Raw text extraction from the PDF, plus ligature/hyphen normalization.

use std::path::Path;

use anyhow::Result;

/// Common alphabetic ligature codepoints → their ASCII letter expansions.
const LIGATURES: &[(char, &str)] = &[
    ('\u{FB00}', "ff"),  // ﬀ
    ('\u{FB01}', "fi"),  // ﬁ
    ('\u{FB02}', "fl"),  // ﬂ
    ('\u{FB03}', "ffi"), // ﬃ
    ('\u{FB04}', "ffl"), // ﬄ
    ('\u{FB05}', "st"),  // ﬅ
    ('\u{FB06}', "st"),  // ﬆ
];

/// Extract raw PDF text (ligatures left as `ToUnicode` mapped them).
pub fn extract_text(path: &Path) -> Result<String> {
    pdf_extract::extract_text(path)
        .map_err(|e| anyhow::anyhow!("failed to extract text from {}: {e}", path.display()))
}

/// Extract raw PDF text from bytes.
#[allow(dead_code)]
pub fn extract_text_from_bytes(data: &[u8]) -> Result<String> {
    pdf_extract::extract_text_from_mem(data)
        .map_err(|e| anyhow::anyhow!("failed to extract text from PDF bytes: {e}"))
}

/// Extract raw text one page at a time (1-based order).
pub fn extract_text_by_pages(path: &Path) -> Result<Vec<String>> {
    pdf_extract::extract_text_by_pages(path).map_err(|e| {
        anyhow::anyhow!(
            "failed to extract per-page text from {}: {e}",
            path.display()
        )
    })
}

/// Extract raw text one page at a time from bytes.
///
/// Used by the fixture tests and by callers that already hold the PDF in memory.
#[allow(dead_code)]
pub fn extract_text_by_pages_from_bytes(data: &[u8]) -> Result<Vec<String>> {
    pdf_extract::extract_text_from_mem_by_pages(data)
        .map_err(|e| anyhow::anyhow!("failed to extract per-page text from PDF bytes: {e}"))
}

/// Map common ligature codepoints back to letters and rejoin hyphenated
/// line-breaks (`Deep Learn-\ning` → `Deep Learning`).
pub fn normalize_pdf_text(raw: &str) -> String {
    let expanded = expand_ligatures(raw);
    rejoin_hyphenated_linebreaks(&expanded)
}

/// Expand U+FB00..U+FB06 ligatures to their ASCII letter sequences.
pub fn expand_ligatures(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if let Some((_, repl)) = LIGATURES.iter().find(|(lig, _)| *lig == c) {
            out.push_str(repl);
        } else {
            out.push(c);
        }
    }
    out
}

/// Rejoin words split by a hyphen at a line break.
///
/// Matches `letter - newline letter` (with optional CR) and also strips soft
/// hyphens (U+00AD).
pub fn rejoin_hyphenated_linebreaks(text: &str) -> String {
    let without_soft = text.replace('\u{00AD}', "");
    let chars: Vec<char> = without_soft.chars().collect();
    let mut out = String::with_capacity(without_soft.len());
    let mut i = 0;
    // Bounded: each pass emits or skips at least one character, so one
    // iteration per input character covers the whole walk.
    for _ in 0..=chars.len() {
        let Some(&c) = chars.get(i) else {
            break;
        };
        if c == '-' && i > 0 && chars[i - 1].is_alphabetic() {
            if let Some(after) = word_after_hyphen_break(&chars, i) {
                // Skip the hyphen and the line break(s); keep the next letter.
                i = after;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// The index just past the line break(s) that follow the hyphen at
/// `hyphen`, when a letter continues the word there — i.e. the hyphen
/// separated a word across a line break rather than ending it.
fn word_after_hyphen_break(chars: &[char], hyphen: usize) -> Option<usize> {
    let start = hyphen + 1;
    if !chars.get(start).is_some_and(|c| matches!(c, '\n' | '\r')) {
        return None;
    }
    let mut j = start;
    for _ in 0..=chars.len() {
        if !chars.get(j).is_some_and(|c| matches!(c, '\n' | '\r')) {
            break;
        }
        j += 1;
    }
    chars.get(j).is_some_and(|c| c.is_alphabetic()).then_some(j)
}

#[cfg(test)]
mod tests {
    use super::super::fixtures::LIGATURES_PDF;
    use super::*;

    #[test]
    fn expand_ligatures_maps_common_codepoints() {
        assert_eq!(expand_ligatures("Arti\u{FB01}cial"), "Artificial");
        assert_eq!(expand_ligatures("ML\u{FB02}ow"), "MLflow");
        assert_eq!(expand_ligatures("work\u{FB02}ows"), "workflows");
        assert_eq!(expand_ligatures("local-\u{FB01}rst"), "local-first");
        assert_eq!(expand_ligatures("\u{FB00}\u{FB03}\u{FB04}"), "ffffiffl");
    }

    #[test]
    fn rejoin_hyphenated_linebreaks_merges_split_words() {
        assert_eq!(
            rejoin_hyphenated_linebreaks("Deep Learn-\ning"),
            "Deep Learning"
        );
        assert_eq!(
            rejoin_hyphenated_linebreaks("Deep Learn-\r\ning"),
            "Deep Learning"
        );
        // Real hyphen in a compound must stay.
        assert_eq!(rejoin_hyphenated_linebreaks("local-first"), "local-first");
    }

    /// Boundary shapes: only `letter - newline letter` rejoins. A hyphen
    /// with no break after it, a break with no hyphen, a break that runs off
    /// the end of the text, and a leading hyphen all pass through byte for
    /// byte.
    #[test]
    fn rejoin_only_touches_a_hyphen_that_actually_breaks_a_word() {
        assert_eq!(rejoin_hyphenated_linebreaks("word-"), "word-");
        assert_eq!(
            rejoin_hyphenated_linebreaks("Deep Learn-\n"),
            "Deep Learn-\n"
        );
        assert_eq!(rejoin_hyphenated_linebreaks("-\ning"), "-\ning");
        assert_eq!(rejoin_hyphenated_linebreaks("ab\ncd"), "ab\ncd");
        assert_eq!(
            rejoin_hyphenated_linebreaks("line one\nline two"),
            "line one\nline two"
        );
        // A break whose continuation is not a letter (or is absent) keeps
        // the hyphen: the word ended there, it was not split.
        assert_eq!(rejoin_hyphenated_linebreaks("Learn-\n2024"), "Learn-\n2024");
        // One or more line breaks are still a break: the split word rejoins
        // across a blank line too (the historical behaviour).
        assert_eq!(rejoin_hyphenated_linebreaks("Learn-\n\nNext"), "LearnNext");
    }

    #[test]
    fn normalize_handles_ligatures_and_hyphenation_together() {
        let raw = "Arti\u{FB01}cial Deep Learn-\ning work\u{FB02}ows";
        assert_eq!(
            normalize_pdf_text(raw),
            "Artificial Deep Learning workflows"
        );
    }

    #[test]
    fn fixture_extracts_raw_ligature_codepoints() {
        let raw = extract_text_from_bytes(LIGATURES_PDF).unwrap();
        assert!(
            raw.contains('\u{FB01}') || raw.contains('\u{FB02}'),
            "fixture must contain ligature codepoints; got {raw:?}"
        );
        assert!(!raw.contains("Artificial"), "raw should keep ﬁ, not fi");
    }

    #[test]
    fn fixture_normalized_text_is_searchable() {
        let raw = extract_text_from_bytes(LIGATURES_PDF).unwrap();
        let norm = normalize_pdf_text(&raw);
        for word in [
            "Artificial",
            "MLflow",
            "workflows",
            "local-first",
            "Deep",
            "Learning",
        ] {
            assert!(
                norm.contains(word),
                "normalized text missing {word}: {norm:?}"
            );
        }
    }
}
