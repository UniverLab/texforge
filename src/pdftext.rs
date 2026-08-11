//! PDF text extraction and source-to-PDF fidelity (TF8).
//!
//! Extracts text the way a reader or ATS sees it, normalizes typographic
//! ligatures and hyphenated line breaks for comparison, and checks that every
//! significant source word (from the TF3 tokenizer) still appears in the PDF.
//! Missing words are reported as warnings; the source is never auto-edited.
//!
//! Also verifies font embedding and Info-dictionary date shape (TF15).

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result};
use lopdf::{Dictionary, Document, Object, ObjectId};

use crate::linter::{LintFinding, Severity};
use crate::texparse::{tokenize, tokenize_document, Token, TokenizedFile};

/// PDF Info date shape required by ISO 32000 (`D:YYYYMMDDHHmmSS` plus optional TZ).
pub const PDF_DATE_EXPECTED: &str = "D:YYYYMMDDHHmmSS";

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

/// One font used by the PDF.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PdfFontInfo {
    /// PostScript / `BaseFont` name.
    pub name: String,
    /// `/Subtype` (Type1, TrueType, Type0, …).
    pub subtype: String,
    /// Whether a `FontFile` / `FontFile2` / `FontFile3` stream is present.
    pub embedded: bool,
    /// 1-based page numbers that reference this font (sorted, unique).
    pub pages: Vec<usize>,
}

/// Document-level metadata from the Info dictionary.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PdfMetadata {
    pub title: Option<String>,
    pub author: Option<String>,
    pub subject: Option<String>,
    pub keywords: Option<String>,
    pub creator: Option<String>,
    pub producer: Option<String>,
    pub creation_date: Option<String>,
    pub mod_date: Option<String>,
}

/// Summary returned by [`pdf_info`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PdfInfo {
    pub pages: usize,
    pub fonts: Vec<PdfFontInfo>,
    pub metadata: PdfMetadata,
}

/// One page in the machine-readable pages report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PdfPageBreak {
    /// 1-based page number.
    pub page: usize,
    /// Dotted section number that opens this page, when known.
    pub section: Option<String>,
    /// Section title that opens this page, when known.
    pub title: Option<String>,
}

/// A distinct source word missing from the extracted PDF text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissingWord {
    pub word: String,
    pub count: usize,
}

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
    while i < chars.len() {
        if chars[i] == '-' && i > 0 && chars[i - 1].is_alphabetic() && i + 1 < chars.len() {
            let mut j = i + 1;
            while j < chars.len() && (chars[j] == '\n' || chars[j] == '\r') {
                j += 1;
            }
            if j < chars.len() && chars[j].is_alphabetic() && j > i + 1 {
                // Skip the hyphen and the line break(s); keep the next letter.
                i += 1;
                while i < chars.len() && (chars[i] == '\n' || chars[i] == '\r') {
                    i += 1;
                }
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

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
                    // Re-tokenize so math/commands inside the title are excluded.
                    for t in tokenize(title) {
                        if let Token::Text(text) = t {
                            for word in words_in_text(&text) {
                                *counts.entry(word).or_insert(0) += 1;
                            }
                        }
                    }
                }
                Token::Text(text) if in_document => {
                    for word in words_in_text(text) {
                        *counts.entry(word).or_insert(0) += 1;
                    }
                }
                _ => {}
            }
        }
    }
    counts
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

