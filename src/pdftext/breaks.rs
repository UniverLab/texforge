//! Which section opens each page, matched from the extracted page text.

use super::extract::normalize_pdf_text;
use super::PdfPageBreak;

/// Report which section opens each page.
///
/// `sections` is `(number, title)` in document order (e.g. from the outline).
/// A section title matches a page only when it appears as its own line in
/// that page's extracted text — either the bare title, or the title with a
/// leading numbering prefix (`"1 "`, `"2.4. "`, `"2.4.1 "`), since a numbered
/// `\section` renders that way (see `section_title_in_page`). A mention
/// inside a sentence does not count, and neither does a table-of-contents
/// entry, whose line carries dot leaders and a page number after the title
/// instead of ending there. For each section, in order, we search forward
/// from the page where the previous *matched* section was found for the
/// first page whose text contains this section's title as a line.
///
/// A page is opened by the *first* matched heading that appears on it, in
/// document order — never the last. Whatever precedes that heading's line
/// (a diagram's rendered vector text spilling over the top of the page from
/// a floated figure, a caption, blank lines) is not itself a section, so it
/// cannot be what the page opens with instead: the heading is still the
/// first piece of section content the page reaches, however much non-section
/// filler sits above it. A page with no matched heading of its own carries
/// the previous page's section forward unchanged; pages before the first
/// match anywhere stay unattributed.
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
    for i in 0..normalized_pages.len() {
        // `matches` is grouped by page in document order, so the entry at
        // `match_idx` — if it belongs to this page at all — is the first
        // (not last) section matched on it.
        if match_idx < matches.len() && matches[match_idx].0 == i {
            let (_, num, title) = &matches[match_idx];
            current = Some((num.clone(), title.clone()));
            while match_idx < matches.len() && matches[match_idx].0 == i {
                match_idx += 1;
            }
        }
        out.push(PdfPageBreak {
            page: i + 1,
            section: current.as_ref().map(|(n, _)| n.clone()),
            title: current.as_ref().map(|(_, t)| t.clone()),
        });
    }
    out
}

/// Strip a leading numbering prefix from `line`: one or more digit groups
/// separated by dots (`2`, `2.4`, `2.4.1`), optionally followed by a
/// trailing dot, then at least one whitespace character — the shape a
/// numbered `\section`'s auto-numbering renders as (`"2.4. Estilos..."`,
/// `"2.4.1 Something"`). Returns the remainder with leading whitespace
/// trimmed, or `None` if `line` does not start with such a prefix.
///
/// A character walk, not a regex: the prefix grammar is small and fixed.
fn strip_numbering_prefix(line: &str) -> Option<&str> {
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;

    if i >= chars.len() || !chars[i].is_ascii_digit() {
        return None;
    }
    while i < chars.len() && chars[i].is_ascii_digit() {
        i += 1;
    }
    while i < chars.len() && chars[i] == '.' {
        if i + 1 < chars.len() && chars[i + 1].is_ascii_digit() {
            i += 1;
            while i < chars.len() && chars[i].is_ascii_digit() {
                i += 1;
            }
        } else {
            // Trailing dot with nothing numeric after it: consume it and
            // stop — the next character must be whitespace.
            i += 1;
            break;
        }
    }
    if i >= chars.len() || !chars[i].is_whitespace() {
        return None;
    }
    let byte_offset: usize = chars[..i].iter().map(|c| c.len_utf8()).sum();
    Some(line[byte_offset..].trim_start())
}

/// True when a trimmed page line is the heading line for `title`: either the
/// bare title, or the title with a leading numbering prefix stripped (see
/// [`strip_numbering_prefix`]) — the two forms a `\section` and a numbered
/// `\section` render as in extracted PDF text. Equality is required after
/// stripping, so a table-of-contents line — title followed by dot leaders
/// and a page number — does not match, and neither does a mention inside a
/// sentence: neither form leaves the line ending exactly at the title.
fn line_is_section_heading(line: &str, needle: &str) -> bool {
    if line == needle {
        return true;
    }
    match strip_numbering_prefix(line) {
        Some(rest) => rest == needle,
        None => false,
    }
}

