//! Pure-Rust PDF rasterization.
//!
//! Turns a compiled PDF into RGBA8 bitmaps with `hayro` — no C toolchain, no
//! `-sys` crates. One [`RenderCache`] is created per document and shared across
//! every page. Each rendered buffer also reports ink coverage, so blank,
//! nearly-blank and overfull pages are cheap to detect from the same pixels.

use std::path::Path;

use anyhow::{bail, Context, Result};
use hayro::hayro_interpret::InterpreterSettings;
use hayro::hayro_syntax::Pdf;
use hayro::vello_cpu::color::palette::css::WHITE;
use hayro::{render, RenderCache, RenderSettings};

/// A rasterized page: RGBA8 pixels plus ink coverage.
#[derive(Debug, Clone)]
pub struct RenderedPage {
    /// 0-based page index within the document.
    pub index: usize,
    /// Rendered width in pixels.
    pub width: usize,
    /// Rendered height in pixels.
    pub height: usize,
    /// RGBA8 pixels, row-major, straight alpha with alpha forced to 255.
    pub rgba: Vec<u8>,
    /// Fraction of the page covered by ink, computed from the same buffer.
    pub ink_coverage: f64,
}

/// A parsed PDF document with one render cache shared across all its pages.
///
/// The lifetime parameter is internal plumbing: the cache is invariant over it,
/// so rendering takes `&'a self` to unify the page and cache borrows.
pub struct PdfDocument<'a> {
    pdf: Pdf,
    cache: RenderCache<'a>,
    settings: InterpreterSettings,
}

impl<'a> PdfDocument<'a> {
    /// Parse the PDF at `path`.
    pub fn open(path: &Path) -> Result<Self> {
        let data =
            std::fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
        Self::from_bytes(data).with_context(|| format!("{} is not a readable PDF", path.display()))
    }

    /// Parse a PDF from raw bytes.
    pub fn from_bytes(data: Vec<u8>) -> Result<Self> {
        let pdf = Pdf::new(data).map_err(|_| anyhow::anyhow!("invalid or encrypted PDF"))?;
        Ok(Self {
            pdf,
            cache: RenderCache::new(),
            settings: InterpreterSettings::default(),
        })
    }

    /// Number of pages in the document.
    pub fn page_count(&self) -> usize {
        self.pdf.pages().len()
    }

    /// Logical size of page `index` in PDF points.
    ///
    /// Exercised by the module tests and consumed by the visual-regression and
    /// page-break specs; the bin only renders, so it looks dead here.
    #[allow(dead_code)]
    pub fn page_dimensions(&self, index: usize) -> Result<(usize, usize)> {
        let page = self
            .pdf
            .pages()
            .get(index)
            .with_context(|| self.out_of_range(index))?;
        let (w, h) = page.render_dimensions();
        Ok((w.round() as usize, h.round() as usize))
    }

    /// The scale that maps `page_width` PDF points onto `target_width` pixels.
    ///
    /// Exercised by the module tests and consumed by the visual-regression and
    /// page-break specs; the bin only renders, so it looks dead here.
    #[allow(dead_code)]
    pub fn scale_for_width(page_width: f32, target_width: usize) -> f32 {
        target_width as f32 / page_width.max(1.0)
    }

    /// Rasterize page `index` at `scale` (pixels per PDF point).
    pub fn render_page(&'a self, index: usize, scale: f32) -> Result<RenderedPage> {
        let page = self
            .pdf
            .pages()
            .get(index)
            .with_context(|| self.out_of_range(index))?;
        let (w, h) = page.render_dimensions();
        if w <= 0.0 || h <= 0.0 {
            bail!("page {} has invalid dimensions", index + 1);
        }
        let pixmap = render(
            page,
            &self.cache,
            &self.settings,
            &RenderSettings {
                x_scale: scale,
                y_scale: scale,
                bg_color: WHITE,
                ..Default::default()
            },
        );
        let (width, height) = (pixmap.width() as usize, pixmap.height() as usize);
        let mut rgba = pixmap.data_as_u8_slice().to_vec();
        // Pages render on an opaque white base, so premultiplied equals direct;
        // force alpha anyway so compositors see a solid channel.
        for px in rgba.as_chunks_mut::<4>().0 {
            px[3] = 255;
        }
        let ink_coverage = ink_coverage(&rgba);
        Ok(RenderedPage {
            index,
            width,
            height,
            rgba,
            ink_coverage,
        })
    }

