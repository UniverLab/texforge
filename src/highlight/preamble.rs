//! Preamble injection and collision checks for code listings.
//!
//! Injection happens **in the build copy only** — the author's sources are
//! never touched (the same guarantee the diagram pass gives). The block is
//! inserted immediately before `\begin{document}` of the entry file, and only
//! when at least one code block was actually rewritten; a document without
//! code blocks is byte-identical to what it was before this feature existed.

use std::collections::BTreeSet;
use std::path::Path;

use anyhow::{Context, Result};

use crate::texparse;
use crate::texutil;

use super::caption;
use super::engine::Rgb;
use super::palette::{self, HighlightStyle, HighlightTheme, Palette};

pub(crate) const BEGIN_MARKER: &str =
    "% ---- texforge code listings (injected; do not edit, rebuild to refresh) ----";
pub(crate) const END_MARKER: &str = "% ---- end texforge code listings ----";

/// Build the injected preamble block.
///
/// * `colors` — every colour the rewritten blocks actually use, sorted
///   (a `BTreeSet`), so the block is deterministic;
/// * `has_gutter` — at least one numbered block exists;
/// * `color_pkg_visible_load` — the author already loads `color`/`xcolor`
///   themselves; the `\usepackage{color}` inside the guard is then dropped
///   (the guard itself stays, as defense in depth for classes such as beamer
///   that load `xcolor` behind the scan's back);
/// * `theme` — the active highlight theme, which picks the *light* palette
///   whose tint/frame/gutter get the fixed `tfxtint`/`tfxframe`/`tfxgutter`
///   names. Any other style brings its own names through `colors`, so a
///   document mixing styles needs no extra preamble lines.
/// * `caption_names` — `Some` when at least one block carries `caption=`;
///   appends the listing counter, the chapter-aware numbering and
///   `\listoflistings`.
pub(crate) fn injected_block(
    colors: &BTreeSet<Rgb>,
    has_gutter: bool,
    color_pkg_visible_load: bool,
    theme: HighlightTheme,
) -> String {
    injected_block_with_caption(colors, has_gutter, None, color_pkg_visible_load, theme)
}

/// [`injected_block`] with an optional caption machinery section.
pub(crate) fn injected_block_with_caption(
    colors: &BTreeSet<Rgb>,
    has_gutter: bool,
    caption_names: Option<&caption::Names>,
    color_pkg_visible_load: bool,
    theme: HighlightTheme,
) -> String {
    let frame = light_palette(theme);
    let load_color = if color_pkg_visible_load {
        ""
    } else {
        r"\usepackage{color}"
    };
    // The guard double-checks at runtime what the Rust-side scan above
    // already knows, so a class that loads `xcolor` itself (beamer) can
    // never see a second package load.
    const GUARD: &str = r"\@ifpackageloaded{color}{}{\@ifpackageloaded{xcolor}{}{@PKG@}}%";
    let mut out = String::new();
    out.push_str(BEGIN_MARKER);
    out.push('\n');
    out.push_str(r"\makeatletter");
    out.push('\n');
    out.push_str(&GUARD.replace("@PKG@", load_color));
    out.push('\n');
    out.push_str(r"\makeatother");
    out.push('\n');
    out.push_str(
        r"\newcommand{\tfxcodestyle}{\ttfamily\small\setlength{\parindent}{0pt}\setlength{\parskip}{0pt}}",
    );
    out.push('\n');
    // Zero-metric overlay helper: the frame rules carry ink but report no
    // size, so page breaking and line widths stay exactly as without it.
    out.push_str(r"\newcommand{\tfxsmash}[1]{\setbox0=\hbox{#1}\ht0=0pt\dp0=0pt\box0}");
    out.push('\n');
    // Non-breaking space for code lines: identical glue to `~`, but a bare
    // `~` is active under `babel` shorthands (spanish) and misfires before
    // `}` (a `\textcolor` boundary) or `-` (as in `n - 1`), so spaces are
    // emitted as `\tfxsp{}` and never as a `~` token.
    out.push_str(r"\newcommand{\tfxsp}{\nobreakspace{}}");
    out.push('\n');
    out.push_str(&format!(
        "\\definecolor{{tfxtint}}{{rgb}}{{{}}}\n",
        frame.tint.to_rgb_list()
    ));
    out.push_str(&format!(
        "\\definecolor{{tfxframe}}{{rgb}}{{{}}}\n",
        frame.frame.to_rgb_list()
    ));
    for color in colors {
        out.push_str(&format!(
            "\\definecolor{{{}}}{{rgb}}{{{}}}\n",
            color.name(),
            color.to_rgb_list()
        ));
    }
    if has_gutter {
        out.push_str(&format!(
            "\\definecolor{{tfxgutter}}{{rgb}}{{{}}}\n",
            frame.gutter.to_rgb_list()
        ));
    }
    if let Some(names) = caption_names {
        out.push_str(&caption_machinery(names));
    }
    out.push_str(END_MARKER);
    out.push('\n');
    out
}

