//! Character-level escaping and per-line paragraph emission for code blocks.
//!
//! The output is plain LaTeX kernel + `color.sty` only (route 3): every source
//! character is escaped so the author's text can never be interpreted, every
//! source line becomes its own paragraph so TeX may break the page between any
//! two lines, and spaces become non-breaking so lines never wrap — which is
//! what makes the Rust-side overfull check (rather than TeX's log) the
//! authoritative warning.

use std::collections::BTreeSet;

use crate::highlight::engine::{Rgb, Span};
use crate::highlight::Warning;

/// A line longer than this many display columns (tab-expanded source plus the
/// line-number gutter) probably pokes past the default `\textwidth` at
/// `\small` cmtt (~4.7 pt per character × 90 ≈ 423 pt < 469 pt). Heuristic,
/// documented in `docs/listings.md`; TeX still reports its own overfull
/// hboxes, so this is a convenience, not a guarantee.
pub(crate) const OVERFULL_CHAR_LIMIT: usize = 90;

/// Everything `render_block` needs to place one block in the document: which
/// build-copy file it came from (warnings point there — the build copy is what
/// Tectonic reports errors in), the `\begin` line, the line the body starts
/// on, the `\end` line, and whether the line-number gutter is on.
pub(crate) struct EmitOpts<'a> {
    pub(crate) file: &'a str,
    pub(crate) first_line: usize,
    /// 1-based line (in the build copy) of the block's first body line.
    /// Options spanning several lines shift the body off the `\begin` line;
    /// this is the body's own line.
    pub(crate) body_line: usize,
    pub(crate) end_line: usize,
    pub(crate) numbers: bool,
}

/// Map one source character to its LaTeX expansion, or `None` when it passes
/// through untouched (`XeTeX` reads UTF-8 source, so every non-ASCII character
/// prints as itself).
///
/// `|` deliberately passes through: it prints fine in `cmtt`.
pub(crate) fn escape_char(c: char) -> Option<&'static str> {
    match c {
        '\\' => Some("\\textbackslash{}"),
        '{' => Some("\\{"),
        '}' => Some("\\}"),
        '$' => Some("\\$"),
        '&' => Some("\\&"),
        '#' => Some("\\#"),
        '_' => Some("\\_"),
        '%' => Some("\\%"),
        '~' => Some("\\textasciitilde{}"),
        '^' => Some("\\textasciicircum{}"),
        '<' => Some("\\(<\\)"),
        '>' => Some("\\(>\\)"),
        ' ' => Some("~"),
        '\t' => Some("~~~~"),
        '\r' => Some(""),
        _ => None,
    }
}

/// Append `text` to `out` with every character escaped.
pub(crate) fn escape_into(out: &mut String, text: &str) {
    for c in text.chars() {
        match escape_char(c) {
            Some(escaped) => out.push_str(escaped),
            None => out.push(c),
        }
    }
}

/// Display width of one source line: characters after tab expansion, plus the
/// gutter the line is prefixed with when numbering is on.
fn display_width(line: &str, numbers: bool, gutter_width: usize) -> usize {
    // Tabs expand to 4 (emission turns each into four `~`, so the width must
    // agree with what TeX lays out).
    let chars: usize = line.chars().map(|c| if c == '\t' { 4 } else { 1 }).sum();
    if numbers {
        // `\hbox to <w>em` plus the 0.8 em gap before the code.
        chars + gutter_width + 1
    } else {
        chars
    }
}

/// The number box for one numbered line, right-aligned inside its
/// `{width}em` gutter: the stretchable `\hss` comes *first*, so the digit
/// hugs the 0.3pt separator rule instead of the frame's left edge (the
/// separator rule and the 0.8em gap are emitted separately by
/// [`render_block`]).
fn gutter(number: usize, width: usize) -> String {
    format!("\\textcolor{{tfxgutter}}{{\\hbox to {width}em{{\\hss {number}}}}}")
}

/// Width of the gutter: at least two columns, wider only when the block has
/// ten or more lines.
fn gutter_width(line_count: usize) -> usize {
    let digits = line_count.to_string().len();
    digits.max(2)
}

