//! Character-level escaping and per-line paragraph emission for code blocks.
//!
//! The output is plain LaTeX kernel + `color.sty` only (route 3): every source
//! character is escaped so the author's text can never be interpreted, every
//! source line becomes its own paragraph so TeX may break the page between any
//! two lines, and spaces become `\tfxsp{}` (non-breaking) so lines never
//! wrap — which is what makes the Rust-side overfull check (rather than TeX's
//! log) the authoritative warning.

use std::collections::BTreeSet;

use crate::highlight::engine::{Rgb, Span};
use crate::highlight::palette::{BlockColors, FontStyle};
use crate::highlight::Warning;

/// A line longer than this many display columns (tab-expanded source plus the
/// line-number gutter) probably pokes past the default `\textwidth` at
/// `\small` cmtt (~4.7 pt per character × 90 ≈ 423 pt < 469 pt). Heuristic,
/// documented in `docs/listings.md`; TeX still reports its own overfull
/// hboxes, so this is a convenience, not a guarantee.
pub(crate) const OVERFULL_CHAR_LIMIT: usize = 90;

/// The caption vocabulary of one block: the author's text (raw LaTeX, like
/// the `caption=` contract) and the optional label placed right after the
/// counter step so `\ref`/`\pageref` resolve.
pub(crate) struct EmitCaption<'a> {
    pub(crate) text: &'a str,
    pub(crate) label: Option<&'a str>,
}

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
    pub(crate) caption: Option<EmitCaption<'a>>,
    /// Float placement (`pos=` other than `H`) — `None` renders the block
    /// inline, where it is written and may break across pages. Independent of
    /// the caption: a listing may be floated without being numbered.
    pub(crate) float: Option<&'a str>,
    /// `None` = default `small`; `Some("\\footnotesize")` etc.
    pub(crate) size_command: Option<&'static str>,
    /// This block's frame colours. A document may mix styles, so they travel
    /// with the block instead of living in one global set of names.
    pub(crate) colors: &'a BlockColors,
    /// The palette's base foreground, when the style needs it spelled out:
    /// a dark frame must not leave unscoped tokens on the document's black.
    /// `None` for the light styles, which inherit black exactly as before.
    pub(crate) base: Option<Rgb>,
}