/// Remove empty LaTeX groups (`{}`) from a source token. They produce no
/// glyph — `workf{}lows` is the recommended fix for the ligature `workflows`,
/// so it must be searched for as `workflows`, not penalized for following
/// the tool's own suggestion.
fn strip_empty_groups(word: &str) -> String {
    word.replace("{}", "")
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

/// Font embedding + Info date checks (TF15) over an already-parsed [`PdfInfo`].
///
/// Silent when every referenced font is embedded and present dates match
/// [`PDF_DATE_EXPECTED`]. Findings are always [`Severity::Warning`].
pub fn quality_findings(info: &PdfInfo) -> Vec<LintFinding> {
    let mut findings = Vec::new();
    findings.extend(font_embedding_findings(&info.fonts));
    findings.extend(metadata_date_findings(&info.metadata));
    findings
}

/// Run TF15 quality checks on a PDF path.
pub fn check_quality(pdf_path: &Path) -> Result<Vec<LintFinding>> {
    let info = pdf_info(pdf_path)?;
    Ok(quality_findings(&info))
}

/// Warn for every font referenced by the PDF that is not embedded.
fn font_embedding_findings(fonts: &[PdfFontInfo]) -> Vec<LintFinding> {
    fonts
        .iter()
        // Only page-referenced fonts (Type0/simple faces in Resources).
        // Descendant CID entries have empty `pages` and are covered by the parent.
        .filter(|f| !f.embedded && !f.pages.is_empty())
        .map(|f| {
            let pages = format_page_list(&f.pages);
            LintFinding {
                file: "pdf".into(),
                line: 0,
                severity: Severity::Warning,
                message: format!(
                    "font `{}` is referenced but not embedded (pages: {})",
                    f.name, pages
                ),
                suggestion: Some(
                    "Embed the font at compile time so the PDF travels; viewers on other \
                     machines may substitute a different face"
                        .into(),
                ),
            }
        })
        .collect()
}

fn format_page_list(pages: &[usize]) -> String {
    if pages.is_empty() {
        return "unknown".into();
    }
    pages
        .iter()
        .map(|p| p.to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

/// Warn when `/CreationDate` or `/ModDate` is present but not PDF-shaped.
fn metadata_date_findings(meta: &PdfMetadata) -> Vec<LintFinding> {
    let mut findings = Vec::new();
    for (field, value) in [
        ("CreationDate", meta.creation_date.as_deref()),
        ("ModDate", meta.mod_date.as_deref()),
    ] {
        let Some(observed) = value else {
            continue;
        };
        if is_valid_pdf_date(observed) {
            continue;
        }
        findings.push(LintFinding {
            file: "pdf".into(),
            line: 0,
            severity: Severity::Warning,
            message: format!(
                "/{field} value `{observed}` is not a valid PDF date (expected {PDF_DATE_EXPECTED})"
            ),
            suggestion: Some(format!(
                "Set pdf{field} to a PDF date string like `{PDF_DATE_EXPECTED}` \
                 (e.g. via hyperref); avoid `\\today` which expands to a human date"
            )),
        });
    }
    findings
}

/// True when `value` matches `D:YYYYMMDDHHmmSS` with an optional timezone suffix.
///
/// Accepts trailing `Z` / `z` or `±HH'mm'` as allowed by ISO 32000.
pub fn is_valid_pdf_date(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() < 16 {
        return false;
    }
    if &bytes[..2] != b"D:" {
        return false;
    }
    if !bytes[2..16].iter().all(u8::is_ascii_digit) {
        return false;
    }
    match &bytes[16..] {
        [] | [b'Z'] | [b'z'] => true,
        [b'+' | b'-', h1, h2, b'\'', m1, m2, b'\'']
            if h1.is_ascii_digit()
                && h2.is_ascii_digit()
                && m1.is_ascii_digit()
                && m2.is_ascii_digit() =>
        {
            true
        }
        _ => false,
    }
}

/// Pages, fonts (with embedding), and Info metadata.
pub fn pdf_info(path: &Path) -> Result<PdfInfo> {
    let doc =
        Document::load(path).with_context(|| format!("failed to open PDF {}", path.display()))?;
    Ok(info_from_document(&doc))
}

/// Pages, fonts, and metadata from an already-loaded document / bytes.
#[allow(dead_code)]
pub fn pdf_info_from_bytes(data: &[u8]) -> Result<PdfInfo> {
    let doc = Document::load_mem(data).context("failed to parse PDF bytes")?;
    Ok(info_from_document(&doc))
}

fn info_from_document(doc: &Document) -> PdfInfo {
    let pages = doc.get_pages().len();
    let fonts = collect_fonts(doc);
    let metadata = read_metadata(doc);
    PdfInfo {
        pages,
        fonts,
        metadata,
    }
}

fn read_metadata(doc: &Document) -> PdfMetadata {
    let Ok(info_obj) = doc.trailer.get(b"Info") else {
        return PdfMetadata::default();
    };
    let dict = match info_obj {
        Object::Reference(id) => match doc.get_object(*id) {
            Ok(Object::Dictionary(d)) => d,
            _ => return PdfMetadata::default(),
        },
        Object::Dictionary(d) => d,
        _ => return PdfMetadata::default(),
    };
    PdfMetadata {
        title: dict_text(dict, b"Title"),
        author: dict_text(dict, b"Author"),
        subject: dict_text(dict, b"Subject"),
        keywords: dict_text(dict, b"Keywords"),
        creator: dict_text(dict, b"Creator"),
        producer: dict_text(dict, b"Producer"),
        creation_date: dict_text(dict, b"CreationDate"),
        mod_date: dict_text(dict, b"ModDate"),
    }
}

fn dict_text(dict: &Dictionary, key: &[u8]) -> Option<String> {
    let obj = dict.get(key).ok()?;
    object_to_string(obj)
}

fn object_to_string(obj: &Object) -> Option<String> {
    match obj {
        Object::String(bytes, _) => Some(decode_pdf_string(bytes)),
        Object::Name(n) => Some(String::from_utf8_lossy(n).into_owned()),
        _ => None,
    }
}

fn decode_pdf_string(bytes: &[u8]) -> String {
    // UTF-16BE with BOM
    if bytes.starts_with(&[0xFE, 0xFF]) && bytes.len() >= 2 {
        let u16s: Vec<u16> = bytes[2..]
            .chunks(2)
            .filter_map(|c| {
                if c.len() == 2 {
                    Some(u16::from_be_bytes([c[0], c[1]]))
                } else {
                    None
                }
            })
            .collect();
        return String::from_utf16_lossy(&u16s);
    }
    String::from_utf8_lossy(bytes).into_owned()
}

fn collect_fonts(doc: &Document) -> Vec<PdfFontInfo> {
    let mut fonts: BTreeMap<String, PdfFontInfo> = BTreeMap::new();
    for (page_num, page_id) in doc.get_pages() {
        collect_fonts_from_page(doc, page_num as usize, page_id, &mut fonts);
    }
    fonts.into_values().collect()
}

fn collect_fonts_from_page(
    doc: &Document,
    page_num: usize,
    page_id: ObjectId,
    fonts: &mut BTreeMap<String, PdfFontInfo>,
) {
    let Ok(Object::Dictionary(page)) = doc.get_object(page_id) else {
        return;
    };
    let Some(resources) = resolve_dict(doc, page.get(b"Resources").ok()) else {
        return;
    };
    let Some(font_dict) = resolve_dict(doc, resources.get(b"Font").ok()) else {
        return;
    };
    for (_, obj) in font_dict.iter() {
        let font_obj = match obj {
            Object::Reference(id) => doc.get_object(*id).ok(),
            other => Some(other),
        };
        let Some(Object::Dictionary(font)) = font_obj else {
            continue;
        };
        push_font(doc, font, Some(page_num), fonts);
    }
}

fn push_font(
    doc: &Document,
    font: &Dictionary,
    page_num: Option<usize>,
    fonts: &mut BTreeMap<String, PdfFontInfo>,
) {
    let subtype = font
        .get(b"Subtype")
        .ok()
        .and_then(object_to_string)
        .unwrap_or_else(|| "Unknown".into());

    // Type0: report the parent BaseFont; embedding lives on the descendant.
    let name = font
        .get(b"BaseFont")
        .ok()
        .and_then(object_to_string)
        .unwrap_or_else(|| "(unnamed)".into());

    let embedded = font_is_embedded(doc, font);
    fonts
        .entry(name.clone())
        .and_modify(|f| {
            if embedded {
                f.embedded = true;
            }
            if let Some(p) = page_num {
                if !f.pages.contains(&p) {
                    f.pages.push(p);
                    f.pages.sort_unstable();
                }
            }
        })
        .or_insert_with(|| {
            let pages = match page_num {
                Some(p) => vec![p],
                None => Vec::new(),
            };
            PdfFontInfo {
                name,
                subtype,
                embedded,
                pages,
            }
        });

    // Also walk DescendantFonts for CID fonts (no page attribution — the
    // parent Type0 already recorded the pages that reference the face).
    if let Ok(obj) = font.get(b"DescendantFonts") {
        let arr = match obj {
            Object::Array(a) => Some(a.as_slice()),
            Object::Reference(id) => match doc.get_object(*id) {
                Ok(Object::Array(a)) => Some(a.as_slice()),
                _ => None,
            },
            _ => None,
        };
        if let Some(items) = arr {
            for item in items {
                let desc = match item {
                    Object::Reference(id) => doc.get_object(*id).ok(),
                    other => Some(other),
                };
                if let Some(Object::Dictionary(d)) = desc {
                    push_font(doc, d, None, fonts);
                }
            }
        }
    }
}

fn font_is_embedded(doc: &Document, font: &Dictionary) -> bool {
    if let Some(desc) = resolve_dict(doc, font.get(b"FontDescriptor").ok()) {
        for key in [b"FontFile".as_slice(), b"FontFile2", b"FontFile3"] {
            if desc.get(key).is_ok() {
                return true;
            }
        }
    }

    // Type0 has no descriptor; embedding lives on DescendantFonts.
    let Ok(obj) = font.get(b"DescendantFonts") else {
        return false;
    };
    let arr = match obj {
        Object::Array(a) => Some(a.as_slice()),
        Object::Reference(id) => match doc.get_object(*id) {
            Ok(Object::Array(a)) => Some(a.as_slice()),
            _ => None,
        },
        _ => None,
    };
    let Some(items) = arr else {
        return false;
    };
    items.iter().any(|item| {
        let desc = match item {
            Object::Reference(id) => doc.get_object(*id).ok(),
            other => Some(other),
        };
        match desc {
            Some(Object::Dictionary(d)) => font_is_embedded(doc, d),
            _ => false,
        }
    })
}

fn resolve_dict<'a>(doc: &'a Document, obj: Option<&'a Object>) -> Option<&'a Dictionary> {
    match obj? {
        Object::Dictionary(d) => Some(d),
        Object::Reference(id) => match doc.get_object(*id).ok()? {
            Object::Dictionary(d) => Some(d),
            _ => None,
        },
        _ => None,
    }
}

/// Report which section opens each page.
///
/// `sections` is `(number, title)` in document order (e.g. from the outline).
/// A page is attributed to the *last section heading that begins on or
/// before that page*: for each section, in order, we search forward from
/// the page where the previous *matched* section was found for the first
/// page whose text contains this section's title.
///
/// Matching keys on page position, not on a strict in-order text-equality
/// chain: a title that fails to match anywhere (for example a heading whose
/// rendered PDF form diverges from its resolved source text — dot-leader
/// alignment, a substituted dash, ...) is skipped rather than permanently
/// blocking every section that follows it. Skipping a title does not move
/// the search position, so later sections still search from the last page
/// that *did* match.
pub fn page_breaks(page_texts: &[String], sections: &[(String, String)]) -> Vec<PdfPageBreak> {
    let normalized_pages: Vec<String> = page_texts.iter().map(|p| normalize_pdf_text(p)).collect();

    // (page index, number, title) for each section that could be located.
    let mut matches: Vec<(usize, String, String)> = Vec::new();
    let mut search_from = 0usize;
    for (num, title) in sections {
        let found = normalized_pages
            .iter()
            .enumerate()
            .skip(search_from)
            .find(|(_, text)| section_title_in_page(text, title))
            .map(|(idx, _)| idx);
        if let Some(page_idx) = found {
            matches.push((page_idx, num.clone(), title.clone()));
            search_from = page_idx;
        }
    }

    let mut out = Vec::with_capacity(page_texts.len());
    let mut current: Option<(String, String)> = None;
    let mut match_idx = 0usize;
    for i in 0..page_texts.len() {
        while match_idx < matches.len() && matches[match_idx].0 <= i {
            let (_, num, title) = &matches[match_idx];
            current = Some((num.clone(), title.clone()));
            match_idx += 1;
        }
        out.push(PdfPageBreak {
            page: i + 1,
            section: current.as_ref().map(|(n, _)| n.clone()),
            title: current.as_ref().map(|(_, t)| t.clone()),
        });
    }
    out
}

fn section_title_in_page(page_text: &str, title: &str) -> bool {
    let needle = normalize_pdf_text(title);
    if needle.is_empty() {
        return false;
    }
    page_text.contains(needle.as_str())
}

/// Format page breaks for diff-friendly output: one line per page.
pub fn format_page_breaks(breaks: &[PdfPageBreak]) -> String {
    let mut lines = Vec::with_capacity(breaks.len());
    for b in breaks {
        match (&b.section, &b.title) {
            (Some(num), Some(title)) => {
                lines.push(format!("page={} section={} title={}", b.page, num, title));
            }
            _ => lines.push(format!("page={} section= title=", b.page)),
        }
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::texparse::TokenizedFile;
    use std::path::PathBuf;

    const LIGATURES_PDF: &[u8] = include_bytes!("../tests/fixtures/ligatures.pdf");
    const PAGES_PDF: &[u8] = include_bytes!("../tests/fixtures/pages-ligatures.pdf");
    const MALFORMED_DATE_PDF: &[u8] = include_bytes!("../tests/fixtures/malformed-date.pdf");

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

    #[test]
    fn pages_fixture_has_hyphenation_and_sections() {
        let pages = extract_text_by_pages_from_bytes(PAGES_PDF).unwrap();
        assert_eq!(pages.len(), 2);
        assert!(pages[0].contains('\u{FB01}') || pages[0].contains("Learn-"));
        let norm = normalize_pdf_text(&pages[0]);
        assert!(norm.contains("Deep Learning"), "got {norm:?}");
        assert!(pages[0].contains("Introduction"));
        assert!(pages[1].contains("Methods"));
    }

    #[test]
    fn page_breaks_are_machine_readable() {
        let pages = extract_text_by_pages_from_bytes(PAGES_PDF).unwrap();
        let sections = vec![
            ("1".into(), "Introduction".into()),
            ("2".into(), "Methods".into()),
        ];
        let breaks = page_breaks(&pages, &sections);
        let formatted = format_page_breaks(&breaks);
        assert_eq!(
            formatted,
            "page=1 section=1 title=Introduction\npage=2 section=2 title=Methods"
        );
    }

    #[test]
    fn page_breaks_skip_a_permanently_unmatched_section_between_matches() {
        // TE5: a heading between two matchable sections whose resolved title
        // never appears verbatim in any page's extracted text (e.g. a résumé
        // job-date line built with a dot leader, which renders as a row of
        // dots in the PDF but collapses to plain text once resolved) must
        // not permanently block later sections from being found.
        let pages = extract_text_by_pages_from_bytes(PAGES_PDF).unwrap();
        let sections = vec![
            ("1".into(), "Introduction".into()),
            (
                "1.1".into(),
                "AI Engineer en Accenture Julio 2026 -- Actual".into(),
            ),
            ("2".into(), "Methods".into()),
        ];
        let breaks = page_breaks(&pages, &sections);
        let formatted = format_page_breaks(&breaks);
        assert_eq!(
            formatted,
            "page=1 section=1 title=Introduction\npage=2 section=2 title=Methods"
        );
    }

    #[test]
    fn pdf_info_reports_pages_fonts_and_metadata() {
        let info = pdf_info_from_bytes(LIGATURES_PDF).unwrap();
        assert_eq!(info.pages, 1);
        assert!(!info.fonts.is_empty());
        assert!(
            info.fonts.iter().all(|f| f.embedded),
            "fixture fonts should all be embedded: {:?}",
            info.fonts
        );
        assert!(
            info.fonts.iter().any(|f| f.pages == vec![1]),
            "page-level fonts should record page 1: {:?}",
            info.fonts
        );
        assert_eq!(info.metadata.creator.as_deref(), Some("tectonic"));
    }

    #[test]
    fn embedded_fixture_is_silent_for_quality_checks() {
        let info = pdf_info_from_bytes(LIGATURES_PDF).unwrap();
        let findings = quality_findings(&info);
        assert!(
            findings.is_empty(),
            "embedded fonts + well-formed dates must be silent: {findings:?}"
        );
    }

    #[test]
    fn malformed_date_fixture_warns_with_expected_shape() {
        let info = pdf_info_from_bytes(MALFORMED_DATE_PDF).unwrap();
        assert_eq!(
            info.metadata.creation_date.as_deref(),
            Some("July 28, 2026")
        );
        assert_eq!(info.metadata.mod_date.as_deref(), Some("August 7, 2026"));

        let findings = quality_findings(&info);
        let date_findings: Vec<_> = findings
            .iter()
            .filter(|f| f.message.contains("CreationDate") || f.message.contains("ModDate"))
            .collect();
        assert_eq!(date_findings.len(), 2, "{findings:?}");
        for f in &date_findings {
            assert_eq!(f.severity, Severity::Warning);
            assert!(
                f.message.contains(PDF_DATE_EXPECTED),
                "must name expected shape: {}",
                f.message
            );
        }
        assert!(date_findings
            .iter()
            .any(|f| f.message.contains("July 28, 2026")));
        assert!(date_findings
            .iter()
            .any(|f| f.message.contains("August 7, 2026")));
    }

    #[test]
    fn non_embedded_font_warns_with_page_list() {
        let info = pdf_info_from_bytes(MALFORMED_DATE_PDF).unwrap();
        let helvetica = info
            .fonts
            .iter()
            .find(|f| f.name == "Helvetica")
            .expect("Helvetica present");
        assert!(!helvetica.embedded);
        assert_eq!(helvetica.pages, vec![1, 2]);

        let findings = quality_findings(&info);
        let font_finding = findings
            .iter()
            .find(|f| f.message.contains("Helvetica") && f.message.contains("not embedded"))
            .expect("non-embedded font warning");
        assert_eq!(font_finding.severity, Severity::Warning);
        assert!(
            font_finding.message.contains("pages: 1, 2"),
            "{}",
            font_finding.message
        );
    }

    #[test]
    fn pdf_date_accepts_spec_shape_and_timezone() {
        assert!(is_valid_pdf_date("D:20260807144421"));
        assert!(is_valid_pdf_date("D:20260807144421Z"));
        assert!(is_valid_pdf_date("D:20260807144421-00'00'"));
        assert!(is_valid_pdf_date("D:20260807144421+05'30'"));
        assert!(!is_valid_pdf_date("July 28, 2026"));
        assert!(!is_valid_pdf_date("D:20260807"));
        assert!(!is_valid_pdf_date(""));
    }

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