/// Vertical geometry of one line's frame overlay: `(raise, height)`.
///
/// The mid-line rule covers the line's depth (`-3pt`, safe for `\small`
/// descenders) up to `\baselineskip - 3pt`. The first line extends 3.4pt
/// above (3pt top padding + the 0.4pt top border) and the last 3.4pt below,
/// so the tint edge and the border coincide; a single-line block extends
/// both ways (6.8pt total). All overlays are zero-metric (`\tfxsmash` +
/// `\rlap`), so page breaking and line widths are exactly as without them.
fn geometry(first: bool, last: bool) -> (&'static str, String) {
    match (first, last) {
        (true, true) => ("-6.4pt", "\\dimexpr\\baselineskip+6.8pt\\relax".to_string()),
        (true, false) => ("-3pt", "\\dimexpr\\baselineskip+3.4pt\\relax".to_string()),
        (false, true) => ("-6.4pt", "\\dimexpr\\baselineskip+3.4pt\\relax".to_string()),
        (false, false) => ("-3pt", "\\baselineskip".to_string()),
    }
}

/// The background tint for one line, full `\linewidth` minus the borders.
fn tint_rule(first: bool, last: bool) -> String {
    let (raise, height) = geometry(first, last);
    format!(
        "\\tfxsmash{{\\rlap{{\\kern0.4pt\\textcolor{{tfxtint}}{{\\rule[{raise}]{{\\dimexpr\\linewidth-0.8pt\\relax}}{{{height}}}}}}}}}"
    )
}

/// The 0.4pt left and right hairlines in `tfxframe`.
fn side_rules(first: bool, last: bool) -> String {
    let (raise, height) = geometry(first, last);
    format!(
        "\\tfxsmash{{\\rlap{{\\textcolor{{tfxframe}}{{\\rule[{raise}]{{0.4pt}}{{{height}}}}}}}}}\
         \\tfxsmash{{\\rlap{{\\kern\\dimexpr\\linewidth-0.4pt\\relax\\textcolor{{tfxframe}}{{\\rule[{raise}]{{0.4pt}}{{{height}}}}}}}}}"
    )
}

/// The horizontal borders: top rule on the first line, bottom rule on the
/// last (a single-line block gets both). Mid lines carry none, so a block
/// split across pages stays open at the break.
fn end_rules(first: bool, last: bool) -> String {
    let mut out = String::new();
    if first {
        out.push_str(
            "\\tfxsmash{\\rlap{\\kern0.4pt\\textcolor{tfxframe}{\\rule[\\baselineskip]{\\dimexpr\\linewidth-0.8pt\\relax}{0.4pt}}}}",
        );
    }
    if last {
        out.push_str(
            "\\tfxsmash{\\rlap{\\kern0.4pt\\textcolor{tfxframe}{\\rule[-6.4pt]{\\dimexpr\\linewidth-0.8pt\\relax}{0.4pt}}}}",
        );
    }
    out
}

/// The 0.3pt gutter separator in `tfxframe` (numbered blocks only). Smashed
/// like the rest, but not `\rlap`ped: its 0.3pt width is real spacing
/// between the numbers and the code.
fn separator_rule(first: bool, last: bool) -> String {
    let (raise, height) = geometry(first, last);
    format!("\\tfxsmash{{\\textcolor{{tfxframe}}{{\\rule[{raise}]{{0.3pt}}{{{height}}}}}}}")
}

/// Widow/orphan protection: glue the first two and the last two lines of
/// the block together. Each source line is its own one-line paragraph, so
/// club/widow penalties do not apply — `\vadjust{\penalty10000}` in the
/// vlist between the paragraphs does.
fn break_penalty(line: usize, last: usize) -> &'static str {
    if last < 2 {
        return "";
    }
    if line == 1 || line == last - 1 {
        "\\vadjust{\\penalty10000}"
    } else {
        ""
    }
}

/// Render one line's overlay + gutter + code into a single `\noindent`
/// paragraph. Stays on one output file line: the build-copy line map
/// depends on it.
fn emit_line(body: &str, line: usize, last: usize, numbers: bool, width: usize) -> String {
    let first = line == 1;
    let is_last = line == last;
    let mut out = String::from("\\noindent ");
    out.push_str(&tint_rule(first, is_last));
    out.push_str(&side_rules(first, is_last));
    out.push_str(&end_rules(first, is_last));
    // 4pt inner padding between the frame and the code.
    out.push_str("\\kern4pt");
    if numbers {
        out.push_str(&gutter(line, width));
        out.push_str(&separator_rule(first, is_last));
        out.push_str("\\hspace{0.8em}");
    }
    out.push_str(body);
    out.push_str(break_penalty(line, last));
    out
}