/// Map one source character to its LaTeX expansion, or `None` when it passes
/// through untouched (`XeTeX` reads UTF-8 source, so every non-ASCII character
/// prints as itself).
///
/// `|` deliberately passes through: it prints fine in `cmtt`.
///
/// Spaces become `\tfxsp{}` (a `\nobreakspace`, see `preamble::injected_block`)
/// rather than a bare `~`: `~` is active under `babel` shorthands (notably
/// spanish) and misfires when followed by `}` (a `\textcolor` boundary, very
/// common) or `-` (as in `n - 1`). `\tfxsp{}` is identical glue without ever
/// emitting a `~` token.
pub(crate) fn escape_char(c: char) -> Option<&'static str> {
    match c {
        '\\' => Some("\\textbackslash{}"),
        '{' => Some("\\{"),
        '}' => Some("\\}"),
        '$' => Some("\\$"),
        '&' => Some("\\&"),
        '#' => Some("\\#"),
        '_' => Some("\\_"),
        // `%` passes through babel's `\%`, which spanish redefines to drop the
        // preceding interword glue and insert a `\,` thin space (see spanish.ldf
        // `\es@sppercent`), knocking every following glyph off the cell grid.
        // `\char37{}` prints the font's `%` without touching the preceding space —
        // the same defense as `"` below.
        '%' => Some("\\char37{}"),
        '~' => Some("\\textasciitilde{}"),
        '^' => Some("\\textasciicircum{}"),
        // `'` and `` ` `` must print as straight quotes: a literal U+0027/U+0060
        // is typeset as a curly quote under T1 (and reads back as `’`/`‘`), so
        // the listing would not be copy-pasteable. `textcomp` provides both and
        // is in the LaTeX kernel.
        '\'' => Some("\\textquotesingle{}"),
        '`' => Some("\\textasciigrave{}"),
        // `"` is active under `babel` shorthands (e.g. spanish): a literal
        // `"` would be misread as `\language@active@arg"`. `\char34{}` prints
        // the glyph without ever emitting a `"` character.
        '"' => Some("\\char34{}"),
        // `<`/`>` are active under `babel` shorthands (spanish quoting:
        // `<<`/`>>` guillemets plus the `system` single-character
        // shorthands), so raw angle brackets must never be emitted.
        // `\char60{}`/`\char62{}` print the font's own glyphs without ever
        // reading an active token — and, unlike the previous `\(<\)`/`\(>\)`
        // math wrap, occupy exactly one monospace cell: a math `>` measures
        // 8.48 pt against the 5.73 pt cell, which shifted every glyph after
        // a `->` or `=>` off the column grid.
        '<' => Some("\\char60{}"),
        '>' => Some("\\char62{}"),
        ' ' => Some("\\tfxsp{}"),
        '\t' => Some("\\tfxsp{}\\tfxsp{}\\tfxsp{}\\tfxsp{}"),
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
    // Tabs expand to 4 (emission turns each into four `\tfxsp{}`, so the width must
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
fn gutter(number: usize, width: usize, color: &str) -> String {
    format!("\\textcolor{{{color}}}{{\\hbox to {width}em{{\\hss {number}}}}}")
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
fn tint_rule(first: bool, last: bool, color: &str) -> String {
    const TEMPLATE: &str = "\\tfxsmash{\\rlap{\\kern0.4pt\\textcolor{@tint@}{\\rule[@raise@]{\\dimexpr\\linewidth-0.8pt\\relax}{@height@}}}}";
    let (raise, height) = geometry(first, last);
    fill(TEMPLATE, color, raise, &height)
}

/// Substitute one frame rule's placeholders. The rules are brace-dense
/// templates rather than format strings — see the note in [`end_rules`].
fn fill(template: &str, color: &str, raise: &'static str, height: &str) -> String {
    template
        .replace("@frame@", color)
        .replace("@tint@", color)
        .replace("@raise@", raise)
        .replace("@height@", height)
}

/// The 0.4pt left and right hairlines in the frame colour.
fn side_rules(first: bool, last: bool, color: &str) -> String {
    const LEFT: &str = "\\tfxsmash{\\rlap{\\textcolor{@frame@}{\\rule[@raise@]{0.4pt}{@height@}}}}";
    const RIGHT: &str = "\\tfxsmash{\\rlap{\\kern\\dimexpr\\linewidth-0.4pt\\relax\\textcolor{@frame@}{\\rule[@raise@]{0.4pt}{@height@}}}}";
    let (raise, height) = geometry(first, last);
    fill(LEFT, color, raise, &height) + &fill(RIGHT, color, raise, &height)
}

/// The horizontal borders: top rule on the first line, bottom rule on the
/// last (a single-line block gets both). Mid lines carry none, so a block
/// split across pages stays open at the break.
///
/// Written as templates with a `@frame@` placeholder rather than as format
/// strings: these rules are brace-dense, and doubling every literal brace by
/// hand is exactly the kind of edit that silently loses one.
fn end_rules(first: bool, last: bool, color: &str) -> String {
    const TOP: &str = "\\tfxsmash{\\rlap{\\kern0.4pt\\textcolor{@frame@}{\\rule[\\baselineskip]{\\dimexpr\\linewidth-0.8pt\\relax}{0.4pt}}}}";
    const BOTTOM: &str = "\\tfxsmash{\\rlap{\\kern0.4pt\\textcolor{@frame@}{\\rule[-6.4pt]{\\dimexpr\\linewidth-0.8pt\\relax}{0.4pt}}}}";
    let mut out = String::new();
    if first {
        out.push_str(&TOP.replace("@frame@", color));
    }
    if last {
        out.push_str(&BOTTOM.replace("@frame@", color));
    }
    out
}

/// The 0.3pt gutter separator in the frame colour (numbered blocks only).
/// Smashed like the rest, but not `\rlap`ped: its 0.3pt width is real spacing
/// between the numbers and the code.
fn separator_rule(first: bool, last: bool, color: &str) -> String {
    // Template form, as in `end_rules` (see the note there).
    const RULE: &str = "\\tfxsmash{\\textcolor{@frame@}{\\rule[@raise@]{0.3pt}{@height@}}}";
    let (raise, height) = geometry(first, last);
    fill(RULE, color, raise, &height)
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
fn emit_line(
    body: &str,
    line: usize,
    last: usize,
    numbers: bool,
    width: usize,
    colors: &BlockColors,
) -> String {
    let first = line == 1;
    let is_last = line == last;
    let mut out = String::from("\\noindent ");
    out.push_str(&tint_rule(first, is_last, &colors.tint));
    out.push_str(&side_rules(first, is_last, &colors.frame));
    out.push_str(&end_rules(first, is_last, &colors.frame));
    // 4pt inner padding between the frame and the code.
    out.push_str("\\kern4pt");
    if numbers {
        out.push_str(&gutter(line, width, &colors.gutter));
        out.push_str(&separator_rule(first, is_last, &colors.frame));
        out.push_str("\\hspace{0.8em}");
    }
    out.push_str(body);
    out.push_str(break_penalty(line, last));
    out
}

/// How one run of tokens is wrapped: its colour (if any) and its emphasis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Run {
    color: Option<Rgb>,
    font: FontStyle,
}

/// Emit one line's runs, merging adjacent spans that share colour *and*
/// emphasis, and leaving base-colour plain runs unwrapped (they are plain
/// black text; wrapping them would inflate the output for no visual change).
/// Emphasis exists only for the mono styles, where it replaces hue.
fn render_runs(spans: &[Span], used: &mut BTreeSet<Rgb>) -> String {
    let mut out = String::new();
    let mut current: Option<Run> = None;
    let mut buf = String::new();

    for span in spans {
        let run = Run {
            color: span.color,
            font: span.font,
        };
        if current != Some(run) {
            flush(current, &mut buf, &mut out, used);
            current = Some(run);
        }
        escape_into(&mut buf, &span.text);
    }
    flush(current, &mut buf, &mut out, used);
    out
}

/// Wrap and emit the pending run, then clear the buffer.
fn flush(run: Option<Run>, buf: &mut String, out: &mut String, used: &mut BTreeSet<Rgb>) {
    let Some(run) = run else { return };
    let emphasised = match run.font {
        FontStyle::Normal => buf.clone(),
        FontStyle::Bold => format!("\\textbf{{{}}}", buf),
        FontStyle::Italic => format!("\\textit{{{}}}", buf),
    };
    match run.color {
        Some(rgb) => {
            used.insert(rgb);
            out.push_str("\\textcolor{");
            out.push_str(&rgb.name());
            out.push_str("}{");
            out.push_str(&emphasised);
            out.push('}');
        }
        None => out.push_str(&emphasised),
    }
    buf.clear();
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
/// The frame colours come from `opts.colors` (per block, because a document
/// may mix styles) and `opts.base` adds the style's base foreground inside the
/// style group when the style needs one — `None` leaves the light styles
/// exactly as they were.
///
/// Structure: `\par\medskip` for an inline block (`\par` for a floated one),
/// then a group scoping `\tfxcodestyle` with one
/// framed `\noindent` paragraph per source line separated by blank lines (so
/// TeX may break the page between any two lines), then `\par` + `}`; an
/// inline block gets the trailing `\medskip` appended by the caller (a
/// floated one gets none — it has left the text flow), with the `\noindent`
/// for the next paragraph, once the caller knows what follows the block.
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
    let mut out_lines: Vec<String> = Vec::with_capacity(total * 2 + 10);
    let mut out_origins: Vec<usize> = Vec::with_capacity(total * 2 + 10);
    push_opening(&mut out_lines, &mut out_origins, opts);
    // The style's base foreground, inside the `{ … }` group: a dark frame
    // needs its light text colour spelled out, and the group keeps it off the
    // caption above and whatever other ink the block's surroundings carry.
    if let Some(base) = opts.base {
        used.insert(base);
        out_lines.push(format!("\\color{{{}}}", base.name()));
        out_origins.push(opts.first_line);
    }
    if let Some(size) = opts.size_command {
        out_lines.push(size.to_string());
        out_origins.push(opts.first_line);
    }
    out_lines.push(String::new());
    out_origins.push(opts.first_line);

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
        out_lines.push(emit_line(
            &rendered_body,
            i + 1,
            total,
            opts.numbers,
            width,
            opts.colors,
        ));
        out_origins.push(opts.body_line + i);
        if i + 1 < total {
            // Blank separator between two line paragraphs: Tectonic reports
            // an overfull paragraph on its end line, so attribute it to the
            // code line above, not below.
            out_lines.push(String::new());
            out_origins.push(opts.body_line + i);
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
    out_origins.push(opts.body_line + total.saturating_sub(1));
    out_lines.push("}".to_string());
    out_origins.push(opts.end_line);
    push_closing(&mut out_lines, &mut out_origins, opts);
    origins.extend(out_origins);
    out_lines.join("\n")
}

/// Opening lines: `\par\medskip` for an inline block (`\par` for a floated
/// one), an optional float wrapper, the caption lines, then the style
/// group. Caption/`\begin{figure}` lines map to the `\begin` line.
fn push_opening(out_lines: &mut Vec<String>, out_origins: &mut Vec<usize>, opts: &EmitOpts) {
    // A float leaves the text flow, so it carries no vertical rhythm of its
    // own; an inline block keeps its `\par\medskip`.
    let opening = if opts.float.is_some() {
        "\\par"
    } else {
        "\\par\\medskip"
    };
    out_lines.push(opening.to_string());
    out_origins.push(opts.first_line);
    if let Some(pos) = opts.float {
        out_lines.push(format!("\\begin{{figure}}[{pos}]"));
        out_origins.push(opts.first_line);
    }
    if let Some(caption) = opts.caption.as_ref() {
        render_caption_lines(out_lines, out_origins, opts, caption);
    }
    out_lines.push("{".to_string());
    out_origins.push(opts.first_line);
    out_lines.push("\\tfxcodestyle".to_string());
    out_origins.push(opts.first_line);
}

/// Caption lines: counter step (+ label), the bold "Name N:" line glued to
/// the frame's first line, and the list-of-listings entry.
fn render_caption_lines(
    out_lines: &mut Vec<String>,
    out_origins: &mut Vec<usize>,
    opts: &EmitOpts,
    caption: &EmitCaption,
) {
    let step = match caption.label {
        Some(label) => format!("\\refstepcounter{{tfxlisting}}\\label{{{label}}}%"),
        None => "\\refstepcounter{tfxlisting}%".to_string(),
    };
    out_lines.push(step);
    out_origins.push(opts.first_line);
    let glue = match opts.float {
        // Inline: forbid a page break right after the caption line so it can
        // never be orphaned from the frame's first line. `\penalty10000` is
        // LaTeX's `\nobreak` value (`\@M`) — TeX's *forced* break is `-10000`.
        // The `\kern\medskipamount` that follows pushes the frame down so its
        // first tint line no longer reaches back over the caption. The penalty
        // must stay first: a break after a kern that follows a penalty is not
        // legal. In a float the caption and the frame already share one box.
        Some(_) => "\\vadjust{\\kern\\medskipamount}".to_string(),
        None => "\\vadjust{\\penalty10000\\kern\\medskipamount}".to_string(),
    };
    out_lines.push(format!(
        "\\noindent{{\\normalfont\\textbf{{\\tfxlistingname~\\thetfxlisting:}}~{}}}{}\\par",
        caption.text, glue
    ));
    out_origins.push(opts.first_line);
    out_lines.push(format!(
        "\\addcontentsline{{lol}}{{listing}}{{\\protect\\numberline{{\\thetfxlisting}}{}}}",
        caption.text
    ));
    out_origins.push(opts.first_line);
}

/// Closing lines: `\end{figure}` for a float, mapping to the `\end` line.
fn push_closing(out_lines: &mut Vec<String>, out_origins: &mut Vec<usize>, opts: &EmitOpts) {
    if opts.float.is_some() {
        out_lines.push("\\end{figure}".to_string());
        out_origins.push(opts.end_line);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::LazyLock;

    use super::*;

    fn opts(file: &str, body_line: usize, numbers: bool) -> EmitOpts<'_> {
        EmitOpts {
            file,
            first_line: body_line.saturating_sub(1),
            body_line,
            end_line: body_line + 100,
            numbers,
            caption: None,
            float: None,
            size_command: None,
            colors: &LEGACY,
            base: None,
        }
    }

    fn caption_opts<'a>(
        file: &'a str,
        body_line: usize,
        text: &'a str,
        label: Option<&'a str>,
        float: Option<&'a str>,
        size_command: Option<&'static str>,
    ) -> EmitOpts<'a> {
        EmitOpts {
            file,
            first_line: body_line.saturating_sub(1),
            body_line,
            end_line: body_line + 100,
            numbers: false,
            caption: Some(EmitCaption { text, label }),
            float,
            size_command,
            colors: &LEGACY,
            base: None,
        }
    }

    /// The legacy light frame: the fixed `tfxtint`/`tfxframe`/`tfxgutter`
    /// names and no base-colour line, which is what every existing snapshot
    /// was rendered with.
    static LEGACY: LazyLock<BlockColors> = LazyLock::new(|| BlockColors {
        tint: "tfxtint".to_string(),
        frame: "tfxframe".to_string(),
        gutter: "tfxgutter".to_string(),
    });

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
        assert_eq!(escape_char('%'), Some("\\char37{}"));
        assert_eq!(escape_char('\''), Some("\\textquotesingle{}"));
        assert_eq!(escape_char('`'), Some("\\textasciigrave{}"));
        assert_eq!(escape_char('~'), Some("\\textasciitilde{}"));
        assert_eq!(escape_char('^'), Some("\\textasciicircum{}"));
        assert_eq!(escape_char('"'), Some("\\char34{}"));
        assert_eq!(escape_char('<'), Some("\\char60{}"));
        assert_eq!(escape_char('>'), Some("\\char62{}"));
        assert_eq!(escape_char(' '), Some("\\tfxsp{}"));
        assert_eq!(
            escape_char('\t'),
            Some("\\tfxsp{}\\tfxsp{}\\tfxsp{}\\tfxsp{}")
        );
        assert_eq!(escape_char('\r'), Some(""));
        // Pass-through: everything else, including `|` and non-ASCII.
        for c in ['a', '|', '=', '/', '-', ':', ';', ',', '!', '?', 'é', 'λ'] {
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
                "\\tfxsp{}\\tfxsp{}\\tfxsp{}\\tfxsp{}a\\tfxsp{}=\\tfxsp{}b\\tfxsp{}\\#\\tfxsp{}\\$\\tfxsp{}\\char37{}\\tfxsp{}\\textasciicircum{}\\tfxsp{}\\&\\tfxsp{}\\_\\tfxsp{}\\{\\tfxsp{}\\}\\tfxsp{}\\textasciitilde{}\\tfxsp{}\\char60{}\\char62{}"
            ),
            "the escaped payload must survive byte-for-byte: {out}"
        );
        assert!(out.contains("\\textcolor{tfxtint}"), "out: {out}");
        assert!(out.contains("\\textcolor{tfxframe}"), "out: {out}");
    }

    /// Spaces must never reach the engine as a bare `~`: under spanish
    /// `babel` a `~` at the `{...}` boundary of a `\textcolor` group (very
    /// common) aborts with "extra }" (TeX never saw its matching `{`), and
    /// one followed by `-` (as in `n - 1`) with "Bad character code (-1)".
    /// `\tfxsp{}` is the same glue without ever emitting a `~` token.
    #[test]
    fn spaces_never_emit_a_bare_tilde() {
        let (out, _, _, _) = render("a - b # c\n", None, &opts("main.tex", 3, false));
        assert!(
            out.contains("a\\tfxsp{}-\\tfxsp{}b\\tfxsp{}\\#\\tfxsp{}c"),
            "spaces and hyphen must survive byte-for-byte: {out}"
        );
        let payload = &out[out.find("\\kern4pt").unwrap()..out.find("\\par\n}").unwrap()];
        let stripped = payload.replace("\\textasciitilde{}", "");
        assert!(!stripped.contains('~'), "no bare tilde may survive: {out}");
        assert!(
            !stripped.contains('"'),
            "no raw double quote may survive: {out}"
        );
    }

    #[test]
    fn double_quote_never_reaches_the_engine_raw() {
        // `"` is active under `babel` shorthands (spanish): it must be
        // emitted as `\char34{}` so the output contains no raw `"` at all.
        let (out, _, _, _) = render("a = \"hi\"\n", None, &opts("main.tex", 3, false));
        assert!(
            out.contains("a\\tfxsp{}=\\tfxsp{}\\char34{}hi\\char34{}"),
            "out: {out}"
        );
        assert!(!out.contains('"'), "no raw double quote may survive: {out}");
    }

    /// Req 2/3: the space between `$` and `%` is one `\tfxsp{}` cell; `%` must
    /// never reach babel's space-eating `\%` (spanish `\es@sppercent`).
    #[test]
    fn dollar_space_percent_keeps_a_fixed_width_cell() {
        let (out, _, _, _) = render("$ %", None, &opts("main.tex", 1, false));
        assert!(out.contains("\\$\\tfxsp{}\\char37{}"), "out: {out}");
        assert!(
            !out.contains("\\%"),
            "babel's percent must never be emitted: {out}"
        );
    }

    /// Req 1/3: straight quotes/backtick are textcomp commands, not raw chars.
    #[test]
    fn straight_quotes_and_backtick_use_textcomp_commands() {
        let (out, _, _, _) = render("'`", None, &opts("main.tex", 1, false));
        assert!(
            out.contains("\\textquotesingle{}\\textasciigrave{}"),
            "out: {out}"
        );
        assert!(!out.contains('\''), "no raw apostrophe may survive: {out}");
        // Backtick check must exclude the wrapper's own backslashes:
        assert!(!out.contains("`"), "no raw backtick may survive: {out}");
    }

    /// Req 2: `<`/`>` print as one exact monospace cell — the old math wrap
    /// `\(<\)`/`\(>\)` typeset a 8.48 pt math glyph against the 5.73 pt
    /// cell, shifting everything after `->`/`=>` off the column grid — and
    /// they never reach `babel`'s active spanish quoting shorthands.
    #[test]
    fn angle_brackets_print_one_exact_cell_without_math() {
        let (out, _, _, _) = render("a -> b < c\n", None, &opts("main.tex", 1, false));
        assert!(
            out.contains("a\\tfxsp{}-\\char62{}\\tfxsp{}b\\tfxsp{}\\char60{}\\tfxsp{}c"),
            "out: {out}"
        );
        assert!(!out.contains("\\("), "no math wrap may survive: {out}");
        assert!(
            !out.contains('<') && !out.contains('>'),
            "no raw angle bracket may survive: {out}"
        );
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
            caption: None,
            float: None,
            size_command: None,
            colors: &LEGACY,
            base: None,
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

    /// An empty body still renders one (empty) source line — the block is a
    /// real stanza, not a hole in the output.
    #[test]
    fn empty_body_still_renders_one_source_line() {
        let (out, _, _, origins) = render("", None, &opts("main.tex", 1, false));
        assert!(
            out.contains("\\mbox{}"),
            "an empty body must still emit its line: {out}"
        );
        assert!(
            origins.len() > 4,
            "the empty source line carries an origin: {origins:?}"
        );
    }

    /// A direct caller that passes a trailing newline gets the same stanza as
    /// one that does not: the lone trailing segment is not a source line.
    #[test]
    fn a_trailing_newline_does_not_add_a_source_line() {
        let (with_newline, _, _, origins_nl) = render("a\n", None, &opts("main.tex", 1, false));
        let (without, _, _, origins) = render("a", None, &opts("main.tex", 1, false));
        assert_eq!(with_newline, without);
        assert_eq!(origins_nl, origins);
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
                font: FontStyle::Normal,
            },
            Span {
                text: " ".to_string(),
                color: Some(Rgb::new(0xd7, 0x3a, 0x49)),
                font: FontStyle::Normal,
            },
            Span {
                text: "x".to_string(),
                color: None,
                font: FontStyle::Normal,
            },
            Span {
                text: " = ".to_string(),
                color: None,
                font: FontStyle::Normal,
            },
        ]];
        let (out, used, _, _) = render("def x = ", Some(&spans), &opts("main.tex", 1, false));
        assert!(
            out.contains("\\textcolor{tfxcold73a49}{def\\tfxsp{}}"),
            "out: {out}"
        );
        assert!(out.contains("}x\\tfxsp{}=\\tfxsp{}"), "out: {out}");
        assert_eq!(used.len(), 1);
        assert!(used.contains(&Rgb::new(0xd7, 0x3a, 0x49)));
    }

    /// The mono styles carry the hierarchy in weight and slant instead of
    /// hue: a keyword is `\textbf`, a comment `\textit`, and both merge with
    /// their neighbours only when the emphasis matches too.
    #[test]
    fn font_runs_emit_textbf_and_textit() {
        let spans = vec![vec![
            Span {
                text: "def ".to_string(),
                color: Some(Rgb::new(0, 0, 0)),
                font: FontStyle::Bold,
            },
            Span {
                text: "f".to_string(),
                color: Some(Rgb::new(0, 0, 0)),
                font: FontStyle::Bold,
            },
            Span {
                text: " # note".to_string(),
                color: Some(Rgb::new(0x6e, 0x6e, 0x6e)),
                font: FontStyle::Italic,
            },
            Span {
                text: "plain".to_string(),
                color: None,
                font: FontStyle::Normal,
            },
        ]];
        let (out, used, _, _) = render(
            "def f # note plain",
            Some(&spans),
            &opts("main.tex", 1, false),
        );
        assert!(
            out.contains("\\textcolor{tfxcol000000}{\\textbf{def\\tfxsp{}f}}"),
            "bold keyword, merged: {out}"
        );
        assert!(
            out.contains("\\textcolor{tfxcol6e6e6e}{\\textit{\\tfxsp{}\\#\\tfxsp{}note}}"),
            "italic comment: {out}"
        );
        assert!(
            out.contains("note}}plain\n\\par"),
            "the base-colour run stays unwrapped: {out}"
        );
        assert_eq!(used.len(), 2, "the base colour is the document's: {used:?}");
    }

    /// An emphasised run with no colour of its own still needs its wrapper —
    /// the mono palettes paint some tokens in the base colour, which syntect
    /// reports as `None`.
    #[test]
    fn emphasis_survives_a_run_without_a_colour() {
        let spans = vec![vec![Span {
            text: "def".to_string(),
            color: None,
            font: FontStyle::Bold,
        }]];
        let (out, used, _, _) = render("def", Some(&spans), &opts("main.tex", 1, false));
        assert!(out.contains("\\kern4pt\\textbf{def}"), "out: {out}");
        assert!(used.is_empty(), "no colour to define: {used:?}");
    }

    /// A dark frame's base colour is emitted inside the style group, once per
    /// block; a light block emits nothing there and stays byte-identical.
    #[test]
    fn base_colour_line_is_emitted_only_when_set() {
        let (plain, _, _, origins) = render("a", None, &opts("main.tex", 1, false));
        assert!(
            !plain.contains("\\color{"),
            "the light styles emit no base colour line: {plain}"
        );
        assert_eq!(origins.len(), plain.lines().count());

        let dark = BlockColors {
            tint: "tfxcol22272e".to_string(),
            frame: "tfxcol8b949e".to_string(),
            gutter: "tfxcol8b949e".to_string(),
        };
        let o = EmitOpts {
            colors: &dark,
            base: Some(Rgb::new(0xad, 0xba, 0xc7)),
            ..opts("main.tex", 1, false)
        };
        let (out, used, _, origins) = render("a", None, &o);
        assert!(
            out.contains("{\n\\tfxcodestyle\n\\color{tfxcoladbac7}"),
            "{out}"
        );
        assert!(
            out.contains("\\textcolor{tfxcol22272e}"),
            "dark tint: {out}"
        );
        assert!(
            out.contains("\\textcolor{tfxcol8b949e}"),
            "dark frame: {out}"
        );
        assert_eq!(origins.len(), out.lines().count(), "one origin per line");
        assert!(
            used.contains(&Rgb::new(0xad, 0xba, 0xc7)),
            "the base colour needs a \\definecolor: {used:?}"
        );
    }

    /// The base colour must not leak out of the group: the `\medskip` that
    /// follows the block is document text.
    #[test]
    fn base_colour_stays_inside_the_style_group() {
        let dark = BlockColors {
            tint: "tfxcol22272e".to_string(),
            frame: "tfxcol8b949e".to_string(),
            gutter: "tfxcol8b949e".to_string(),
        };
        let o = EmitOpts {
            colors: &dark,
            base: Some(Rgb::new(0xad, 0xba, 0xc7)),
            ..opts("main.tex", 1, false)
        };
        let (out, _, _, _) = render("a", None, &o);
        let color = out.find("\\color{tfxcoladbac7}").unwrap();
        let close = out.find("\n}").unwrap();
        assert!(
            color < close,
            "the colour must precede the group's end: {out}"
        );
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

    #[test]
    fn caption_line_precedes_frame_and_glues_to_the_first_line() {
        let o = caption_opts("main.tex", 5, "Hello", Some("lst:hi"), None, None);
        let (out, _, _, _) = render("x = 1", None, &o);
        assert!(
            out.contains("\\refstepcounter{tfxlisting}\\label{lst:hi}%"),
            "{out}"
        );
        assert!(out.contains("\\tfxlistingname~\\thetfxlisting:"), "{out}");
        assert!(
            out.contains("\\vadjust{\\penalty10000\\kern\\medskipamount}\\par"),
            "{out}"
        );
        assert!(out.contains("\\addcontentsline{lol}{listing}"), "{out}");
        let caption = out.find("\\refstepcounter").unwrap();
        let frame = out.find("\\tfxcodestyle").unwrap();
        assert!(caption < frame, "caption must precede the frame: {out}");
    }

    #[test]
    fn float_wraps_the_caption_and_frame_in_a_figure() {
        let o = caption_opts("main.tex", 5, "Hello", None, Some("t"), None);
        let (out, _, _, _) = render("x = 1", None, &o);
        assert!(out.contains("\\begin{figure}[t]"), "{out}");
        assert!(out.contains("\\end{figure}"), "{out}");
        let begin = out.find("\\begin{figure}").unwrap();
        let caption = out.find("\\refstepcounter").unwrap();
        let end = out.find("\\end{figure}").unwrap();
        assert!(begin < caption && caption < end, "{out}");
    }

    /// A floated block lives in its own figure box: the caption glue carries
    /// no page-break penalty (caption and frame already share one box), only
    /// the `\kern\medskipamount`, and the block opens with `\par`, not
    /// `\par\medskip`.
    #[test]
    fn float_caption_glues_without_a_penalty_and_opens_with_par() {
        let o = caption_opts("main.tex", 5, "Hello", None, Some("t"), None);
        let (out, _, _, _) = render("x = 1", None, &o);
        assert!(
            out.contains("\\vadjust{\\kern\\medskipamount}\\par"),
            "{out}"
        );
        assert!(!out.contains("\\par\\medskip"), "{out}");
        assert_eq!(out.lines().next(), Some("\\par"), "{out}");
    }

    /// `pos=` floats a listing whether or not it is captioned: the placement
    /// is a property of the block, the caption only adds numbering.
    #[test]
    fn float_without_a_caption_wraps_the_frame() {
        let mut o = opts("main.tex", 5, false);
        o.float = Some("b");
        let (out, _, _, origins) = render("x = 1", None, &o);
        let begin = out.find("\\begin{figure}[b]").unwrap();
        let frame = out.find("\\tfxcodestyle").unwrap();
        let end = out.find("\\end{figure}").unwrap();
        assert!(begin < frame && frame < end, "{out}");
        assert!(!out.contains("tfxlisting"), "no caption machinery: {out}");
        assert_eq!(origins.len(), out.lines().count(), "{out:?}");
    }

    #[test]
    fn inline_caption_has_no_float_and_ends_the_stanza() {
        let o = caption_opts("main.tex", 5, "Hi", None, None, None);
        let (out, _, _, _) = render("x", None, &o);
        assert!(!out.contains("figure"), "{out}");
        assert!(
            out.contains("\\vadjust{\\penalty10000\\kern\\medskipamount}"),
            "{out}"
        );
    }

    #[test]
    fn size_command_is_appended_after_codestyle_only_when_set() {
        let o = caption_opts("main.tex", 5, "Hi", None, None, Some("\\footnotesize"));
        let (out, _, _, _) = render("x", None, &o);
        let style = out.find("\\tfxcodestyle").unwrap();
        let size = out.find("\\footnotesize").unwrap();
        assert!(style < size, "{out}");
        let (plain, _, _, _) = render("x", None, &opts("main.tex", 5, false));
        assert!(!plain.contains("\\footnotesize"), "{plain}");
    }

    #[test]
    fn origins_cover_caption_and_float_lines() {
        let o = caption_opts("main.tex", 5, "Hi", Some("lst:x"), Some("t"), None);
        let (out, _, _, origins) = render("a\nb", None, &o);
        assert_eq!(origins.len(), out.lines().count(), "{out:?}");
    }
}
