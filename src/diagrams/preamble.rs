//! Preamble package detection and insertion for the diagram pass.
//!
//! The rewrite emits `\includegraphics` (needs `graphicx`) and, for a
//! diagram written with `pos=H`, `\begin{figure}[H]` (needs `float`). A
//! document whose preamble loads neither fails with `Undefined control
//! sequence` pointing at the diagram *the author wrote* — texforge
//! introduced the dependency, so texforge satisfies it, by editing the
//! **temporary build copy only**: the author's sources are never touched.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::texparse;
use crate::texutil;

/// Packages the rewrite can introduce, in insertion order.
const GRAPHICX: &str = "graphicx";
const FLOAT: &str = "float";

/// Which `\usepackage` lines the temp entry still needs.
///
/// `uses_float_h` reports whether a replaced diagram's figure was emitted
/// with `[H]` (an explicit `pos=H`); the caller gates on "at least one
/// diagram was replaced" before calling this at all. Detection reads the
/// *original* sources at `root` (never written): the entry's text before
/// `\begin{document}` plus, transitively, the preambles of the files it
/// `\input`s / `\include`s.
pub(super) fn packages_to_insert(
    root: &Path,
    entry: &str,
    uses_float_h: bool,
) -> Vec<&'static str> {
    let files = gather_preamble(root, entry);
    let mut pkgs = Vec::new();
    if !loads_package(&files, GRAPHICX) {
        pkgs.push(GRAPHICX);
    }
    if uses_float_h && !loads_package(&files, FLOAT) {
        pkgs.push(FLOAT);
    }
    pkgs
}

/// Entry preamble + transitively `\input`/`\include`d preamble files, each
/// cut at its own first `\begin{document}`. Unreadable files are skipped (a
/// missing file is not a reason to fail the build) and visited paths are
/// guarded against `\input` cycles (a preamble that points back at the entry
/// must not hang the build).
fn gather_preamble(root: &Path, entry: &str) -> Vec<String> {
    let mut visited: Vec<PathBuf> = Vec::new();
    let mut queue = vec![entry.to_string()];
    let mut out = Vec::new();

    while let Some(input) = queue.pop() {
        let path = texutil::resolve_tex_path(root, &input);
        if visited.contains(&path) || !path.exists() {
            continue;
        }
        visited.push(path.clone());
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        let preamble = up_to_begin_document(&content);
        queue.extend(preamble_targets(preamble));
        out.push(preamble.to_string());
    }
    out
}

/// Text before the first `\begin{document}`, else the whole content (a
/// preamble file may have no `\begin{document}` of its own).
fn up_to_begin_document(content: &str) -> &str {
    content
        .find("\\begin{document}")
        .map_or(content, |pos| &content[..pos])
}

/// `\input{…}` / `\include{…}` targets on comment-stripped preamble lines.
fn preamble_targets(preamble: &str) -> Vec<String> {
    let mut targets = Vec::new();
    for raw in preamble.lines() {
        let line = texutil::strip_comment(raw);
        for cmd in ["input", "include"] {
            for target in texutil::extract_commands(&line, cmd) {
                targets.push(target.to_string());
            }
        }
    }
    targets
}

/// Does any collected preamble text visibly load `pkg`?
///
/// Comment-stripped and verbatim-body-aware (the same rules the highlight
/// pass uses for `color`/`xcolor`): a commented-out load or a
/// `\usepackage{graphicx}` quoted as sample code must not suppress the
/// insertion the rewrite needs, while a real load must be recognised (a
/// duplicate load risks an option clash).
fn loads_package(files: &[String], pkg: &str) -> bool {
    files.iter().any(|content| {
        let verbatim_lines = texparse::verbatim_body_lines(content);
        content.lines().enumerate().any(|(index, line)| {
            !verbatim_lines.contains(&(index + 1)) && line_loads_package(line, pkg)
        })
    })
}

/// Does one raw line visibly load `pkg`?
fn line_loads_package(line: &str, pkg: &str) -> bool {
    let line = texutil::strip_comment(line);
    ["\\usepackage", "\\RequirePackage"]
        .iter()
        .any(|cmd| scan_cmd_for_package(&line, cmd, pkg))
}

