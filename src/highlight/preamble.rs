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

use crate::texutil;

use super::engine::Rgb;

pub(crate) const BEGIN_MARKER: &str =
    "% ---- texforge code listings (injected; do not edit, rebuild to refresh) ----";
pub(crate) const END_MARKER: &str = "% ---- end texforge code listings ----";

/// The gutter colour, always under the fixed name `tfxgutter`.
const GUTTER: Rgb = Rgb::new(0x6e, 0x77, 0x81);

/// Build the injected preamble block.
///
/// * `colors` — every colour the rewritten blocks actually use, sorted
///   (a `BTreeSet`), so the block is deterministic;
/// * `has_gutter` — at least one numbered block exists;
/// * `color_pkg_visible_load` — the author already loads `color`/`xcolor`
///   themselves; the `\usepackage{color}` inside the guard is then dropped
///   (the guard itself stays, as defense in depth for classes such as beamer
///   that load `xcolor` behind the scan's back).
pub(crate) fn injected_block(
    colors: &BTreeSet<Rgb>,
    has_gutter: bool,
    color_pkg_visible_load: bool,
) -> String {
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
            GUTTER.to_rgb_list()
        ));
    }
    out.push_str(END_MARKER);
    out.push('\n');
    out
}

/// Does any of these sources visibly load `color` or `xcolor`?
///
/// Text scan on comment-stripped lines (so a commented-out `\usepackage`
/// does not count). The runtime `\@ifpackageloaded` guard is the correctness
/// mechanism; this scan only spares the author a redundant line.
pub(crate) fn color_pkg_visible_load(files: &[(String, String)]) -> bool {
    files
        .iter()
        .any(|(_, content)| content.lines().any(line_loads_color))
}

fn line_loads_color(line: &str) -> bool {
    let line = texutil::strip_comment(line);
    let mut rest = line.as_str();
    for cmd in ["\\usepackage", "\\RequirePackage"] {
        while let Some(pos) = rest.find(cmd) {
            let mut after = &rest[pos + cmd.len()..];
            // Optional argument list: \usepackage[dvipsnames]{xcolor}
            if let Some(stripped) = after.strip_prefix('[') {
                match stripped.find(']') {
                    Some(end) => after = &stripped[end + 1..],
                    None => break,
                }
            }
            after = after.trim_start();
            if let Some(args) = after.strip_prefix('{') {
                if let Some(end) = args.find('}') {
                    let loaded = args[..end]
                        .split(',')
                        .map(str::trim)
                        .any(|name| name == "color" || name == "xcolor");
                    if loaded {
                        return true;
                    }
                    rest = &args[end + 1..];
                    continue;
                }
            }
            rest = after;
        }
        rest = line.as_str();
    }
    false
}

/// Insert the injected block into the entry file, right before
/// `\begin{document}`. Fails with a clear message (never a panic) when the
/// anchor is missing — such a document does not compile anyway.
pub(crate) fn inject_entry(entry: &Path, block: &str) -> Result<()> {
    let content =
        std::fs::read_to_string(entry).with_context(|| format!("entry '{}'", entry.display()))?;
    let anchor = "\\begin{document}";
    let Some(pos) = content.find(anchor) else {
        anyhow::bail!(
            "entry '{}' has no \\begin{{document}} to anchor code-listing preamble",
            entry.display()
        );
    };
    let mut out = String::with_capacity(content.len() + block.len());
    out.push_str(&content[..pos]);
    out.push_str(block);
    out.push_str(&content[pos..]);
    std::fs::write(entry, out).with_context(|| format!("entry '{}'", entry.display()))?;
    Ok(())
}

/// Refuse to rewrite a document that already owns the `code` environment or
/// the reserved `\tfx` prefix (D14). Runs on the **original** build-copy
/// sources, comments stripped, before anything is written: our own injected
/// text is full of `\tfx` and must never be scanned (the T3 trap).
pub(crate) fn check_collisions(files: &[(String, String)]) -> Result<()> {
    const DEFINITIONS: &[&str] = &[
        "\\newenvironment{code}",
        "\\renewenvironment{code}",
        "\\NewDocumentEnvironment{code}",
        "\\DeclareDocumentEnvironment{code}",
        "\\def\\code",
        "\\let\\code",
    ];

    for (file, content) in files {
        for (index, raw_line) in content.lines().enumerate() {
            let line = index + 1;
            let line_text = texutil::strip_comment(raw_line);

            for definition in DEFINITIONS {
                if let Some(pos) = line_text.find(definition) {
                    let end = pos + definition.len();
                    let next = line_text[end..].chars().next();
                    // `\def\codefoo` does not define `code`.
                    if (definition.starts_with("\\def") || definition.starts_with("\\let"))
                        && next.is_some_and(|c| c.is_ascii_alphabetic())
                    {
                        continue;
                    }
                    anyhow::bail!(
                        "the 'code' environment is already defined in {file}:{line} — \
                         texforge rewrites \\begin{{code}} blocks itself and requires it to be \
                         free; rename your definition"
                    );
                }
            }

            if let Some(pos) = line_text.find("\\tfx") {
                if line_text[pos + 4..]
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_alphabetic())
                {
                    anyhow::bail!(
                        "\\tfx-prefixed command found in {file}:{line} — the \\tfx prefix is \
                         reserved for texforge code highlighting; rename it"
                    );
                }
            }
        }
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
        let block = injected_block(&used, false, false);
        let first = block.find("\\definecolor{tfxcol000000}").unwrap();
        let second = block.find("\\definecolor{tfxcol032f62}").unwrap();
        assert!(first < second, "colors must be sorted by name:\n{block}");
        assert!(!block.contains("tfxgutter"), "no gutter without numbering");
        assert!(block.contains(r"\usepackage{color}"));
        assert!(block.contains("\\newcommand{\\tfxcodestyle}"));
        assert!(block.starts_with(BEGIN_MARKER));
        assert!(block.contains(END_MARKER));
    }

    #[test]
    fn gutter_color_only_when_numbered() {
        let block = injected_block(&BTreeSet::new(), true, false);
        assert!(block.contains("\\definecolor{tfxgutter}{rgb}{0.431,0.467,0.506}"));
    }

    #[test]
    fn visible_color_load_drops_the_usepackage_but_keeps_the_guard() {
        let block = injected_block(&BTreeSet::new(), false, true);
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

    #[test]
    fn injection_anchors_before_begin_document() {
        let dir = tempfile::tempdir().unwrap();
        let entry = dir.path().join("main.tex");
        std::fs::write(&entry, "\\documentclass{article}\n\\begin{document}\nHi.\n").unwrap();
        inject_entry(&entry, "% injected\n").unwrap();
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
}