    /// Rasterize page `index` at whatever scale fits it to `target_width` pixels.
    ///
    /// Exercised by the module tests and consumed by the visual-regression and
    /// page-break specs; the bin only renders, so it looks dead here.
    #[allow(dead_code)]
    pub fn render_page_to_width(
        &'a self,
        index: usize,
        target_width: usize,
    ) -> Result<RenderedPage> {
        let (page_width, _) = self.page_dimensions(index)?;
        let scale = Self::scale_for_width(page_width as f32, target_width);
        self.render_page(index, scale)
    }

    /// Rasterize every page at `scale`, sharing this document's render cache.
    ///
    /// Exercised by the module tests and consumed by the visual-regression and
    /// page-break specs; the bin only renders, so it looks dead here.
    #[allow(dead_code)]
    pub fn render_all(&'a self, scale: f32) -> Result<Vec<RenderedPage>> {
        (0..self.page_count())
            .map(|index| self.render_page(index, scale))
            .collect()
    }

    fn out_of_range(&self, index: usize) -> String {
        format!(
            "page {} does not exist (document has {} pages)",
            index + 1,
            self.page_count()
        )
    }
}

/// Fraction of a rendered RGBA8 buffer covered by ink: the average of
/// `1 - luminance` across pixels. Pure white reads `0.0`, solid black `1.0`,
/// and antialiased edges count proportionally.
pub fn ink_coverage(rgba: &[u8]) -> f64 {
    let mut ink = 0.0;
    for px in rgba.as_chunks::<4>().0 {
        let luminance = (px[0] as u32 + px[1] as u32 + px[2] as u32) / 3;
        ink += 1.0 - luminance as f64 / 255.0;
    }
    ink / (rgba.len() / 4) as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Committed fixture: two US-Letter pages; page 1 paints a black rectangle
    /// over the left half of the page, page 2 is blank.
    const FIXTURE: &[u8] = include_bytes!("../tests/fixtures/two-page.pdf");

    fn document<'a>() -> PdfDocument<'a> {
        PdfDocument::from_bytes(FIXTURE.to_vec()).expect("fixture parses")
    }

    #[test]
    fn fixture_renders_two_pages_with_opaque_alpha() {
        let doc = document();
        assert_eq!(doc.page_count(), 2);
        let pages = doc.render_all(1.0).unwrap();
        assert_eq!(pages.len(), 2);
        for page in &pages {
            assert_eq!((page.width, page.height), (612, 842));
            assert_eq!(page.rgba.len(), page.width * page.height * 4);
            assert!(
                page.rgba.as_chunks::<4>().0.iter().all(|px| px[3] == 255),
                "alpha must be forced to 255"
            );
        }
    }

    #[test]
    fn ink_coverage_distinguishes_blank_from_inked() {
        let doc = document();
        let pages = doc.render_all(1.0).unwrap();
        let half_page = &pages[0];
        let blank = &pages[1];
        assert!(
            (half_page.ink_coverage - 0.5).abs() < 0.02,
            "left-half fill should be ~50%, got {}",
            half_page.ink_coverage
        );
        assert!(
            blank.ink_coverage < 0.001,
            "blank page should be ~0%, got {}",
            blank.ink_coverage
        );
    }

    #[test]
    fn scale_derives_from_target_width_over_page_width() {
        assert_eq!(PdfDocument::scale_for_width(612.0, 1224), 2.0);
        assert_eq!(PdfDocument::scale_for_width(612.0, 306), 0.5);
        assert_eq!(PdfDocument::scale_for_width(0.0, 100), 100.0);
    }

    #[test]
    fn render_page_to_width_fits_the_target() {
        let doc = document();
        let page = doc.render_page_to_width(0, 306).unwrap();
        assert_eq!(page.width, 306);
        assert_eq!(page.height, 421);
    }

    #[test]
    fn out_of_range_page_errors() {
        let doc = document();
        let err = doc.render_page(5, 1.0).unwrap_err();
        assert!(err.to_string().contains("page 6 does not exist"));
    }

    #[test]
    fn invalid_bytes_error() {
        let err = PdfDocument::from_bytes(b"not a pdf".to_vec())
            .err()
            .expect("invalid bytes must fail");
        assert!(!err.to_string().is_empty());
    }
}