/// True when `title` appears as its own line (trimmed, normalized) anywhere
/// in `page_text`. A mention inside a sentence does not count.
fn section_title_in_page(page_text: &str, title: &str) -> bool {
    let needle = normalize_pdf_text(title);
    let needle = needle.trim();
    if needle.is_empty() {
        return false;
    }
    page_text
        .lines()
        .any(|line| line_is_section_heading(line.trim(), needle))
}

#[cfg(test)]
mod tests {
    use super::super::extract::extract_text_by_pages_from_bytes;
    use super::super::fixtures::PAGES_PDF;
    use super::super::format_page_breaks;
    use super::*;

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
    fn page_breaks_ignore_a_title_mentioned_inside_prose_te9() {
        // TE9: the reporter's two-page document. "UniverLab.org" is a
        // section title, but it also occurs verbatim inside a sentence in
        // the "Perfil Profesional" prose on page 1. A substring match would
        // false-positive there (as the buggy code did); a whole-line match
        // must not.
        let pages = vec![
            "Jane Doe\ncontacto@example.com\nFundador de UniverLab.org y contribuidor activo en open source\nPerfil Profesional\nExperiencia Laboral\nSix positions of professional experience follow.".to_string(),
            "UniverLab.org\nFormación Académica\nHabilidades Técnicas\nRust, Python, and more.".to_string(),
        ];
        let sections = vec![
            ("1".into(), "Perfil Profesional".into()),
            ("2".into(), "Experiencia Laboral".into()),
            ("3".into(), "UniverLab.org".into()),
            ("4".into(), "Formación Académica".into()),
            ("5".into(), "Habilidades Técnicas".into()),
        ];
        let breaks = page_breaks(&pages, &sections);
        let formatted = format_page_breaks(&breaks);
        assert_eq!(
            formatted,
            "page=1 section=1 title=Perfil Profesional\npage=2 section=3 title=UniverLab.org"
        );
    }

    #[test]
    fn page_breaks_title_only_in_prose_never_matches() {
        let pages = vec![
            "This report inline-mentions Special Report but never as a heading.".to_string(),
            "More filler text on the second page, still no heading line.".to_string(),
        ];
        let sections = vec![("1".into(), "Special Report".into())];
        let breaks = page_breaks(&pages, &sections);
        let formatted = format_page_breaks(&breaks);
        assert_eq!(formatted, "page=1 section= title=\npage=2 section= title=");
    }

    #[test]
    fn page_breaks_a_single_match_wins_even_when_preceded_by_other_text() {
        // A page's only matched heading is what opens it, regardless of what
        // precedes that heading's line — text before it (spillover, a
        // caption, a diagram) is not itself a section, so it cannot be what
        // the page opens with instead. See `page_breaks_defect_2_...` below
        // for the real-document shape of this: a heading preceded by a
        // diagram's garbled rendered text, not real prose spillover.
        let pages = vec![
            "Alpha\nBody text under Alpha continues here.".to_string(),
            "Trailing body text from Alpha spills onto this page.\nBeta\nMore Beta content."
                .to_string(),
        ];
        let sections = vec![("1".into(), "Alpha".into()), ("2".into(), "Beta".into())];
        let breaks = page_breaks(&pages, &sections);
        let formatted = format_page_breaks(&breaks);
        assert_eq!(
            formatted,
            "page=1 section=1 title=Alpha\npage=2 section=2 title=Beta"
        );
    }

    #[test]
    fn page_breaks_several_headings_on_one_page_first_wins() {
        let pages = vec![
            "Cover page with no headings at all.".to_string(),
            "Gamma\nDelta\nBody text under Delta.".to_string(),
            "Body continues, no new heading here.".to_string(),
        ];
        let sections = vec![("1".into(), "Gamma".into()), ("2".into(), "Delta".into())];
        let breaks = page_breaks(&pages, &sections);
        let formatted = format_page_breaks(&breaks);
        assert_eq!(
            formatted,
            "page=1 section= title=\npage=2 section=1 title=Gamma\npage=3 section=1 title=Gamma"
        );
    }