/// Emit one line's runs, merging adjacent spans that share a colour and
/// leaving base-colour runs unwrapped (they are plain black text; wrapping
/// them would inflate the output for no visual change).
fn render_runs(spans: &[Span], used: &mut BTreeSet<Rgb>) -> String {
    let mut out = String::new();
    let mut current: Option<Option<Rgb>> = None;
    let mut buf = String::new();

    let mut flush = |current: &mut Option<Option<Rgb>>, buf: &mut String, out: &mut String| {
        match *current {
            Some(Some(rgb)) => {
                used.insert(rgb);
                out.push_str("\\textcolor{");
                out.push_str(&rgb.name());
                out.push_str("}{");
                out.push_str(buf);
                out.push('}');
            }
            Some(None) => out.push_str(buf),
            None => {}
        }
        buf.clear();
    };

    for span in spans {
        if current != Some(span.color) {
            flush(&mut current, &mut buf, &mut out);
            current = Some(span.color);
        }
        escape_into(&mut buf, &span.text);
    }
    flush(&mut current, &mut buf, &mut out);
    out
}

/// Render one whole block into the LaTeX that replaces it.
///
/// * `body` — the block's source, `\r` already stripped, no surrounding
///   `\begin`/`\end`.
/// * `spans` — one `Vec<Span>` per line of `body` from
///   [`crate::highlight::engine::highlight`], or `None` for a monochrome
///   (plain or unknown-language) block.
/// * `origins` — receives one pass-input line per output line, so the build
///   can map finished-copy lines back to source lines.
///
/// Structure: `\par\medskip`, then a group scoping `\tfxcodestyle` with one
/// framed `\noindent` paragraph per source line separated by blank lines (so
/// TeX may break the page between any two lines), then `\par` + `}`; the
/// caller appends the trailing `\medskip` (and `\noindent` for the next
/// paragraph) once it knows what follows the block.
pub(crate) fn render_block(
    body: &str,
    spans: Option<&[Vec<Span>]>,
    opts: &EmitOpts,
    used: &mut BTreeSet<Rgb>,
    warnings: &mut Vec<Warning>,
    origins: &mut Vec<usize>,
) -> String {
    let mut lines: Vec<&str> = body.split('\n').collect();
    // The block scanner strips the body's trailing newline before calling;
    // tolerate direct callers that did not (a lone trailing segment is not a
    // source line).
    if lines.len() > 1 && lines.last() == Some(&"") {
        lines.pop();
    }
    let total = lines.len();
    let width = gutter_width(total);
    let mut out_lines: Vec<String> = Vec::with_capacity(total * 2 + 6);
    out_lines.push("\\par\\medskip".to_string());
    origins.push(opts.first_line);
    out_lines.push("{".to_string());
    origins.push(opts.first_line);
    out_lines.push("\\tfxcodestyle".to_string());
    origins.push(opts.first_line);
    out_lines.push(String::new());
    origins.push(opts.first_line);

    for (i, line) in lines.iter().enumerate() {
        let rendered_body = if line.is_empty() {
            // A blank source line still occupies a real line of output; the
            // gutter hbox already gives a numbered one its height.
            if opts.numbers {
                String::new()
            } else {
                "\\mbox{}".to_string()
            }
        } else {
            match spans.and_then(|s| s.get(i)) {
                Some(line_spans) => render_runs(line_spans, used),
                None => {
                    let mut escaped = String::new();
                    escape_into(&mut escaped, line);
                    escaped
                }
            }
        };
        // The space after `\noindent` is skipped by TeX (it is the control
        // word's delimiter); without it the first code character would glue
        // onto `\noindent` and form an undefined control sequence.
        out_lines.push(emit_line(&rendered_body, i + 1, total, opts.numbers, width));
        origins.push(opts.body_line + i);
        if i + 1 < total {
            // Blank separator between two line paragraphs: Tectonic reports
            // an overfull paragraph on its end line, so attribute it to the
            // code line above, not below.
            out_lines.push(String::new());
            origins.push(opts.body_line + i);
        }

        let width_of_line = display_width(line, opts.numbers, width);
        if width_of_line > OVERFULL_CHAR_LIMIT {
            warnings.push(Warning {
                file: opts.file.to_string(),
                line: opts.body_line + i,
                message: format!(
                    "code line is {width_of_line} chars wide — it may exceed the text width; split it"
                ),
            });
        }
    }

    out_lines.push("\\par".to_string());
    origins.push(opts.body_line + total.saturating_sub(1));
    out_lines.push("}".to_string());
    origins.push(opts.end_line);
    out_lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(file: &str, body_line: usize, numbers: bool) -> EmitOpts<'_> {
        EmitOpts {
            file,
            first_line: body_line.saturating_sub(1),
            body_line,
            end_line: body_line + 100,
            numbers,
        }
    }

    fn render(
        body: &str,
        spans: Option<&[Vec<Span>]>,
        o: &EmitOpts,
    ) -> (String, BTreeSet<Rgb>, Vec<Warning>, Vec<usize>) {
        let mut used = BTreeSet::new();
        let mut warnings = Vec::new();
        let mut origins = Vec::new();
        let out = render_block(body, spans, o, &mut used, &mut warnings, &mut origins);
        (out, used, warnings, origins)
    }

    #[test]
    fn escape_table_covers_every_special_character() {
        assert_eq!(escape_char('\\'), Some("\\textbackslash{}"));
        assert_eq!(escape_char('{'), Some("\\{"));
        assert_eq!(escape_char('}'), Some("\\}"));
        assert_eq!(escape_char('$'), Some("\\$"));
        assert_eq!(escape_char('&'), Some("\\&"));
        assert_eq!(escape_char('#'), Some("\\#"));
        assert_eq!(escape_char('_'), Some("\\_"));
        assert_eq!(escape_char('%'), Some("\\%"));
        assert_eq!(escape_char('~'), Some("\\textasciitilde{}"));
        assert_eq!(escape_char('^'), Some("\\textasciicircum{}"));
        assert_eq!(escape_char('<'), Some("\\(<\\)"));
        assert_eq!(escape_char('>'), Some("\\(>\\)"));
        assert_eq!(escape_char(' '), Some("~"));
        assert_eq!(escape_char('\t'), Some("~~~~"));
        assert_eq!(escape_char('\r'), Some(""));
        // Pass-through: everything else, including `|` and non-ASCII.
        for c in [
            'a', '|', '=', '"', '\'', '`', '/', '-', ':', ';', ',', '!', '?', 'é', 'λ',
        ] {
            assert_eq!(escape_char(c), None, "{c:?} must pass through");
        }
    }

    #[test]
    fn render_block_escapes_and_expands_a_torture_line() {
        let (out, _, _, _) = render(
            "\ta = b # $ % ^ & _ { } ~ <>\n",
            None,
            &opts("main.tex", 3, false),
        );
        // The stanza shape carries the overlay; the payload itself must be
        // byte-for-byte the escaped torture line.
        assert!(
            out.starts_with("\\par\\medskip\n{\n\\tfxcodestyle"),
            "out: {out}"
        );
        assert!(out.ends_with("\\par\n}"), "out: {out}");
        assert!(
            out.contains(
                "~~~~a~=~b~\\#~\\$~\\%~\\textasciicircum{}~\\&~\\_~\\{~\\}~\\textasciitilde{}~\\(<\\)\\(>\\)"
            ),
            "the escaped payload must survive byte-for-byte: {out}"
        );
        assert!(out.contains("\\textcolor{tfxtint}"), "out: {out}");
        assert!(out.contains("\\textcolor{tfxframe}"), "out: {out}");
    }

    #[test]
    fn frame_stanza_shape() {
        let (out, _, _, _) = render("a\nb", None, &opts("main.tex", 5, false));
        assert!(
            out.starts_with("\\par\\medskip\n{\n\\tfxcodestyle"),
            "out: {out}"
        );
        assert!(out.ends_with("\\par\n}"), "out: {out}");
        // One framed paragraph per source line, blank line between.
        assert_eq!(out.matches("\\noindent ").count(), 2, "out: {out}");
    }

    #[test]
    fn tint_border_and_padding_rules_emitted() {
        let (out, _, _, _) = render("a\nb", None, &opts("main.tex", 1, false));
        assert!(out.contains("\\textcolor{tfxtint}"), "tint: {out}");
        assert!(out.contains("\\rule[-3pt]{0.4pt}"), "out: {out}");
        assert!(out.contains("\\kern4pt"), "4pt left padding: {out}");
        // Exactly one top border (first line) and one bottom border (last).
        assert_eq!(
            out.matches("\\rule[\\baselineskip]{\\dimexpr\\linewidth-0.8pt\\relax}{0.4pt}")
                .count(),
            1,
            "one top border: {out}"
        );
        assert_eq!(
            out.matches("\\rule[-6.4pt]{\\dimexpr\\linewidth-0.8pt\\relax}{0.4pt}")
                .count(),
            1,
            "one bottom border: {out}"
        );
    }

    #[test]
    fn single_line_block_gets_both_borders() {
        let (out, _, _, _) = render("a", None, &opts("main.tex", 1, false));
        assert_eq!(
            out.matches("\\rule[\\baselineskip]{\\dimexpr\\linewidth-0.8pt\\relax}{0.4pt}")
                .count(),
            1,
            "top border: {out}"
        );
        assert_eq!(
            out.matches("\\rule[-6.4pt]{\\dimexpr\\linewidth-0.8pt\\relax}{0.4pt}")
                .count(),
            1,
            "bottom border: {out}"
        );
        assert!(
            !out.contains("\\vadjust"),
            "a one-line block needs no break penalty: {out}"
        );
    }

    #[test]
    fn gutter_separator_rule_inside_the_frame() {
        let (out, _, _, _) = render("a", None, &opts("main.tex", 1, true));
        assert!(out.contains("\\kern4pt"), "out: {out}");
        // The `\hss` precedes the digit: the number is right-aligned in its
        // gutter box, hugging the separator rule (FR3).
        assert!(
            out.contains("\\textcolor{tfxgutter}{\\hbox to 2em{\\hss 1}}"),
            "right-aligned gutter: {out}"
        );
        assert!(
            out.contains("\\textcolor{tfxframe}{\\rule[-6.4pt]{0.3pt}"),
            "0.3pt separator: {out}"
        );
        assert!(out.contains("\\hspace{0.8em}"), "out: {out}");
        // Order: gutter, then separator, then gap, then code.
        let gutter = out.find("tfxgutter").unwrap();
        let separator = out.find("0.3pt").unwrap();
        let gap = out.find("\\hspace{0.8em}").unwrap();
        assert!(gutter < separator && separator < gap, "out: {out}");
    }

    #[test]
    fn break_penalties_on_first_and_last_pairs() {
        let (two, _, _, _) = render("a\nb", None, &opts("main.tex", 1, false));
        assert_eq!(two.matches("\\vadjust{\\penalty10000}").count(), 1, "{two}");
        assert!(two.contains("a\\vadjust{\\penalty10000}"), "{two}");

        let (three, _, _, _) = render("a\nb\nc", None, &opts("main.tex", 1, false));
        assert_eq!(
            three.matches("\\vadjust{\\penalty10000}").count(),
            2,
            "{three}"
        );

        let body = (1..=77)
            .map(|i| format!("l{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let (long, _, _, _) = render(&body, None, &opts("main.tex", 1, false));
        assert_eq!(
            long.matches("\\vadjust{\\penalty10000}").count(),
            2,
            "only lines 1 and 76 carry penalties"
        );
        assert!(long.contains("l1\\vadjust{\\penalty10000}"));
        assert!(long.contains("l76\\vadjust{\\penalty10000}"));
        assert!(!long.contains("l77\\vadjust"));
    }

    #[test]
    fn origins_map_every_output_line() {
        let o = EmitOpts {
            file: "main.tex",
            first_line: 4,
            body_line: 5,
            end_line: 9,
            numbers: false,
        };
        let (out, _, _, origins) = render("a\nb", None, &o);
        let lines: Vec<&str> = out.split('\n').collect();
        assert_eq!(origins.len(), lines.len(), "one origin per line: {out:?}");
        // Wrapper lines point at the \begin line; code lines at their own
        // body line; the blank separator at the line above.
        assert_eq!(&origins[0..4], &[4, 4, 4, 4]);
        assert_eq!(origins[4], 5);
        assert_eq!(origins[5], 5);
        assert_eq!(origins[6], 6);
        assert_eq!(origins[lines.len() - 2], 6);
        assert_eq!(origins[lines.len() - 1], 9);
    }

    #[test]
    fn blank_source_lines_become_real_lines() {
        let (out, _, _, _) = render("a\n\nb", None, &opts("main.tex", 1, false));
        assert!(
            out.contains("\\kern4pta\\vadjust{\\penalty10000}"),
            "out: {out}"
        );
        assert!(
            out.contains("\\kern4pt\\mbox{}\\vadjust{\\penalty10000}"),
            "out: {out}"
        );
        assert!(out.contains("\\kern4ptb\n\\par"), "out: {out}");
    }

    /// Numbering runs over *source* lines: a blank line still shows its
    /// number instead of vanishing from the sequence (1, 2, 4 → 1, 2, 3).
    #[test]
    fn numbered_blank_lines_keep_their_number() {
        let (out, _, _, _) = render("a\n\nb", None, &opts("main.tex", 1, true));
        assert!(out.contains("\\hbox to 2em{\\hss 1}"), "out: {out}");
        assert!(out.contains("\\hbox to 2em{\\hss 2}"), "out: {out}");
        assert!(out.contains("\\hbox to 2em{\\hss 3}"), "out: {out}");
        assert!(
            !out.contains("\\mbox{}"),
            "the gutter box is the line: {out}"
        );
    }

    #[test]
    fn adjacent_runs_of_one_colour_merge_into_one_textcolor() {
        let spans = vec![vec![
            Span {
                text: "def".to_string(),
                color: Some(Rgb::new(0xd7, 0x3a, 0x49)),
            },
            Span {
                text: " ".to_string(),
                color: Some(Rgb::new(0xd7, 0x3a, 0x49)),
            },
            Span {
                text: "x".to_string(),
                color: None,
            },
            Span {
                text: " = ".to_string(),
                color: None,
            },
        ]];
        let (out, used, _, _) = render("def x = ", Some(&spans), &opts("main.tex", 1, false));
        assert!(
            out.contains("\\textcolor{tfxcold73a49}{def~}"),
            "out: {out}"
        );
        assert!(out.contains("}x~=~"), "out: {out}");
        assert_eq!(used.len(), 1);
        assert!(used.contains(&Rgb::new(0xd7, 0x3a, 0x49)));
    }

    #[test]
    fn overlong_lines_warn_with_the_build_copy_line_number() {
        let long = "x".repeat(100);
        let body = format!("short\n{long}\nshort");
        let (_, _, warnings, _) = render(&body, None, &opts("chapters/one.tex", 11, false));
        assert_eq!(warnings.len(), 1, "warnings: {warnings:?}");
        assert_eq!(warnings[0].file, "chapters/one.tex");
        assert_eq!(warnings[0].line, 12);
        assert!(warnings[0].message.contains("100 chars wide"));
        assert!(warnings[0].message.contains("split it"));
    }

    #[test]
    fn short_lines_never_warn() {
        let (_, _, warnings, _) = render("ok\nfine", None, &opts("main.tex", 1, false));
        assert!(warnings.is_empty());
    }

    #[test]
    fn numbered_blocks_prefix_every_line_with_the_gutter() {
        let (out, _, _, _) = render("a\nb", None, &opts("main.tex", 1, true));
        assert!(out.contains("\\textcolor{tfxgutter}{\\hbox to 2em{\\hss 1}}"));
        assert!(out.contains("\\textcolor{tfxgutter}{\\hbox to 2em{\\hss 2}}"));
        assert!(out.contains("\\hspace{0.8em}"));
    }

    #[test]
    fn gutter_width_grows_past_nine_lines() {
        assert_eq!(gutter_width(1), 2);
        assert_eq!(gutter_width(9), 2);
        assert_eq!(gutter_width(12), 2);
        assert_eq!(gutter_width(123), 3);
    }

    #[test]
    fn gutter_counts_toward_the_overfull_limit() {
        // 87 characters + a 2-column gutter + gap = 90 → no warning …
        let line_87 = "x".repeat(87);
        let (_, _, warnings, _) = render(&line_87, None, &opts("main.tex", 1, true));
        assert!(warnings.is_empty(), "warnings: {warnings:?}");
        // … 88 characters → 91 → warning.
        let line_88 = "x".repeat(88);
        let (_, _, warnings, _) = render(&line_88, None, &opts("main.tex", 1, true));
        assert_eq!(warnings.len(), 1, "warnings: {warnings:?}");
        assert!(warnings[0].message.contains("91 chars wide"));
    }
}
