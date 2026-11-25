//! Raw text extraction from the PDF, plus ligature/hyphen normalization.

use std::any::Any;
use std::path::Path;
use std::sync::Mutex;

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

/// Load, optionally sanitize, and return a document for extraction.
///
/// The four extractors below all pass `true`: form `XObject` fonts are given
/// per-form resource names so a figure reusing a page's font name no longer
/// decodes with the page font's map (or vice versa). Passing `false` runs
/// the same fixture unsanitized, which lets a test prove the fixture
/// reproduces the defect.
pub(crate) fn extract_doc(mut doc: lopdf::Document, sanitize: bool) -> lopdf::Document {
    if sanitize {
        super::sanitize::rename_form_fonts(&mut doc);
    }
    doc
}

/// Load one document, decrypting with the empty password when it reports
/// being encrypted (the same empty-password attempt `pdf-extract` makes).
fn load_doc(
    doc: Result<lopdf::Document, lopdf::Error>,
) -> Result<lopdf::Document, pdf_extract::OutputError> {
    let mut loaded = doc?;
    if loaded.is_encrypted() {
        loaded
            .decrypt("")
            .map_err(pdf_extract::OutputError::PdfError)?;
    }
    Ok(loaded)
}

/// Extract the whole text of one loaded document.
fn whole_text(doc: &lopdf::Document) -> Result<String, pdf_extract::OutputError> {
    let mut text = String::new();
    {
        let mut output = pdf_extract::PlainTextOutput::new(&mut text);
        pdf_extract::output_doc(doc, &mut output)?;
    }
    Ok(text)
}

/// Extract page `page_num` (1-based) of one loaded document.
fn page_text(doc: &lopdf::Document, page_num: u32) -> Result<String, pdf_extract::OutputError> {
    let mut text = String::new();
    {
        let mut output = pdf_extract::PlainTextOutput::new(&mut text);
        pdf_extract::output_doc_page(doc, &mut output, page_num)?;
    }
    Ok(text)
}

/// Extract every page in order, stopping at the first page number that
/// does not exist (the same loop `pdf-extract` itself uses).
fn all_pages(doc: &lopdf::Document) -> Result<Vec<String>, pdf_extract::OutputError> {
    let mut pages = Vec::new();
    let mut page_num = 1;
    while let Ok(text) = page_text(doc, page_num) {
        pages.push(text);
        page_num += 1;
    }
    Ok(pages)
}

/// Extract raw PDF text (ligatures left as `ToUnicode` mapped them).
pub fn extract_text(path: &Path) -> Result<String> {
    match guarded(|| whole_text(&extract_doc(load_doc(lopdf::Document::load(path))?, true))) {
        Ok(text) => Ok(text),
        Err(GuardError::Extract(message)) => Err(anyhow::anyhow!(
            "failed to extract text from {}: {message}",
            path.display()
        )),
        Err(GuardError::Panicked(message)) => Err(anyhow::anyhow!(
            "failed to extract text from {}: {INCOMPLETE_MAP} ({message})",
            path.display()
        )),
    }
}

/// Extract raw PDF text from bytes.
#[allow(dead_code)]
pub fn extract_text_from_bytes(data: &[u8]) -> Result<String> {
    match guarded(|| {
        whole_text(&extract_doc(
            load_doc(lopdf::Document::load_mem(data))?,
            true,
        ))
    }) {
        Ok(text) => Ok(text),
        Err(GuardError::Extract(message)) => Err(anyhow::anyhow!(
            "failed to extract text from PDF bytes: {message}"
        )),
        Err(GuardError::Panicked(message)) => Err(anyhow::anyhow!(
            "failed to extract text from PDF bytes: {INCOMPLETE_MAP} ({message})"
        )),
    }
}

/// Extract raw text one page at a time (1-based order).
pub fn extract_text_by_pages(path: &Path) -> Result<Vec<String>> {
    match guarded(|| all_pages(&extract_doc(load_doc(lopdf::Document::load(path))?, true))) {
        Ok(pages) => Ok(pages),
        Err(GuardError::Extract(message)) => Err(anyhow::anyhow!(
            "failed to extract per-page text from {}: {message}",
            path.display()
        )),
        Err(GuardError::Panicked(message)) => Err(anyhow::anyhow!(
            "failed to extract per-page text from {}: {INCOMPLETE_MAP} ({message})",
            path.display()
        )),
    }
}