    #[test]
    fn page_breaks_several_headings_on_a_page_reports_the_first_not_the_last() {
        // Defect 1: a page with multiple matched headings must report the
        // *first* one, not the last — and this must hold even once a
        // section has already opened on an earlier page (`current` is
        // already `Some`), which is the actual shape of the bug: a
        // first-page-with-any-match short-circuit made this look fixed
        // while the real failure (pages 4-6 of examples/texforge-capabilites,
        // each carrying a section already open from before) went uncaught.
        let pages = vec![
            "Alpha\nBody under Alpha.".to_string(),
            "Leftover Alpha text spills here.\nBeta\nGamma\nMore body.".to_string(),
        ];
        let sections = vec![
            ("1".into(), "Alpha".into()),
            ("2".into(), "Beta".into()),
            ("3".into(), "Gamma".into()),
        ];
        let breaks = page_breaks(&pages, &sections);
        let formatted = format_page_breaks(&breaks);
        assert_eq!(
            formatted,
            "page=1 section=1 title=Alpha\npage=2 section=2 title=Beta"
        );
    }

    #[test]
    fn page_breaks_pages_before_first_heading_are_unattributed() {
        let pages = vec![
            "Cover page with no headings at all.".to_string(),
            "Epsilon\nBody text under Epsilon.".to_string(),
        ];
        let sections = vec![("1".into(), "Epsilon".into())];
        let breaks = page_breaks(&pages, &sections);
        let formatted = format_page_breaks(&breaks);
        assert_eq!(
            formatted,
            "page=1 section= title=\npage=2 section=1 title=Epsilon"
        );
    }

    #[test]
    fn line_is_section_heading_matches_dotted_numbering() {
        // The real heading line from examples/texforge-capabilites.
        assert!(line_is_section_heading(
            "2.4. Estilos de Diagrama (style)",
            "Estilos de Diagrama (style)"
        ));
    }

    #[test]
    fn line_is_section_heading_matches_single_level_numbering() {
        assert!(line_is_section_heading(
            "2. Diagramas Embebidos",
            "Diagramas Embebidos"
        ));
    }

    #[test]
    fn line_is_section_heading_matches_three_level_numbering() {
        assert!(line_is_section_heading("2.4.1 Something", "Something"));
    }

    #[test]
    fn line_is_section_heading_rejects_table_of_contents_line_te_d() {
        // TF-D: the table-of-contents entry from examples/texforge-capabilites
        // carries the same numbering prefix and title as the real heading,
        // but with dot leaders and a page number trailing it. That must not
        // match — a false match here is the same class of lie TE9 removed.
        assert!(!line_is_section_heading(
            "2.4. Estilos de Diagrama (style) . . . . . . . . . . . . . . . . . 3",
            "Estilos de Diagrama (style)"
        ));
    }

    #[test]
    fn line_is_section_heading_matches_unnumbered_heading() {
        assert!(line_is_section_heading(
            "Estilos de Diagrama (style)",
            "Estilos de Diagrama (style)"
        ));
    }

    #[test]
    fn page_breaks_numbered_heading_matches_and_toc_does_not_te_d() {
        // TF-D: reproduces the bug with the real lines from
        // examples/texforge-capabilites — a table of contents listing the
        // heading (with dot leaders and a page number) must not be mistaken
        // for the heading itself, which appears later as its own line.
        let pages = vec![
            "Table of Contents\n2.4. Estilos de Diagrama (style) . . . . . . . . . . . . . . . . . 3"
                .to_string(),
            "Some other page with no heading.".to_string(),
            "2.4. Estilos de Diagrama (style)\nBody text about diagram styles follows."
                .to_string(),
        ];
        let sections = vec![("2.4".into(), "Estilos de Diagrama (style)".into())];
        let breaks = page_breaks(&pages, &sections);
        let formatted = format_page_breaks(&breaks);
        assert_eq!(
            formatted,
            "page=1 section= title=\npage=2 section= title=\npage=3 section=2.4 title=Estilos de Diagrama (style)"
        );
    }

    #[test]
    fn line_is_section_heading_rejects_leaderless_table_of_contents_line_tf_f() {
        // TF-F: examples/texforge-capabilites' table of contents has a
        // *second* shape for top-level entries — no dot leaders at all,
        // just the title then a bare page number. `d9af352` only ever
        // tested the dot-leader shape; this must be rejected too.
        assert!(!line_is_section_heading("3. Matemáticas 4", "Matemáticas"));
    }

