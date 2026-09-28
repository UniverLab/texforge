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
/// Tectonic reports errors in), the line the block opens on, and whether the
/// line-number gutter is on.
pub(crate) struct EmitOpts<'a> {
    pub(crate) file: &'a str,
    /// 1-based line of the `\begin{code}` / `\begin{lstlisting}` opener.
    pub(crate) first_line: usize,
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

/// The `\noindent` paragraph for one numbered (or unnumbered) line.
fn gutter(number: usize, width: usize) -> String {
    format!("\\textcolor{{tfxgutter}}{{\\hbox to {width}em{{{number}\\hss}}}}\\hspace{{0.8em}}")
}

/// Width of the gutter: at least two columns, wider only when the block has
/// ten or more lines.
fn gutter_width(line_count: usize) -> usize {
    let digits = line_count.to_string().len();
    digits.max(2)
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
///
/// Structure: a group that scopes `\tfxcodestyle`, then one `\noindent`
/// paragraph per source line separated by blank lines (so TeX may break the
/// page between any two lines), then an explicit `\par` so the group can
/// close without swallowing whatever the author wrote next.
pub(crate) fn render_block(
    body: &str,
    spans: Option<&[Vec<Span>]>,
    opts: &EmitOpts,
    used: &mut BTreeSet<Rgb>,
    warnings: &mut Vec<Warning>,
) -> String {
    let mut lines: Vec<&str> = body.split('\n').collect();
    // The block scanner strips the body's trailing newline before calling;
    // tolerate direct callers that did not (a lone trailing segment is not a
    // source line).
    if lines.len() > 1 && lines.last() == Some(&"") {
        lines.pop();
    }
    let width = gutter_width(lines.len());
    let mut out = String::from("{\n\\tfxcodestyle\n\n");

    for (i, line) in lines.iter().enumerate() {
        if i > 0 {
            out.push('\n');
            out.push('\n');
        }
        out.push_str("\\noindent");
        if line.is_empty() {
            // A blank source line still occupies a real line of output.
            out.push_str("\\mbox{}");
            continue;
        }
        // The space after the control word is skipped by TeX; without it the
        // first code character would glue onto `\noindent` and form an
        // undefined control sequence (`\noindenta`).
        out.push(' ');
        if opts.numbers {
            out.push_str(&gutter(i + 1, width));
        }
        match spans.and_then(|s| s.get(i)) {
            Some(line_spans) => out.push_str(&render_runs(line_spans, used)),
            None => escape_into(&mut out, line),
        }

        let width_of_line = display_width(line, opts.numbers, width);
        if width_of_line > OVERFULL_CHAR_LIMIT {
            warnings.push(Warning {
                file: opts.file.to_string(),
                line: opts.first_line + 1 + i,
                message: format!(
                    "code line is {width_of_line} chars wide — it may exceed the text width; split it"
                ),
            });
        }
    }

    out.push_str("\n\\par\n}");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(file: &str, first_line: usize, numbers: bool) -> EmitOpts<'_> {
        EmitOpts {
            file,
            first_line,
            numbers,
        }
    }

    fn render(
        body: &str,
        spans: Option<&[Vec<Span>]>,
        o: &EmitOpts,
    ) -> (String, BTreeSet<Rgb>, Vec<Warning>) {
        let mut used = BTreeSet::new();
        let mut warnings = Vec::new();
        let out = render_block(body, spans, o, &mut used, &mut warnings);
        (out, used, warnings)
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
        let (out, _, _) = render(
            "\ta = b # $ % ^ & _ { } ~ <>\n",
            None,
            &opts("main.tex", 3, false),
        );
        assert_eq!(
            out,
            "{\n\\tfxcodestyle\n\n\\noindent ~~~~a~=~b~\\#~\\$~\\%~\\textasciicircum{}~\\&~\\_~\\{~\\}~\\textasciitilde{}~\\(<\\)\\(>\\)\n\\par\n}"
        );
    }

    #[test]
    fn blank_source_lines_become_real_lines() {
        let (out, _, _) = render("a\n\nb", None, &opts("main.tex", 1, false));
        assert!(out.contains("\\noindent a\n\n\\noindent\\mbox{}\n\n\\noindent b"));
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
        let (out, used, _) = render("def x = ", Some(&spans), &opts("main.tex", 1, false));
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
        let (_, _, warnings) = render(&body, None, &opts("chapters/one.tex", 10, false));
        assert_eq!(warnings.len(), 1, "warnings: {warnings:?}");
        assert_eq!(warnings[0].file, "chapters/one.tex");
        assert_eq!(warnings[0].line, 12);
        assert!(warnings[0].message.contains("100 chars wide"));
        assert!(warnings[0].message.contains("split it"));
    }

    #[test]
    fn short_lines_never_warn() {
        let (_, _, warnings) = render("ok\nfine", None, &opts("main.tex", 1, false));
        assert!(warnings.is_empty());
    }

    #[test]
    fn numbered_blocks_prefix_every_line_with_the_gutter() {
        let (out, _, _) = render("a\nb", None, &opts("main.tex", 1, true));
        assert!(out.contains("\\textcolor{tfxgutter}{\\hbox to 2em{1\\hss}}"));
        assert!(out.contains("\\textcolor{tfxgutter}{\\hbox to 2em{2\\hss}}"));
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
        let (_, _, warnings) = render(&line_87, None, &opts("main.tex", 1, true));
        assert!(warnings.is_empty(), "warnings: {warnings:?}");
        // … 88 characters → 91 → warning.
        let line_88 = "x".repeat(88);
        let (_, _, warnings) = render(&line_88, None, &opts("main.tex", 1, true));
        assert_eq!(warnings.len(), 1, "warnings: {warnings:?}");
        assert!(warnings[0].message.contains("91 chars wide"));
    }
}
