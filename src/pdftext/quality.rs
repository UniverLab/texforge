//! PDF quality checks: font embedding and Info-dictionary date shape.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result};
use lopdf::{Dictionary, Document, Object, ObjectId};

use crate::linter::{LintFinding, Severity};

use super::{PdfFontInfo, PdfInfo, PdfMetadata, PDF_DATE_EXPECTED};

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

pub(super) fn dict_text(dict: &Dictionary, key: &[u8]) -> Option<String> {
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

pub(super) fn resolve_dict<'a>(
    doc: &'a Document,
    obj: Option<&'a Object>,
) -> Option<&'a Dictionary> {
    match obj? {
        Object::Dictionary(d) => Some(d),
        Object::Reference(id) => match doc.get_object(*id).ok()? {
            Object::Dictionary(d) => Some(d),
            _ => None,
        },
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::super::fixtures::{LIGATURES_PDF, MALFORMED_DATE_PDF};
    use super::*;

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
}