    #[test]
    fn line_is_section_heading_matches_title_ending_in_a_number() {
        // A heading can legitimately end in a digit (`Capítulo 2`); the
        // leaderless-TOC rejection must key on what follows the title, not
        // merely on the line ending in a number.
        assert!(line_is_section_heading("Capítulo 2", "Capítulo 2"));
        assert!(line_is_section_heading("5. Capítulo 2", "Capítulo 2"));
    }

    #[test]
    fn page_breaks_leaderless_toc_entry_never_matches_tf_f() {
        // TF-F: reproduces the second bug shape with the real lines from
        // examples/texforge-capabilites — a leaderless top-level TOC entry
        // must not be mistaken for the heading itself.
        let pages = vec![
            "Table of Contents\n3. Matemáticas 4".to_string(),
            "Some other page with no heading.".to_string(),
            "3. Matemáticas\nBody text about equations follows.".to_string(),
        ];
        let sections = vec![("3".into(), "Matemáticas".into())];
        let breaks = page_breaks(&pages, &sections);
        let formatted = format_page_breaks(&breaks);
        assert_eq!(
            formatted,
            "page=1 section= title=\npage=2 section= title=\npage=3 section=3 title=Matemáticas"
        );
    }

    #[test]
    fn page_breaks_full_example_structure_tf_g() {
        // TF-G: a seven-page fixture reproducing examples/texforge-capabilites'
        // actual structure — TOC page mixing both TOC shapes (leaderless
        // top-level entries, dot-leader subsection entries), then the real
        // per-page content including the garbled letter-spaced text that
        // `pdf-extract` produces for the document's embedded diagrams. That
        // filler is what a prior attempt's fixture omitted: without it every
        // heading happens to be the first line of its page, so a page's only
        // match always "opens" the page and both defects stay hidden. Pages
        // 4-6 additionally carry more than one matched heading, so this
        // fixture alone would have caught both defect 1 (last match wins,
        // should be first) and defect 2 (a single non-opening match was
        // dropped in favor of carrying the previous section forward).
        let pages = vec![
            "1. Introducción 2\n\
             1.1. Antecedentes . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . 2\n\
             2. Diagramas Embebidos 2\n\
             2.4. Estilos de Diagrama (style) . . . . . . . . . . . . . . . . . . . . . . . . . . 3\n\
             3. Matemáticas 4\n\
             3.1. Ecuaciones en línea . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . 4\n\
             3.3. Matrices . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . 4\n\
             4. Citas Bibliográficas 5\n\
             5. Listados de Código 5\n\
             5.2. LaTeX . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . . 6\n\
             6. Tablas 6\n\
             7. Comandos de Texforge 7"
                .to_string(),
            "1. Introducción\nTexforge simplifica el flujo de trabajo académico.\n\
             1.1. Antecedentes\nLa edición de documentos académicos en LATEX requiere ...\n\
             2. Diagramas Embebidos\nTexforge renderiza diagramas directamente."
                .to_string(),
            // Real garbled text from the Mermaid diagram (Figura 1) spills
            // onto the top of the page before the 2.4 heading — this is the
            // exact shape of defect 2.
            "l o o p [ D i a g r a m a s   e m b e b i d o s ]\n\
             Figura 1: Secuencia de compilación de Texforge\n\
             2.4. Estilos de Diagrama (style)\n\
             Los tres entornos aceptan también un atributo style."
                .to_string(),
            // Real garbled preset-diagram text before three headings — the
            // shape of defect 1: must report the first (3), not the last
            // (3.3).
            "l o p [ D i D a [ g r m s g   D e b i D\n\
             El preset editorial sirve para documentos que se leen en pantalla.\n\
             3. Matemáticas\n\
             3.1. Ecuaciones en línea\n\
             La fórmula cuadrática se expresa como sigue.\n\
             3.3. Matrices\n\
             A ="
                .to_string(),
            "C ó d i g o   L a T e X\n\
             Figura 3: Pipeline de renderizado\n\
             4. Citas Bibliográficas\n\
             Texforge gestiona automáticamente las referencias.\n\
             5. Listados de Código\n\
             5.1. Python"
                .to_string(),
            "5.2. LaTeX\n\
             Listing 2: Estructura básica de Texforge\n\
             6. Tablas\n\
             Comparación de motores de renderizado."
                .to_string(),
            "7. Comandos de Texforge\n\
             Cuadro 2: Resumen de comandos principales.\n\
             8. Conclusión\n\
             Texforge demuestra ser una herramienta completa."
                .to_string(),
        ];
        let sections = vec![
            ("1".into(), "Introducción".into()),
            ("1.1".into(), "Antecedentes".into()),
            ("2".into(), "Diagramas Embebidos".into()),
            ("2.4".into(), "Estilos de Diagrama (style)".into()),
            ("3".into(), "Matemáticas".into()),
            ("3.1".into(), "Ecuaciones en línea".into()),
            ("3.3".into(), "Matrices".into()),
            ("4".into(), "Citas Bibliográficas".into()),
            ("5".into(), "Listados de Código".into()),
            ("5.2".into(), "LaTeX".into()),
            ("6".into(), "Tablas".into()),
            ("7".into(), "Comandos de Texforge".into()),
        ];
        let breaks = page_breaks(&pages, &sections);
        let formatted = format_page_breaks(&breaks);
        assert_eq!(
            formatted,
            "page=1 section= title=\n\
             page=2 section=1 title=Introducción\n\
             page=3 section=2.4 title=Estilos de Diagrama (style)\n\
             page=4 section=3 title=Matemáticas\n\
             page=5 section=4 title=Citas Bibliográficas\n\
             page=6 section=5.2 title=LaTeX\n\
             page=7 section=7 title=Comandos de Texforge"
        );
    }

