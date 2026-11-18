//! Overlap between consecutive per-line frame-background rules.
//!
//! Every source line is its own `\noindent` paragraph (so a page may break
//! between any two lines) and each line paints its own zero-metric tint
//! rectangle (see `tint_rule` in [`crate::highlight`] emission). If those
//! rectangles abutted exactly at the line boundary, rounding left a hairline
//! of paper showing between them on dark backgrounds. Extending the mid-line
//! height past the boundary by [`TINT_OVERLAP`] makes consecutive rectangles
//! overlap instead, so no gap can show through. The last line carries the
//! same overlap above (its bottom padding is unchanged) so its top seam
//! overlaps too; the first line's bottom seam is covered by the second
//! line's overlap. Page breaking is unchanged: the rules stay smashed and
//! per-line, and the single whole-block box that would break
//! `frame-page-split` is still avoided.

/// How far each mid-line background rule extends past the line boundary, so
/// it overlaps the next line's rule instead of abutting it.
pub(crate) const TINT_OVERLAP: &str = "0.8pt";

/// The mid-line rule height: one `\baselineskip` plus the overlap.
pub(crate) fn mid_line_height() -> String {
    format!("\\dimexpr\\baselineskip+{TINT_OVERLAP}\\relax")
}

/// The last-line rule height: the 3.4pt end padding below plus the overlap
/// above, so its top seam overlaps the previous line like a mid line.
pub(crate) fn last_line_height() -> String {
    format!("\\dimexpr\\baselineskip+3.4pt+{TINT_OVERLAP}\\relax")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mid_line_height_carries_the_overlap() {
        assert_eq!(mid_line_height(), "\\dimexpr\\baselineskip+0.8pt\\relax");
        assert_eq!(
            last_line_height(),
            "\\dimexpr\\baselineskip+3.4pt+0.8pt\\relax"
        );
    }

    #[test]
    fn first_and_last_lines_keep_their_exact_padding() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("main.tex"),
            "\\documentclass{article}\n\\begin{document}\n\
             \\begin{code}[lang=python, numbers=true]\nline one\nline two\nline three\n\\end{code}\n\\end{document}\n",
        )
        .unwrap();
        crate::highlight::run(
            dir.path(),
            "main.tex",
            crate::highlight::Settings::default(),
        )
        .unwrap();
        let actual = std::fs::read_to_string(dir.path().join("main.tex")).unwrap();
        // First line: tint, two sides, separator with 3.4pt. Last line: the
        // 3.4pt end padding below plus the overlap above.
        assert_eq!(
            actual
                .matches("\\dimexpr\\baselineskip+3.4pt\\relax")
                .count(),
            4,
            "first line (tint, sides, separator) keeps 3.4pt: {actual}"
        );
        assert_eq!(
            actual
                .matches("\\dimexpr\\baselineskip+3.4pt+0.8pt\\relax")
                .count(),
            4,
            "last line (tint, sides, separator) keeps 3.4pt plus the overlap: {actual}"
        );
        assert!(
            !actual.contains("\\dimexpr\\baselineskip+6.8pt\\relax"),
            "a three-line block has no single-line geometry: {actual}"
        );
    }

    #[test]
    fn tint_sides_and_separator_share_the_mid_line_geometry() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("main.tex"),
            "\\documentclass{article}\n\\begin{document}\n\
             \\begin{code}[lang=python, numbers=true]\nline one\nline two\nline three\n\\end{code}\n\\end{document}\n",
        )
        .unwrap();
        crate::highlight::run(
            dir.path(),
            "main.tex",
            crate::highlight::Settings::default(),
        )
        .unwrap();
        let actual = std::fs::read_to_string(dir.path().join("main.tex")).unwrap();
        assert_eq!(
            actual
                .matches("\\dimexpr\\baselineskip+0.8pt\\relax")
                .count(),
            4,
            "the one mid line paints tint, two sides and separator with the overlap: {actual}"
        );
        assert!(
            !actual.contains("{\\baselineskip}}"),
            "no tint/side/separator height may abut at the bare baselineskip: {actual}"
        );
    }

    /// Run the highlight pass on a nine-line bash block and return the
    /// directory (kept alive by the caller) with `rewrite` applied to the
    /// rewritten `main.tex`. Identity replays the fixed emission; the
    /// reproduction below strips the overlap to replay the pre-fix one.
    fn prepare_block(options: &str, rewrite: impl FnOnce(&str) -> String) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let mut body = String::new();
        for i in 1..=9 {
            body.push_str(&format!("echo line{i}\n"));
        }
        let main = format!(
            "\\documentclass{{article}}\n\\begin{{document}}\n\
             \\begin{{code}}[{options}]\n{body}\\end{{code}}\n\\end{{document}}\n"
        );
        std::fs::write(dir.path().join("main.tex"), &main).unwrap();
        crate::highlight::run(
            dir.path(),
            "main.tex",
            crate::highlight::Settings::default(),
        )
        .unwrap();
        let path = dir.path().join("main.tex");
        let actual = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, rewrite(&actual)).unwrap();
        dir
    }

    /// Compile a prepared block and rasterize its page at 200 dpi. The PDF
    /// is fully parsed into memory, so the returned pixels outlive `dir`.
    fn render_prepared(name: &str, dir: &tempfile::TempDir) -> crate::raster::RenderedPage {
        crate::compiler::compile(
            dir.path(),
            "main.tex",
            false,
            None,
            &crate::highlight::LineMap::default(),
        )
        .unwrap_or_else(|e| panic!("{name}: tectonic failed: {e}"));
        let pdf = dir.path().join("main.pdf");
        assert!(pdf.exists(), "{name}: no pdf written");
        let doc = crate::raster::PdfDocument::open(&pdf).unwrap();
        assert_eq!(doc.page_count(), 1, "{name}: block must fit on one page");
        doc.render_page(0, 200.0 / 72.0).unwrap()
    }

    /// Compile a nine-line bash block with `options` and rasterize its page
    /// at 200 dpi. The tempdir is dropped on return; the pixels are owned.
    fn render_block(name: &str, options: &str) -> crate::raster::RenderedPage {
        let dir = prepare_block(options, |tex| tex.to_string());
        render_prepared(name, &dir)
    }

    /// Brightness of one pixel column through empty tint (70% of the page
    /// width: the short lines never reach it, so only background shows).
    fn tint_column(page: &crate::raster::RenderedPage) -> Vec<f32> {
        let x = page.width * 7 / 10;
        (0..page.height)
            .map(|y| {
                let i = (y * page.width + x) * 4;
                (f32::from(page.rgba[i])
                    + f32::from(page.rgba[i + 1])
                    + f32::from(page.rgba[i + 2]))
                    / 3.0
            })
            .collect()
    }

    /// Longest below-paper stretch of the column: the block's tinted band.
    fn tint_run(col: &[f32], paper: f32) -> (usize, usize) {
        let mut best = (0usize, 0usize);
        let mut start: Option<usize> = None;
        for (y, &v) in col.iter().enumerate() {
            if v < paper {
                if start.is_none() {
                    start = Some(y);
                }
            } else if let Some(s) = start.take() {
                if y - s > best.1 - best.0 {
                    best = (s, y);
                }
            }
        }
        if let Some(s) = start {
            if col.len() - s > best.1 - best.0 {
                best = (s, col.len());
            }
        }
        best
    }

    /// Rows of the tinted band far from its median brightness. Antialiased
    /// rule edges stay within tolerance of the band median, while a hairline
    /// seam rasterizes a full row far from it.
    fn off_tint_rows(page: &crate::raster::RenderedPage, dark: bool) -> (f32, Vec<usize>) {
        let col = tint_column(page);
        let paper = if dark { 128.0 } else { 250.0 };
        // Text glyphs are darker than the tint; column x sits past the line
        // ends so only tint and paper show.
        let (top, bottom) = tint_run(&col, paper);
        assert!(bottom - top > 50, "tint band too short");
        let band = &col[top + 5..bottom - 5];
        let mut sorted = band.to_vec();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let median = sorted[sorted.len() / 2];
        let off: Vec<usize> = band
            .iter()
            .enumerate()
            .filter(|(_, v)| (**v - median).abs() > 25.0)
            .map(|(y, _)| top + 5 + y)
            .collect();
        (median, off)
    }

    /// Reproduction: the pre-fix emission — per-line tint rules abutting at
    /// the bare `\baselineskip` — rasterizes one lighter row per seam on a
    /// dark background (8 seams in a nine-line block: one per line boundary).
    /// Replays it by stripping the overlap from the emitted LaTeX, then
    /// renders at 200 dpi and samples a pixel column through the background.
    /// Skips (never fails) without Tectonic. On the light style the paper
    /// seam is invisible by construction, so the light case only pins the
    /// abutting structure, not the pixels.
    #[test]
    fn abutting_rules_show_a_lighter_row_per_seam() {
        // Spawns tectonic: the child inherits the process env, so hold
        // ENV_LOCK or a concurrent HOME swap gives it a cold bundle cache.
        let _env = crate::test_sync::ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if crate::compiler::locate_tectonic().is_none() {
            eprintln!("skipping: tectonic not available in environment");
            return;
        }
        let _tectonic = crate::test_support::tectonic_lock();
        for (name, options, dark) in [
            ("dark", "lang=bash, style=dark, numbers=true", true),
            (
                "dark-mono",
                "lang=bash, style=dark-mono, numbers=true",
                true,
            ),
            ("light", "lang=bash, numbers=true", false),
        ] {
            let dir = prepare_block(options, |tex| {
                tex.replace("\\dimexpr\\baselineskip+0.8pt\\relax", "\\baselineskip")
                    .replace(
                        "\\dimexpr\\baselineskip+3.4pt+0.8pt\\relax",
                        "\\dimexpr\\baselineskip+3.4pt\\relax",
                    )
            });
            let actual = std::fs::read_to_string(dir.path().join("main.tex")).unwrap();
            assert!(
                actual.contains("{\\baselineskip}}"),
                "{name}: stripped emission must abut at the bare baselineskip"
            );
            let page = render_prepared(name, &dir);
            if !dark {
                continue;
            }
            let (median, off) = off_tint_rows(&page, dark);
            assert!(
                off.len() >= 6,
                "{name}: expected a lighter row per seam (median {median:.0}), got {}: {off:?}",
                off.len()
            );
        }
    }

    /// Regression: the tint between two line baselines must be continuous —
    /// no lighter hairline row on dark backgrounds, and the overlap must not
    /// darken light ones. Skips (never fails) without Tectonic.
    ///
    /// Observed visually (`PyMuPDF` at 200 dpi, scratch project from
    /// `texforge new`): a dark bash listing with numbers, a dark Python
    /// listing without numbers, a 59-line dark listing across a page break,
    /// and the same three in `dark-mono` — each renders as one continuous
    /// background with no line between code lines; the light control shows
    /// no darker row either.
    #[test]
    fn dark_listings_have_no_hairline_between_lines() {
        // Spawns tectonic: the child inherits the process env, so hold
        // ENV_LOCK or a concurrent HOME swap gives it a cold bundle cache.
        let _env = crate::test_sync::ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if crate::compiler::locate_tectonic().is_none() {
            eprintln!("skipping: tectonic not available in environment");
            return;
        }
        let _tectonic = crate::test_support::tectonic_lock();
        // Abutting tint rules rasterize as one lighter row per seam; with the
        // overlap there are no interior seams at all. Count rows far from the
        // band median: the old emission shows one per line boundary (8 in a
        // nine-line block), the fixed one none.
        for (name, options, dark) in [
            ("dark", "lang=bash, style=dark, numbers=true", true),
            (
                "dark-mono",
                "lang=bash, style=dark-mono, numbers=true",
                true,
            ),
            ("light", "lang=bash, numbers=true", false),
        ] {
            let page = render_block(name, options);
            let (median, off) = off_tint_rows(&page, dark);
            assert!(
                off.len() <= 2,
                "{name}: {} off-tint rows {:?} (median {median:.0})",
                off.len(),
                &off[..off.len().min(10)]
            );
        }
    }
}