/// The palette behind the fixed frame colour names. Always the `light`
/// style: those three names describe one frame, and a document using only
/// other styles has its colours in `colors` instead.
fn light_palette(theme: HighlightTheme) -> &'static Palette {
    palette::palette(theme, HighlightStyle::Light)
}

/// LaTeX caption machinery: the listing counter (reset per chapter when the
/// class defines `\chapter`), the list-of-listings command and the
/// language-resolved names. Tokenised while `\makeatletter` is active so
/// `\@`-commands are safe.
fn caption_machinery(names: &caption::Names) -> String {
    let mut out = String::new();
    out.push_str("\\makeatletter\n");
    out.push_str("\\newcounter{tfxlisting}\n");
    out.push_str("\\@ifundefined{chapter}{}{%\n");
    out.push_str("  \\@addtoreset{tfxlisting}{chapter}%\n");
    out.push_str("  \\renewcommand{\\thetfxlisting}{\\thechapter.\\arabic{tfxlisting}}%\n");
    out.push_str("}\n");
    out.push_str("\\providecommand{\\listoflistings}{%\n");
    out.push_str(
        "  \\@ifundefined{chapter}{\\section*{\\tfxlistname}}{\\chapter*{\\tfxlistname}}%\n",
    );
    out.push_str("  \\@starttoc{lol}}\n");
    out.push_str("\\providecommand*\\l@listing{\\@dottedtocline{1}{1.5em}{2.3em}}\n");
    out.push_str("\\makeatother\n");
    out.push_str(&format!(
        "\\newcommand{{\\tfxlistingname}}{{{}}}\n",
        names.listing
    ));
    out.push_str(&format!(
        "\\newcommand{{\\tfxlistname}}{{{}}}\n",
        names.list
    ));
    out
}

/// Does any of these sources visibly load `color` or `xcolor`?
///
/// Text scan on comment-stripped lines (so a commented-out `\usepackage`
/// does not count), skipping verbatim body lines (so a `\usepackage{xcolor}`
/// quoted as sample code does not count — it is escaped text, not a real
/// load; without this a code example would suppress the `\usepackage{color}`
/// the listing actually needs). The runtime `\@ifpackageloaded` guard is the
/// correctness mechanism; this scan only spares the author a redundant line.
pub(crate) fn color_pkg_visible_load(files: &[(String, String)]) -> bool {
    files.iter().any(|(_, content)| {
        let verbatim_lines = texparse::verbatim_body_lines(content);
        content
            .lines()
            .enumerate()
            .any(|(index, line)| !verbatim_lines.contains(&(index + 1)) && line_loads_color(line))
    })
}

fn line_loads_color(line: &str) -> bool {
    let line = texutil::strip_comment(line);
    ["\\usepackage", "\\RequirePackage"]
        .iter()
        .any(|cmd| scan_cmd_for_color(&line, cmd))
}

/// Scan one package-loading command for a visible `color`/`xcolor` load.
fn scan_cmd_for_color(line: &str, cmd: &str) -> bool {
    let mut rest = line;
    // Bounded: each pass either returns or resumes past the occurrence it
    // just looked at, so `line.len()` passes cover every candidate.
    for _ in 0..=line.len() {
        let Some(pos) = rest.find(cmd) else {
            break;
        };
        let after_cmd = &rest[pos + cmd.len()..];
        let Some(without_opts) = strip_optional_package_options(after_cmd) else {
            break;
        };
        let after_opts = without_opts.trim_start();
        let Some((loads_color, next)) = parse_braced_package_list(after_opts) else {
            rest = after_opts;
            continue;
        };
        if loads_color {
            return true;
        }
        rest = next;
    }
    false
}