    #[test]
    fn page_breaks_a_later_match_on_a_page_does_not_override_the_first() {
        // A page can carry more than one matched heading; the page is still
        // reported as opened by the first one, and that is what carries
        // forward into a following page with no heading of its own — never
        // the later heading.
        let pages = vec![
            "Alpha\nBody under Alpha.".to_string(),
            "Trailing Alpha content spills onto this page.\nBeta\nMore text.\nGamma\nBody."
                .to_string(),
            "No new heading here, still reads as later content.".to_string(),
        ];
        let sections = vec![
            ("1".into(), "Alpha".into()),
            ("2".into(), "Beta".into()),
            ("3".into(), "Gamma".into()),
        ];
        let breaks = page_breaks(&pages, &sections);
        let formatted = format_page_breaks(&breaks);
        assert_eq!(
            formatted,
            "page=1 section=1 title=Alpha\npage=2 section=2 title=Beta\npage=3 section=2 title=Beta"
        );
    }

    #[test]
    fn page_breaks_defect_2_regression_heading_preceded_by_diagram_junk() {
        // Regression for defect 2: `2.4. Estilos de Diagrama (style)` is a
        // real heading line from examples/texforge-capabilites (page 3 of
        // the compiled PDF), but it is preceded by the garbled letter-spaced
        // text `pdf-extract` produces for the Mermaid diagram rendered at
        // the top of that page ("l o o p [ D i a g r a m a s ... ]", the
        // literal extracted text of Figura 1). The buggy code required a
        // page's first matched heading to be the very first non-blank line
        // on the page or fall back to carrying the previous section forward;
        // since nothing else on the page matched, it kept reporting the
        // prior section ("1 Introducción") instead of "2.4". The fix: a
        // page's first matched heading opens it regardless of what
        // (non-section) text precedes it.
        let pages = vec![
            "1. Introducción\nTexforge simplifica el flujo de trabajo académico.".to_string(),
            "l o o p [ D i a g r a m a s   e m b e b i d o s ]\n\
             Figura 1: Secuencia de compilación de Texforge\n\
             2.4. Estilos de Diagrama (style)\n\
             Los tres entornos aceptan también un atributo style."
                .to_string(),
        ];
        let sections = vec![
            ("1".into(), "Introducción".into()),
            ("2.4".into(), "Estilos de Diagrama (style)".into()),
        ];
        let breaks = page_breaks(&pages, &sections);
        let formatted = format_page_breaks(&breaks);
        assert_eq!(
            formatted,
            "page=1 section=1 title=Introducción\npage=2 section=2.4 title=Estilos de Diagrama (style)"
        );
    }
}