/// Extract raw text one page at a time from bytes.
///
/// Used by the fixture tests and by callers that already hold the PDF in memory.
#[allow(dead_code)]
pub fn extract_text_by_pages_from_bytes(data: &[u8]) -> Result<Vec<String>> {
    match guarded(|| {
        all_pages(&extract_doc(
            load_doc(lopdf::Document::load_mem(data))?,
            true,
        ))
    }) {
        Ok(pages) => Ok(pages),
        Err(GuardError::Extract(message)) => Err(anyhow::anyhow!(
            "failed to extract per-page text from PDF bytes: {message}"
        )),
        Err(GuardError::Panicked(message)) => Err(anyhow::anyhow!(
            "failed to extract per-page text from PDF bytes: {INCOMPLETE_MAP} ({message})"
        )),
    }
}

/// The cause attached to a caught `pdf-extract` panic: the crate aborts with
/// `expect("missing unicode map and encoding")` when a simple font carries a
/// `/ToUnicode` map that lacks a used character code and no `/Encoding`.
const INCOMPLETE_MAP: &str = "the PDF has a font with a ToUnicode map that lacks a used \
    character and no encoding";

/// Serializes the panic-hook swap in [`guarded`]: without it, two threads
/// would install and restore the silent hook around each other and a panic
/// message could leak onto stderr (or the wrong hook could be restored).
static GUARD: Mutex<()> = Mutex::new(());

/// How a guarded `pdf-extract` call failed. The split keeps ordinary
/// extraction errors on their existing messages while panics get the
/// [`INCOMPLETE_MAP`] cause; callers format each side.
#[derive(Debug)]
enum GuardError {
    Extract(String),
    Panicked(String),
}

/// Run a `pdf-extract` call, turning a panic into an [`GuardError::Panicked`]
/// instead of letting it unwind into the caller. The default panic hook is
/// silenced for the duration so no `thread '...' panicked at` line reaches
/// the user; the previous hook is always restored.
fn guarded<T>(f: impl FnOnce() -> Result<T, pdf_extract::OutputError>) -> Result<T, GuardError> {
    let _lock = GUARD
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let previous_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
    std::panic::set_hook(previous_hook);
    match outcome {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(e)) => Err(GuardError::Extract(e.to_string())),
        Err(payload) => Err(GuardError::Panicked(panic_message(payload))),
    }
}