/// Strip a `[...]` option list after `\usepackage`/`\RequirePackage`.
///
/// Returns the text after the options, or the input unchanged when there is
/// no option list. Returns `None` when `[` is never closed.
fn strip_optional_package_options(after: &str) -> Option<&str> {
    let Some(stripped) = after.strip_prefix('[') else {
        return Some(after);
    };
    let end = stripped.find(']')?;
    Some(&stripped[end + 1..])
}

/// Parse a `{pkg,...}` list (caller must have trimmed leading whitespace).
/// Returns whether it loads `color`/`xcolor` plus the text after `}`.
/// Returns `None` when there is no braced list to parse.
fn parse_braced_package_list(after: &str) -> Option<(bool, &str)> {
    let args = after.strip_prefix('{')?;
    let (list, next) = args.split_once('}')?;
    Some((package_list_loads_color(list), next))
}

/// Does a comma-separated package list load `color` or `xcolor`?
fn package_list_loads_color(list: &str) -> bool {
    list.split(',')
        .map(str::trim)
        .any(|name| name == "color" || name == "xcolor")
}

/// Insert the injected block into the entry file, right before
/// `\begin{document}`. Fails with a clear message (never a panic) when the
/// anchor is missing — such a document does not compile anyway. Returns
/// `(anchor_line, line_count)`: the 1-based line the anchor sits on, and the
/// number of lines the file had *before* injection. The caller needs both to
/// shift its line map — `line_count` so an entry file that had no code block
/// of its own (hence no recorded origins) still gets a full-length map, not
/// one truncated to the injected lines.
pub(crate) fn inject_entry(entry: &Path, block: &str) -> Result<(usize, usize)> {
    let content =
        std::fs::read_to_string(entry).with_context(|| format!("entry '{}'", entry.display()))?;
    let anchor = "\\begin{document}";
    let Some(pos) = content.find(anchor) else {
        anyhow::bail!(
            "entry '{}' has no \\begin{{document}} to anchor code-listing preamble",
            entry.display()
        );
    };
    let anchor_line = 1 + content[..pos].matches('\n').count();
    let line_count = content.lines().count();
    let mut out = String::with_capacity(content.len() + block.len());
    out.push_str(&content[..pos]);
    out.push_str(block);
    out.push_str(&content[pos..]);
    std::fs::write(entry, out).with_context(|| format!("entry '{}'", entry.display()))?;
    Ok((anchor_line, line_count))
}

/// Refuse to rewrite a document that already owns the `code` environment or
/// the reserved `\tfx` prefix (D14). Runs on the **original** build-copy
/// sources, comments stripped, before anything is written: our own injected
/// text is full of `\tfx` and must never be scanned (the T3 trap).
pub(crate) fn check_collisions(files: &[(String, String)]) -> Result<()> {
    for (file, content) in files {
        // Text inside a verbatim body is source the pass will escape, not a
        // macro the document defines: a `code` (or `lstlisting`) block that
        // *quotes* `\newenvironment{code}` — documenting this very feature —
        // must not collide with itself. The `\begin`/`\end` lines stay
        // visible; only whole body lines are skipped.
        let verbatim_lines = texparse::verbatim_body_lines(content);
        for (index, raw_line) in content.lines().enumerate() {
            let line = index + 1;
            if verbatim_lines.contains(&line) {
                continue;
            }
            let line_text = texutil::strip_comment(raw_line);
            check_code_definition(&line_text, file, line)?;
            check_tfx_prefix(&line_text, file, line)?;
        }
    }
    Ok(())
}

/// Fail when a comment-stripped line defines the `code` environment.
fn check_code_definition(line_text: &str, file: &str, line: usize) -> Result<()> {
    const DEFINITIONS: &[&str] = &[
        "\\newenvironment{code}",
        "\\renewenvironment{code}",
        "\\NewDocumentEnvironment{code}",
        "\\DeclareDocumentEnvironment{code}",
        "\\def\\code",
        "\\let\\code",
    ];

    for definition in DEFINITIONS {
        // Scan every occurrence, not just the first: a lookalike
        // (`\def\codefoo`) must not shield a real definition later
        // on the same line.
        let mut search = line_text;
        while let Some(pos) = search.find(definition) {
            let end = pos + definition.len();
            let next = search[end..].chars().next();
            if is_lookalike_definition(definition, next) {
                search = &search[end..];
                continue;
            }
            anyhow::bail!(
                "the 'code' environment is already defined in {file}:{line} — \
                 texforge rewrites \\begin{{code}} blocks itself and requires it to \
                 be free; rename your definition"
            );
        }
    }
    Ok(())
}

