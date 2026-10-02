//! PDF text extraction and source-to-PDF fidelity (TF8).
//!
//! Extracts text the way a reader or ATS sees it, normalizes typographic
//! ligatures and hyphenated line breaks for comparison, and checks that every
//! significant source word (from the TF3 tokenizer) still appears in the PDF.
//! Missing words are reported as warnings; the source is never auto-edited.
//!
//! Also verifies font embedding and Info-dictionary date shape (TF15).

mod breaks;
mod extract;
mod fidelity;
mod outline;
mod quality;

#[cfg(test)]
mod fixtures;

pub use breaks::page_breaks;
pub use extract::{extract_text, extract_text_by_pages, normalize_pdf_text};
pub use fidelity::check_fidelity;
pub use outline::{format_page_breaks, page_breaks_from_outline, read_pdf_outline};
pub use quality::{check_quality, pdf_info};

/// PDF Info date shape required by ISO 32000 (`D:YYYYMMDDHHmmSS` plus optional TZ).
pub const PDF_DATE_EXPECTED: &str = "D:YYYYMMDDHHmmSS";

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

/// One entry in the PDF outline (bookmark tree).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PdfOutlineEntry {
    /// Section title from the outline.
    pub title: String,
    /// 1-based destination page number.
    pub page: usize,
    /// Nesting level (0 = top-level).
    pub level: usize,
}

/// Which path was used to derive the section-to-page mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageBreakSource {
    /// PDF outline (bookmark tree).
    Outline,
    /// Text matching against LaTeX-derived sections.
    TextMatch,
}

/// A distinct source word missing from the extracted PDF text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissingWord {
    pub word: String,
    pub count: usize,
}