/// Read a panic payload the way the hook would print it: `&str` and `String`
/// payloads keep their text, everything else becomes `unknown panic`.
/// Takes the box by value and downcasts by value: on this toolchain only
/// the `Box` receiver downcasts correctly, a `&(dyn Any + Send)` receiver
/// misses `&str`/`String` payloads.
fn panic_message(payload: Box<dyn Any + Send>) -> String {
    match payload.downcast::<&str>() {
        Ok(message) => message.to_string(),
        Err(payload) => match payload.downcast::<String>() {
            Ok(message) => message.to_string(),
            Err(_) => "unknown panic".to_string(),
        },
    }
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
    use super::super::fixtures::{
        font_name_clash_compressed_pdf, font_name_clash_pdf, font_name_clash_shared_resources_pdf,
        font_name_clash_two_forms_pdf, missing_unicode_map_pdf,
        missing_unicode_map_with_encoding_pdf, LIGATURES_PDF,
    };
    use super::super::sanitize::rename_form_fonts;
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

    /// The panicking fixture must surface as an `Err` naming the cause, not
    /// as a panic: the outer `catch_unwind` proves the guard caught it (an
    /// escaping panic would make the outer guard return `Err` and fail the
    /// `expect`), and the silenced hook keeps stderr quiet.
    #[test]
    fn missing_unicode_map_bytes_returns_tounicode_error() {
        let outcome =
            std::panic::catch_unwind(|| extract_text_from_bytes(&missing_unicode_map_pdf()));
        let result = outcome.expect("guard must not let the pdf-extract panic escape");
        let err = result.expect_err("incomplete ToUnicode map must fail extraction");
        let message = err.to_string();
        assert!(
            message.contains("ToUnicode"),
            "error must name the cause: {message}"
        );
        assert!(
            message.contains("no encoding"),
            "error must name the cause: {message}"
        );
        assert!(
            message.contains("missing unicode map and encoding"),
            "error must carry the extractor panic payload: {message}"
        );
        assert!(
            message.contains("PDF bytes"),
            "bytes variant must say PDF bytes: {message}"
        );
    }

    #[test]
    fn missing_unicode_map_pages_bytes_returns_tounicode_error() {
        let outcome = std::panic::catch_unwind(|| {
            extract_text_by_pages_from_bytes(&missing_unicode_map_pdf())
        });
        let result = outcome.expect("guard must not let the pdf-extract panic escape");
        let err = result.expect_err("incomplete ToUnicode map must fail extraction");
        let message = err.to_string();
        assert!(
            message.contains("ToUnicode"),
            "error must name the cause: {message}"
        );
        assert!(
            message.contains("per-page text"),
            "per-page variant must say per-page text: {message}"
        );
        assert!(
            message.contains("PDF bytes"),
            "bytes variant must say PDF bytes: {message}"
        );
    }

    #[test]
    fn missing_unicode_map_with_encoding_extracts_ab() {
        let text = extract_text_from_bytes(&missing_unicode_map_with_encoding_pdf())
            .expect("encoding variant must extract");
        assert!(text.contains("AB"), "expected AB, got {text:?}");
        let pages = extract_text_by_pages_from_bytes(&missing_unicode_map_with_encoding_pdf())
            .expect("encoding variant must extract per page");
        assert!(
            pages.iter().any(|page| page.contains("AB")),
            "expected AB on some page, got {pages:?}"
        );
    }

    /// Path variants report the file they failed on.
    #[test]
    fn missing_unicode_map_path_errors_name_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing-map.pdf");
        std::fs::write(&path, missing_unicode_map_pdf()).unwrap();
        let err = extract_text(&path).expect_err("incomplete ToUnicode map must fail");
        let message = err.to_string();
        assert!(
            message.contains("ToUnicode"),
            "error must name the cause: {message}"
        );
        assert!(
            message.contains(&*path.to_string_lossy()),
            "error must name the file: {message}"
        );
        let err = extract_text_by_pages(&path).expect_err("incomplete ToUnicode map must fail");
        let message = err.to_string();
        assert!(
            message.contains("ToUnicode"),
            "error must name the cause: {message}"
        );
        assert!(
            message.contains(&*path.to_string_lossy()),
            "error must name the file: {message}"
        );
    }

    /// Two threads through the guard at once (one panicking, one fine) must
    /// each get their own correct outcome: the hook swap is serialized.
    #[test]
    fn concurrent_guarded_calls_stay_correct() {
        let panicking = std::thread::spawn(|| {
            guarded(|| -> Result<String, pdf_extract::OutputError> {
                std::panic::panic_any("boom")
            })
        });
        let fine = std::thread::spawn(|| {
            guarded(|| Ok::<String, pdf_extract::OutputError>("ok".to_string()))
        });
        let panicked = panicking.join().expect("panicking thread must finish");
        assert!(
            matches!(panicked, Err(GuardError::Panicked(ref message)) if message.contains("boom")),
            "panicking call must report its payload: {panicked:?}"
        );
        let ok = fine.join().expect("fine thread must finish");
        assert!(
            matches!(ok, Ok(ref value) if value == "ok"),
            "clean call must pass through: {ok:?}"
        );
    }

    /// Extract `data` with (`true`) or without (`false`) the form-font
    /// rename, through the same load path the extractors use.
    fn text_with_sanitize(data: &[u8], sanitize: bool) -> String {
        let doc = lopdf::Document::load_mem(data).expect("fixture must load");
        let doc = extract_doc(doc, sanitize);
        let mut text = String::new();
        {
            let mut output = pdf_extract::PlainTextOutput::new(&mut text);
            pdf_extract::output_doc(&doc, &mut output).expect("fixture must extract");
        }
        text
    }

    /// How often the clash letters appear: page draws `P`, the form `Q`.
    fn clash_counts(text: &str) -> (usize, usize) {
        (
            text.chars().filter(|&c| c == 'P').count(),
            text.chars().filter(|&c| c == 'Q').count(),
        )
    }

    /// Without the rename at least one of the two operator orders must
    /// collapse the two fonts into two equal letters: the fixture has to
    /// reproduce the defect, otherwise it proves nothing.
    #[test]
    fn clash_fixture_reproduces_the_defect_unsanitized() {
        let page_first = text_with_sanitize(&font_name_clash_pdf(false), false);
        let form_first = text_with_sanitize(&font_name_clash_pdf(true), false);
        let collides = |text: &str| {
            let (p, q) = clash_counts(text);
            p == 2 || q == 2
        };
        assert!(
            collides(&page_first) || collides(&form_first),
            "fixture must reproduce the defect: one order must read PP or QQ, \
             got page-first {page_first:?} and form-first {form_first:?}"
        );
    }

    /// With the rename both orders read exactly one `P` (page) and one `Q`
    /// (form), in operator order.
    #[test]
    fn clash_fixture_decodes_both_fonts_sanitized() {
        for form_first in [false, true] {
            let text = text_with_sanitize(&font_name_clash_pdf(form_first), true);
            assert_eq!(
                clash_counts(&text),
                (1, 1),
                "expected one P and one Q with form_first={form_first}, got {text:?}"
            );
            let (p, q) = (text.find('P').unwrap(), text.find('Q').unwrap());
            if form_first {
                assert!(q < p, "form draws first, so Q precedes P: {text:?}");
            } else {
                assert!(p < q, "page draws first, so P precedes Q: {text:?}");
            }
        }
        // The public entry point sanitizes too.
        let text = extract_text_from_bytes(&font_name_clash_pdf(false)).unwrap();
        assert_eq!(clash_counts(&text), (1, 1), "public path: {text:?}");
    }

    /// The rename reports one renamed form for both the raw and the
    /// compressed fixture, and nothing for a fixture without form fonts.
    #[test]
    fn rename_report_counts_renamed_and_untouched() {
        for data in [font_name_clash_pdf(false), font_name_clash_compressed_pdf()] {
            let mut doc = lopdf::Document::load_mem(&data).expect("fixture must load");
            let report = rename_form_fonts(&mut doc);
            assert_eq!(report.renamed, 1, "one form font must be renamed");
            assert_eq!(report.skipped, 0, "no form may be skipped");
        }
        let mut ligatures = lopdf::Document::load_mem(LIGATURES_PDF).expect("fixture must load");
        let report = rename_form_fonts(&mut ligatures);
        assert_eq!(report.renamed, 0, "no form fonts to rename");
        assert_eq!(report.skipped, 0, "no form may be skipped");
    }

    /// The compressed form still decodes after the rename and no longer
    /// carries its `/FlateDecode` filter.
    #[test]
    fn compressed_form_decodes_without_filter_after_rename() {
        let mut doc = lopdf::Document::load_mem(&font_name_clash_compressed_pdf())
            .expect("fixture must load");
        let before = form_filter(&doc);
        assert_eq!(
            before.as_deref(),
            Some("FlateDecode"),
            "fixture must start compressed"
        );
        rename_form_fonts(&mut doc);
        assert!(
            form_filter(&doc).is_none(),
            "renamed content must be stored plain"
        );
        let content = form_content_bytes(&doc);
        let operations =
            lopdf::content::Content::decode(&content).expect("renamed content must decode");
        let renamed_tf = operations
            .operations
            .iter()
            .filter(|op| op.operator == "Tf")
            .count();
        assert_eq!(renamed_tf, 1, "form keeps its single Tf operator");
        let text = text_with_sanitize(&font_name_clash_compressed_pdf(), true);
        assert_eq!(clash_counts(&text), (1, 1), "compressed: {text:?}");
    }

    /// `/Filter` of the single form stream in the compressed fixture.
    fn form_filter(doc: &lopdf::Document) -> Option<String> {
        for object in doc.objects.values() {
            if let Ok(stream) = object.as_stream() {
                let is_form = stream
                    .dict
                    .get(b"Subtype")
                    .and_then(lopdf::Object::as_name)
                    .is_ok_and(|name| name == b"Form");
                if is_form {
                    return stream
                        .dict
                        .get(b"Filter")
                        .and_then(lopdf::Object::as_name)
                        .ok()
                        .map(|name| String::from_utf8_lossy(name).into_owned());
                }
            }
        }
        None
    }

    /// Decompressed bytes of the single form stream in the fixture.
    fn form_content_bytes(doc: &lopdf::Document) -> Vec<u8> {
        for object in doc.objects.values() {
            if let Ok(stream) = object.as_stream() {
                let is_form = stream
                    .dict
                    .get(b"Subtype")
                    .and_then(lopdf::Object::as_name)
                    .is_ok_and(|name| name == b"Form");
                if is_form {
                    return stream.decompressed_content().expect("form must decode");
                }
            }
        }
        panic!("fixture must contain a form stream");
    }

    /// Two forms reusing the page's `F1` for different fonts decode as
    /// exactly one `P` (page), one `Q` (`M1`) and one `R` (`M2`).
    #[test]
    fn two_clashing_forms_each_decode_with_their_own_font() {
        let text = extract_text_from_bytes(&font_name_clash_two_forms_pdf()).unwrap();
        for letter in ['P', 'Q', 'R'] {
            assert_eq!(
                text.chars().filter(|&c| c == letter).count(),
                1,
                "expected exactly one {letter}: {text:?}"
            );
        }
    }

    /// A form without its own `/Resources` keeps using the page's font: both
    /// `A` letters read as `P` and nothing is renamed.
    #[test]
    fn form_without_resources_keeps_the_page_font() {
        let mut doc =
            lopdf::Document::load_mem(&font_name_clash_pdf(false)).expect("fixture must load");
        doc.get_object_mut((8, 0))
            .expect("form must exist")
            .as_stream_mut()
            .expect("form must be a stream")
            .dict
            .remove(b"Resources");
        let report = rename_form_fonts(&mut doc);
        assert_eq!(report.renamed, 0, "resourceless form needs no rename");
        assert_eq!(report.skipped, 0, "resourceless form is not a failure");
        let doc = extract_doc(doc, true);
        let mut text = String::new();
        {
            let mut output = pdf_extract::PlainTextOutput::new(&mut text);
            pdf_extract::output_doc(&doc, &mut output).expect("rest must extract");
        }
        assert_eq!(
            text.chars().filter(|&c| c == 'P').count(),
            2,
            "both letters use the page font: {text:?}"
        );
        assert!(!text.contains('Q'), "no form font involved: {text:?}");
    }

    /// A form whose content cannot be decoded (bogus filter) is left
    /// untouched, counted as skipped, and the rest still extracts.
    #[test]
    fn undecodable_form_is_skipped_but_rest_extracts() {
        let mut doc =
            lopdf::Document::load_mem(&font_name_clash_pdf(false)).expect("fixture must load");
        let stream = doc
            .get_object_mut((8, 0))
            .expect("form must exist")
            .as_stream_mut()
            .expect("form must be a stream");
        stream.set_content(b"%garbage, not content operators".to_vec());
        stream
            .dict
            .set("Filter", lopdf::Object::Name(b"FooDecode".to_vec()));
        let report = rename_form_fonts(&mut doc);
        assert_eq!(report.renamed, 0, "nothing could be renamed");
        assert_eq!(report.skipped, 1, "one form must be skipped");
        let untouched = doc
            .get_object((8, 0))
            .expect("form must exist")
            .as_stream()
            .expect("form must be a stream");
        assert!(
            untouched
                .dict
                .get(b"Filter")
                .and_then(lopdf::Object::as_name)
                .is_ok_and(|name| name == b"FooDecode"),
            "skipped form must be left completely untouched"
        );
        let text = extract_text_from_bytes(&{
            let mut bytes = Vec::new();
            doc.save_to(&mut bytes).expect("fixture must save");
            bytes
        })
        .unwrap();
        assert!(text.contains('P'), "page text must still extract: {text:?}");
    }

    /// Two forms borrowing the same `/Resources` object each get their own
    /// names: the shared object keeps the old names for other borrowers, and
    /// both forms still decode with their own font.
    #[test]
    fn shared_form_resources_stay_independent() {
        let mut doc = lopdf::Document::load_mem(&font_name_clash_shared_resources_pdf())
            .expect("fixture must load");
        let report = rename_form_fonts(&mut doc);
        assert_eq!(report.renamed, 2, "both forms must be renamed");
        assert_eq!(report.skipped, 0, "no form may be skipped");
        let shared_font = doc
            .get_object((13, 0))
            .expect("shared resources must exist")
            .as_dict()
            .expect("shared resources must stay a dict")
            .get(b"Font")
            .expect("shared font entry must survive")
            .as_dict()
            .expect("shared font must stay a dict");
        assert!(
            shared_font.get(b"F1").is_ok(),
            "shared object must keep the old names: {shared_font:?}"
        );
        let mut bytes = Vec::new();
        doc.save_to(&mut bytes).expect("fixture must save");
        let text = extract_text_from_bytes(&bytes).unwrap();
        assert_eq!(
            text.chars().filter(|&c| c == 'P').count(),
            1,
            "page draws one P: {text:?}"
        );
        assert_eq!(
            text.chars().filter(|&c| c == 'Q').count(),
            2,
            "both forms draw Q: {text:?}"
        );
    }

    /// Per-page extraction matches the whole-text result on the fixture.
    #[test]
    fn by_pages_matches_whole_text_on_clash_fixture() {
        let pages = extract_text_by_pages_from_bytes(&font_name_clash_pdf(false)).unwrap();
        assert_eq!(pages.len(), 1, "one-page fixture: {pages:?}");
        assert_eq!(clash_counts(&pages[0]), (1, 1), "per-page: {:?}", pages[0]);
    }

    #[test]
    fn panic_message_reads_str_and_string_payloads() {
        let from_str =
            panic_message(Box::new("missing unicode map and encoding") as Box<dyn Any + Send>);
        assert_eq!(from_str, "missing unicode map and encoding");
        let from_string = panic_message(Box::new("boom".to_string()) as Box<dyn Any + Send>);
        assert_eq!(from_string, "boom");
        let unknown = panic_message(Box::new(42) as Box<dyn Any + Send>);
        assert_eq!(unknown, "unknown panic");
    }
}