/// `\def\codefoo` does not define `code`: only `\def`/`\let` take a bare
/// control word, and only an immediately following letter makes it longer.
fn is_lookalike_definition(definition: &str, next: Option<char>) -> bool {
    (definition.starts_with("\\def") || definition.starts_with("\\let"))
        && next.is_some_and(|c| c.is_ascii_alphabetic())
}

/// Fail when a comment-stripped line uses the reserved `\tfx` prefix.
fn check_tfx_prefix(line_text: &str, file: &str, line: usize) -> Result<()> {
    let mut search = line_text;
    // Bounded: every pass resumes past the `\tfx` it just inspected, so
    // `line_text.len()` passes cover every occurrence on the line.
    for _ in 0..=line_text.len() {
        let Some(pos) = search.find("\\tfx") else {
            break;
        };
        let after = &search[pos + "\\tfx".len()..];
        if after
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic())
        {
            anyhow::bail!(
                "\\tfx-prefixed command found in {file}:{line} — the \\tfx prefix is \
                 reserved for texforge code highlighting; rename it"
            );
        }
        search = after;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(f, c)| (f.to_string(), c.to_string()))
            .collect()
    }

    #[test]
    fn block_defines_used_colors_sorted_and_nothing_else() {
        let mut used = BTreeSet::new();
        used.insert(Rgb::new(0x03, 0x2f, 0x62));
        used.insert(Rgb::new(0x00, 0x00, 0x00));
        let block = injected_block(&used, false, false, HighlightTheme::Github);
        let first = block.find("\\definecolor{tfxcol000000}").unwrap();
        let second = block.find("\\definecolor{tfxcol032f62}").unwrap();
        assert!(first < second, "colors must be sorted by name:\n{block}");
        assert!(!block.contains("tfxgutter"), "no gutter without numbering");
        assert!(block.contains(r"\usepackage{color}"));
        assert!(block.contains("\\newcommand{\\tfxcodestyle}"));
        assert!(block.contains("\\newcommand{\\tfxsp}{\\nobreakspace{}}"));
        assert!(block.starts_with(BEGIN_MARKER));
        assert!(block.contains(END_MARKER));
    }

    #[test]
    fn gutter_color_only_when_numbered() {
        let block = injected_block(&BTreeSet::new(), true, false, HighlightTheme::Github);
        assert!(block.contains("\\definecolor{tfxgutter}{rgb}{0.416,0.451,0.490}"));
        let one_light = injected_block(&BTreeSet::new(), true, false, HighlightTheme::OneLight);
        assert!(one_light.contains("\\definecolor{tfxgutter}{rgb}{0.627,0.631,0.655}"));
    }

    #[test]
    fn frame_colors_come_from_the_theme() {
        let block = injected_block(&BTreeSet::new(), false, false, HighlightTheme::Github);
        assert!(block.contains("\\definecolor{tfxtint}{rgb}{0.965,0.973,0.980}"));
        assert!(block.contains("\\definecolor{tfxframe}{rgb}{0.765,0.780,0.796}"));
        assert!(block.contains("\\newcommand{\\tfxsmash}"));
        let one_light = injected_block(&BTreeSet::new(), false, false, HighlightTheme::OneLight);
        assert!(one_light.contains("\\definecolor{tfxtint}{rgb}{0.980,0.980,0.980}"));
        assert!(one_light.contains("\\definecolor{tfxframe}{rgb}{0.851,0.851,0.863}"));
    }

    #[test]
    fn visible_color_load_drops_the_usepackage_but_keeps_the_guard() {
        let block = injected_block(&BTreeSet::new(), false, true, HighlightTheme::Github);
        assert!(!block.contains("\\usepackage{color}"), "{block}");
        assert!(block.contains(r"\@ifpackageloaded{color}"), "{block}");
        assert!(block.contains(r"\@ifpackageloaded{xcolor}"), "{block}");
    }

    #[test]
    fn color_pkg_detection_handles_options_lists_and_inputs() {
        assert!(color_pkg_visible_load(&files(&[(
            "main.tex",
            "\\documentclass{article}\n\\usepackage[dvipsnames]{xcolor}\n\\begin{document}"
        )])));
        assert!(color_pkg_visible_load(&files(&[(
            "main.tex",
            "\\usepackage{color,graphicx}"
        )])));
        assert!(color_pkg_visible_load(&files(&[(
            "preamble.tex",
            "\\RequirePackage{xcolor}"
        )])));
        // A commented-out load must not count.
        assert!(!color_pkg_visible_load(&files(&[(
            "main.tex",
            "% \\usepackage{xcolor}\n\\begin{document}"
        )])));
        assert!(!color_pkg_visible_load(&files(&[(
            "main.tex",
            "\\usepackage{graphicx}"
        )])));
    }

    #[test]
    fn color_pkg_load_quoted_in_code_is_not_a_real_load() {
        // A `\usepackage{xcolor}` shown as sample code is escaped text, not a
        // preamble load: it must not suppress the `\usepackage{color}` the
        // rewritten listing needs (otherwise `\definecolor` would be
        // undefined at runtime).
        assert!(!color_pkg_visible_load(&files(&[(
            "main.tex",
            "\\documentclass{article}\n\\begin{document}\n\
             \\begin{code}[lang=latex]\n\\usepackage{xcolor}\n\\end{code}\n\\end{document}",
        )])));
        // …but a real load alongside the example still counts.
        assert!(color_pkg_visible_load(&files(&[(
            "main.tex",
            "\\documentclass{article}\n\\usepackage{xcolor}\n\\begin{document}\n\
             \\begin{code}[lang=latex]\n\\usepackage{xcolor}\n\\end{code}\n\\end{document}",
        )])));
    }

    /// Every `\tfx` occurrence on a line is checked, wherever it sits, and
    /// only a real control word (`\tfx` + letter) is reserved.
    #[test]
    fn check_tfx_prefix_rejects_a_reserved_command_wherever_it_sits() {
        assert!(
            check_tfx_prefix("\\tfxuser", "main.tex", 1).is_err(),
            "at the very start of the line"
        );
        assert!(
            check_tfx_prefix("x = \\tfxother", "main.tex", 1).is_err(),
            "mid-line"
        );
        assert!(
            check_tfx_prefix("\\let\\tfx\\relax", "main.tex", 1).is_ok(),
            "`\\tfx` followed by a backslash is a control word boundary, not a use"
        );
        assert!(
            check_tfx_prefix("\\tfx is not a command", "main.tex", 1).is_ok(),
            "a space ends the control word"
        );
        assert!(check_tfx_prefix("no reserved prefix", "main.tex", 1).is_ok());
    }

    /// The scan resumes past each occurrence it inspected: a line carrying
    /// several package loads is not satisfied by the first one.
    #[test]
    fn color_detection_walks_every_package_load_on_the_line() {
        assert!(line_loads_color("\\usepackage{url} \\usepackage{color}"));
        assert!(!line_loads_color(
            "\\usepackage{url} \\usepackage{graphicx}"
        ));
    }

    #[test]
    fn collision_detection_names_file_and_line() {
        let err = check_collisions(&files(&[(
            "main.tex",
            "\\documentclass{article}\n\\newenvironment{code}[2]{a}{b}\n\\begin{document}",
        )]))
        .unwrap_err()
        .to_string();
        assert!(err.contains("main.tex:2"), "{err}");
        assert!(err.contains("already defined"), "{err}");
    }

    #[test]
    fn collision_detection_catches_the_tfx_prefix_but_not_comments() {
        let err = check_collisions(&files(&[(
            "main.tex",
            "% ok comment\n\\newcommand{\\tfxcode}{x}",
        )]))
        .unwrap_err()
        .to_string();
        assert!(err.contains("main.tex:2"), "{err}");
        assert!(err.contains("\\tfx"), "{err}");

        assert!(check_collisions(&files(&[("main.tex", "% \\tfx in a comment")])).is_ok());
    }

    #[test]
    fn collision_detection_ignores_lookalike_names() {
        assert!(
            check_collisions(&files(&[("main.tex", "\\def\\codebase{}\n\\def\\mytfx{}")])).is_ok()
        );
    }

    /// A lookalike on the same line (`\def\codefoo`, `\let\tfx\relax`) must
    /// not shield a real collision further along: every occurrence counts.
    #[test]
    fn collision_detection_catches_second_definition_on_a_line() {
        let err = check_collisions(&files(&[("main.tex", "\\def\\codebase{}\\def\\code{x}")]))
            .unwrap_err()
            .to_string();
        assert!(err.contains("main.tex:1"), "{err}");
        assert!(err.contains("already defined"), "{err}");

        let err = check_collisions(&files(&[(
            "main.tex",
            "\\let\\tfx\\relax\\let\\tfxcode\\relax",
        )]))
        .unwrap_err()
        .to_string();
        assert!(err.contains("main.tex:1"), "{err}");
        assert!(err.contains("\\tfx"), "{err}");
    }

    /// Definitions *quoted* inside a verbatim body are sample text the pass
    /// will escape — they define nothing, so they must not collide (nor may
    /// a `\tfx…` token inside a listing's own body).
    #[test]
    fn definitions_quoted_inside_a_verbatim_body_do_not_collide() {
        assert!(
            check_collisions(&files(&[(
                "main.tex",
                "\\begin{code}[lang=latex]\n\\newenvironment{code}{a}{b}\n\\end{code}",
            )]))
            .is_ok(),
            "a quoted \\newenvironment{{code}} inside a block is text"
        );
        assert!(
            check_collisions(&files(&[(
                "main.tex",
                "\\begin{lstlisting}\n\\newcommand{\\tfxmine}{x}\n\\end{lstlisting}",
            )]))
            .is_ok(),
            "a quoted \\tfx… inside a listing body is text"
        );
        // …but the same definitions outside a body still fail.
        let err = check_collisions(&files(&[(
            "main.tex",
            "\\begin{code}[lang=latex]\n\\end{code}\n\\newenvironment{code}{a}{b}",
        )]))
        .unwrap_err()
        .to_string();
        assert!(err.contains("already defined"), "{err}");
    }

    #[test]
    fn injection_anchors_before_begin_document() {
        let dir = tempfile::tempdir().unwrap();
        let entry = dir.path().join("main.tex");
        std::fs::write(&entry, "\\documentclass{article}\n\\begin{document}\nHi.\n").unwrap();
        // Anchor on line 2; the pre-injection file has three lines.
        let (anchor, lines) = inject_entry(&entry, "% injected\n").unwrap();
        assert_eq!((anchor, lines), (2, 3));
        let written = std::fs::read_to_string(&entry).unwrap();
        assert_eq!(
            written,
            "\\documentclass{article}\n% injected\n\\begin{document}\nHi.\n"
        );
    }

    #[test]
    fn injection_without_begin_document_is_a_clear_error() {
        let dir = tempfile::tempdir().unwrap();
        let entry = dir.path().join("main.tex");
        std::fs::write(&entry, "\\documentclass{article}\n").unwrap();
        let err = inject_entry(&entry, "% injected\n")
            .unwrap_err()
            .to_string();
        assert!(err.contains("no \\begin{document}"), "{err}");
        assert!(err.contains("main.tex"), "{err}");
    }

    #[test]
    fn caption_machinery_defines_counter_chapter_guard_list_and_names() {
        let names = caption::Names {
            listing: "Listing".to_string(),
            list: "List of Listings".to_string(),
        };
        let block = injected_block_with_caption(
            &BTreeSet::new(),
            false,
            Some(&names),
            false,
            HighlightTheme::Github,
        );
        assert!(block.contains("\\newcounter{tfxlisting}"), "{block}");
        assert!(block.contains("\\@ifundefined{chapter}"), "{block}");
        assert!(
            block.contains("\\@addtoreset{tfxlisting}{chapter}"),
            "{block}"
        );
        assert!(
            block.contains("\\renewcommand{\\thetfxlisting}{\\thechapter.\\arabic{tfxlisting}}"),
            "{block}"
        );
        assert!(
            block.contains("\\providecommand{\\listoflistings}"),
            "{block}"
        );
        assert!(block.contains("\\@starttoc{lol}"), "{block}");
        assert!(
            block.contains("\\newcommand{\\tfxlistingname}{Listing}"),
            "{block}"
        );
        assert!(
            block.contains("\\newcommand{\\tfxlistname}{List of Listings}"),
            "{block}"
        );
        assert!(block.contains("\\l@listing"), "{block}");
    }

    #[test]
    fn no_caption_machinery_without_a_caption() {
        let block = injected_block(&BTreeSet::new(), false, false, HighlightTheme::Github);
        assert!(!block.contains("tfxlisting"), "{block}");
        assert!(!block.contains("listoflistings"), "{block}");
        assert!(!block.contains("tfxlistingname"), "{block}");
    }
}