/// Scan one package-loading command for a visible load of `pkg`.
fn scan_cmd_for_package(line: &str, cmd: &str, pkg: &str) -> bool {
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
        let Some((list, next)) = parse_braced_package_list(after_opts) else {
            rest = after_opts;
            continue;
        };
        if package_list_names(list, pkg) {
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

/// Parse a `{pkg,…}` list (caller must have trimmed leading whitespace).
/// Returns the list plus the text after `}`, or `None` when there is no
/// braced list to parse.
fn parse_braced_package_list(after: &str) -> Option<(&str, &str)> {
    let args = after.strip_prefix('{')?;
    let (list, next) = args.split_once('}')?;
    Some((list, next))
}

/// Does a comma-separated package list name `pkg`?
fn package_list_names(list: &str, pkg: &str) -> bool {
    list.split(',').map(str::trim).any(|name| name == pkg)
}

/// Insert `pkgs` as one `\usepackage{…}` line each, at the start of the
/// line containing `\begin{document}` of `entry_path` — the build copy,
/// never the author's source. Missing anchor → no-op: such a document does
/// not compile anyway, and there is nothing to gain from inventing a new
/// failure mode (the caller only reaches this when a diagram was replaced).
pub(super) fn insert_packages(entry_path: &Path, pkgs: &[&str]) -> Result<()> {
    if pkgs.is_empty() {
        return Ok(());
    }
    let content = std::fs::read_to_string(entry_path)
        .with_context(|| format!("entry '{}'", entry_path.display()))?;
    let Some(pos) = content.find("\\begin{document}") else {
        return Ok(());
    };
    // Insert at the start of the anchor's line, never at byte `pos` — the
    // anchor may sit mid-line and splicing there would corrupt that line.
    let line_start = content[..pos].rfind('\n').map_or(0, |nl| nl + 1);
    let mut out = String::with_capacity(content.len() + pkgs.len() * 24);
    out.push_str(&content[..line_start]);
    for pkg in pkgs {
        out.push_str(&format!("\\usepackage{{{pkg}}}\n"));
    }
    out.push_str(&content[line_start..]);
    std::fs::write(entry_path, out).with_context(|| format!("entry '{}'", entry_path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_package_detects_every_supported_spelling() {
        let one = |content: &str, pkg: &str| loads_package(&[content.to_string()], pkg);
        assert!(one(
            "\\documentclass{article}\n\\usepackage{graphicx}",
            "graphicx"
        ));
        assert!(one("\\usepackage[draft]{graphicx}", "graphicx"));
        assert!(one("\\usepackage{amsmath,graphicx}", "graphicx"));
        assert!(one("\\RequirePackage{float}", "float"));
        assert!(one("\\usepackage{url} \\usepackage{graphicx}", "graphicx"));
        // A comma list must not be mistaken for a load of another package.
        assert!(!one("\\usepackage{amsmath,xcolor}", "graphicx"));
    }

    #[test]
    fn loads_package_ignores_comments_verbatim_and_other_packages() {
        // A commented-out load is not a load.
        assert!(!loads_package(
            &["% \\usepackage{graphicx}".to_string()],
            "graphicx"
        ));
        // Another package is not this package.
        assert!(!loads_package(
            &["\\usepackage{xcolor}".to_string()],
            "graphicx"
        ));
        // A load quoted inside a verbatim body is sample text, not a load.
        let quoted = "\\documentclass{article}\n\\begin{document}\n\\begin{code}\n\\usepackage{graphicx}\n\\end{code}";
        assert!(!loads_package(&[quoted.to_string()], "graphicx"));
        // …while a real load alongside the example still counts.
        let real = "\\usepackage{graphicx}\n\\begin{document}\n\\begin{code}\n\\usepackage{graphicx}\n\\end{code}";
        assert!(loads_package(&[real.to_string()], "graphicx"));
    }

    #[test]
    fn preamble_stops_at_begin_document() {
        let content = "\\documentclass{article}\n\\begin{document}\n\\usepackage{graphicx}\n";
        let preamble = up_to_begin_document(content);
        assert!(!loads_package(&[preamble.to_string()], "graphicx"));
        // …and the preamble proper still counts.
        let content = "\\documentclass{article}\n\\usepackage{graphicx}\n\\begin{document}\n";
        let preamble = up_to_begin_document(content);
        assert!(loads_package(&[preamble.to_string()], "graphicx"));
    }

    #[test]
    fn preamble_follows_input_and_include_recursively_and_survives_cycles() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(
            root.join("main.tex"),
            "\\documentclass{article}\n\\input{pre}\n\\include{other}\n\\begin{document}\n",
        )
        .unwrap();
        // `pre.tex` points back at the entry: no hang, no duplicate work.
        std::fs::write(
            root.join("pre.tex"),
            "\\usepackage{amsmath}\n\\input{nested}\n\\input{main}\n",
        )
        .unwrap();
        std::fs::write(root.join("nested.tex"), "\\usepackage{graphicx}\n").unwrap();
        std::fs::write(root.join("other.tex"), "\\RequirePackage{float}\n").unwrap();

        // graphicx via the \input chain, float via \include → nothing needed.
        assert!(packages_to_insert(root, "main.tex", true).is_empty());

        // Drop graphicx from the chain → it must be reported (first).
        std::fs::write(root.join("nested.tex"), "\\usepackage{amsmath}\n").unwrap();
        assert_eq!(packages_to_insert(root, "main.tex", false), vec![GRAPHICX]);

        // Drop float from the \include file too → both, in insertion order.
        std::fs::write(root.join("other.tex"), "% nothing\n").unwrap();
        assert_eq!(
            packages_to_insert(root, "main.tex", true),
            vec![GRAPHICX, FLOAT]
        );
        // Without `pos=H` there is no float requirement at all.
        assert_eq!(packages_to_insert(root, "main.tex", false), vec![GRAPHICX]);
    }

    #[test]
    fn insert_packages_places_lines_immediately_before_the_anchor() {
        let dir = tempfile::tempdir().unwrap();
        let entry = dir.path().join("main.tex");
        std::fs::write(
            &entry,
            "\\documentclass{article}\n\\usepackage{booktabs}\n\\begin{document}\nHi.\n",
        )
        .unwrap();
        insert_packages(&entry, &[GRAPHICX]).unwrap();
        assert_eq!(
            std::fs::read_to_string(&entry).unwrap(),
            "\\documentclass{article}\n\\usepackage{booktabs}\n\\usepackage{graphicx}\n\\begin{document}\nHi.\n"
        );

        // Two packages → two lines, graphicx first.
        let entry = dir.path().join("two.tex");
        std::fs::write(&entry, "preamble\n\\begin{document}\nbody").unwrap();
        insert_packages(&entry, &[GRAPHICX, FLOAT]).unwrap();
        assert_eq!(
            std::fs::read_to_string(&entry).unwrap(),
            "preamble\n\\usepackage{graphicx}\n\\usepackage{float}\n\\begin{document}\nbody"
        );

        // An anchor that does not start its line: insert at the line start,
        // never at the anchor's byte offset (that would split the line).
        let entry = dir.path().join("midline.tex");
        std::fs::write(
            &entry,
            "\\documentclass{article}\npre \\begin{document}\nbody",
        )
        .unwrap();
        insert_packages(&entry, &[GRAPHICX]).unwrap();
        assert_eq!(
            std::fs::read_to_string(&entry).unwrap(),
            "\\documentclass{article}\n\\usepackage{graphicx}\npre \\begin{document}\nbody"
        );
    }

    #[test]
    fn insert_packages_is_a_noop_without_an_anchor() {
        let dir = tempfile::tempdir().unwrap();
        let entry = dir.path().join("main.tex");
        let original = "\\documentclass{article}\n";
        std::fs::write(&entry, original).unwrap();
        insert_packages(&entry, &[GRAPHICX]).unwrap();
        assert_eq!(std::fs::read_to_string(&entry).unwrap(), original);
    }
}
