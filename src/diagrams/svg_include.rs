//! Rewrite `\includegraphics` of SVG files to converted vector PDFs.
//!
//! The build already renders embedded diagrams to PDF via
//! [`super::convert_svg_or_fallback`]; this pass lets authors include their
//! own `.svg` artwork the same way, without converting it by hand. It runs
//! on the build copies only — the author's `.tex` files are never modified.
//!
//! Only files reachable from the entry through `\input` are visited (the
//! same walker the diagram pass uses); `\include` is not followed and is
//! out of scope. `\graphicspath` is not supported: paths resolve against
//! the project root, the same root [`crate::utils::mirror_assets`] mirrors.
//!
//! SVG text is rendered with the shared `svg2pdf` font database, exactly as
//! for diagram figures, and no font-family warning is emitted (nothing in
//! `svg2pdf` exposes which fallback face was used).

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};

/// Competing raster/vector formats that make a bare stem resolve away from
/// SVG: with `figs/a` and an `a.pdf` beside the `a.svg`, LaTeX would pick
/// the PDF itself, so the include is left untouched.
const COMPETING_EXTENSIONS: &[&str] = &["pdf", "png", "jpg", "jpeg", "eps"];

/// Rewrite every SVG `\includegraphics` in `text` to its converted artefact.
///
/// `root` is the project root the include paths resolve against,
/// `build_dir` receives the converted files under `svg-includes/`, and
/// `file` is the source `.tex` file holding the includes (used only for
/// the missing-file error). Comment text (after an unescaped `%`) is
/// ignored. The `[...]` options are passed through untouched; only the
/// `{path}` argument is rewritten, without an extension, so LaTeX picks
/// the converted format.
pub(super) fn rewrite_svg_includes(
    text: &str,
    root: &Path,
    build_dir: &Path,
    file: &Path,
) -> Result<String> {
    let mut result = String::with_capacity(text.len());
    for (index, line) in text.split('\n').enumerate() {
        if index > 0 {
            result.push('\n');
        }
        let code = crate::texutil::strip_comment(line);
        result.push_str(&rewrite_line(
            line,
            &code,
            index + 1,
            root,
            build_dir,
            file,
        )?);
    }
    Ok(result)
}

/// Rewrite the SVG includes on one line; `code` is the uncommented prefix
/// of `line`, so byte offsets into `code` are valid in `line`.
fn rewrite_line(
    line: &str,
    code: &str,
    lineno: usize,
    root: &Path,
    build_dir: &Path,
    file: &Path,
) -> Result<String> {
    const CMD: &str = "\\includegraphics";
    let mut out = String::with_capacity(line.len());
    let mut pos = 0usize;
    while let Some(found) = code[pos..].find(CMD) {
        let start = pos + found;
        let after_cmd = start + CMD.len();
        if next_is_command_char(&code[after_cmd..]) {
            out.push_str(&line[pos..after_cmd]);
            pos = after_cmd;
            continue;
        }
        let Some(occurrence) = parse_occurrence(code, after_cmd) else {
            out.push_str(&line[pos..after_cmd]);
            pos = after_cmd;
            continue;
        };
        let Some(svg_path) = resolve_svg(&occurrence.path, root) else {
            out.push_str(&line[pos..occurrence.close + 1]);
            pos = occurrence.close + 1;
            continue;
        };
        let artefact = convert_include(&occurrence.path, &svg_path, build_dir, file, lineno)?;
        out.push_str(&line[pos..occurrence.open + 1]);
        out.push_str(&artefact);
        pos = occurrence.close;
    }
    out.push_str(&line[pos..]);
    Ok(out)
}

/// A longer command name (`\includegraphicsx`) is not our command.
fn next_is_command_char(after: &str) -> bool {
    after
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphanumeric())
}

/// One `\includegraphics[opts]{path}` occurrence: byte offsets of the `{`
/// (`open`) and its `}` (`close`) in the scanned line, plus the trimmed path.
struct Occurrence {
    open: usize,
    close: usize,
    path: String,
}

/// Parse a single occurrence after the command name, skipping whitespace
/// and at most one `[...]` option group (which may itself contain `{...}`
/// groups, so `]` inside braces does not end it). Returns `None` when the
/// text after the command is not `[…]{…}` — the occurrence is left alone.
fn parse_occurrence(code: &str, after_cmd: usize) -> Option<Occurrence> {
    let bytes = code.as_bytes();
    let mut i = skip_whitespace(bytes, after_cmd);
    if bytes.get(i) == Some(&b'[') {
        i = skip_options(bytes, i)?;
        i = skip_whitespace(bytes, i);
    }
    if bytes.get(i) != Some(&b'{') {
        return None;
    }
    let open = i;
    let close = code[open..].find('}')? + open;
    let path = code[open + 1..close].trim().to_string();
    if path.is_empty() {
        return None;
    }
    Some(Occurrence { open, close, path })
}

/// Skip ASCII whitespace from `i` (byte offsets stay on char boundaries:
/// only ASCII bytes are consumed).
fn skip_whitespace(bytes: &[u8], mut i: usize) -> usize {
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    i
}

/// Skip one `[...]` group starting at `bytes[i] == b'['`, returning the
/// offset just past its closing `]`. `{...}` groups inside are skipped as
/// a whole; `None` means the group never closes.
fn skip_options(bytes: &[u8], i: usize) -> Option<usize> {
    let mut square = 0i32;
    let mut brace = 0i32;
    let mut j = i;
    while j < bytes.len() {
        match bytes[j] {
            b'{' => brace += 1,
            b'}' => brace -= 1,
            b'[' if brace == 0 => square += 1,
            b']' if brace == 0 => {
                square -= 1;
                if square == 0 {
                    return Some(j + 1);
                }
            }
            _ => {}
        }
        j += 1;
    }
    None
}

/// Resolve an include `path` to the SVG file to convert, or `None` when it
/// is not an SVG include. A path ending in `.svg` always counts (a missing
/// file is a build error, reported later); a bare stem counts only when
/// `<root>/<path>.svg` exists and none of the competing formats exists.
fn resolve_svg(path: &str, root: &Path) -> Option<PathBuf> {
    if path.ends_with(".svg") {
        return Some(root.join(path));
    }
    if Path::new(path).extension().is_some() {
        return None;
    }
    if !root.join(format!("{path}.svg")).exists() {
        return None;
    }
    let shadowed = COMPETING_EXTENSIONS
        .iter()
        .any(|ext| root.join(format!("{path}.{ext}")).exists());
    if shadowed {
        return None;
    }
    Some(root.join(format!("{path}.svg")))
}

/// Convert one SVG include, writing the artefact into
/// `<build_dir>/svg-includes/` (an existing file is reused, so the same
/// SVG referenced twice converts once). Returns the rewritten `{path}`
/// argument: `svg-includes/<stem>-<hash8>`, without an extension, so
/// LaTeX picks the converted format. `file`/`lineno` name the source
/// location when the SVG is missing.
fn convert_include(
    path: &str,
    svg_path: &Path,
    build_dir: &Path,
    file: &Path,
    lineno: usize,
) -> Result<String> {
    if !svg_path.exists() {
        anyhow::bail!("{}:{lineno}: SVG not found: {path}", file.display());
    }
    let bytes = std::fs::read(svg_path)
        .with_context(|| format!("Failed to read {}", svg_path.display()))?;
    let svg = String::from_utf8(bytes.clone())
        .with_context(|| format!("{} is not valid UTF-8", svg_path.display()))?;
    let full_hash = format!("{:x}", Sha256::digest(&bytes));
    let short = &full_hash[..8];
    let stem = Path::new(path)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "figure".to_string());
    // Name the warning after the SVG file itself, as for diagrams.
    let svg_name = if path.ends_with(".svg") {
        path.to_string()
    } else {
        format!("{path}.svg")
    };
    let (converted, ext) = super::convert_svg_or_fallback(&svg_name, &svg)?;
    let dir = build_dir.join("svg-includes");
    std::fs::create_dir_all(&dir)?;
    let dest = dir.join(format!("{stem}-{short}.{ext}"));
    if !dest.exists() {
        std::fs::write(&dest, &converted)?;
    }
    Ok(format!("svg-includes/{stem}-{short}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Small SVG with a shape and a text node, mirroring real artwork.
    const SIMPLE_SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="50"><rect x="5" y="5" width="40" height="20" fill="red"/><text x="50" y="30">HelloSvg</text></svg>"#;

    /// Builds an SVG whose group nesting is deep enough to exceed svg2pdf's
    /// PDF content-stream nesting guard, while remaining well within what
    /// resvg's rasterizer renders without issue. Copied from the diagram
    /// fallback test (`super::tests::deeply_nested_svg`): arbitrary garbage
    /// fails both parsers, so only this shape exercises the PNG fallback.
    fn deeply_nested_svg() -> String {
        let mut svg =
            String::from(r#"<svg xmlns="http://www.w3.org/2000/svg" width="80" height="80">"#);
        let depth = 35;
        for i in 0..depth {
            svg.push_str(&format!(
                r#"<g transform="translate(0.1,0.1)"><rect x="{i}" y="0" width="1" height="1" fill="blue"/>"#
            ));
        }
        svg.push_str(r#"<rect width="10" height="10" fill="red"/>"#);
        for _ in 0..depth {
            svg.push_str("</g>");
        }
        svg.push_str("</svg>");
        svg
    }

    fn write_svg(root: &Path, rel: &str) -> Vec<u8> {
        let path = root.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&path, SIMPLE_SVG).unwrap();
        SIMPLE_SVG.as_bytes().to_vec()
    }

    fn expected_ref(bytes: &[u8], stem: &str) -> String {
        let full = format!("{:x}", Sha256::digest(bytes));
        format!("svg-includes/{stem}-{}", &full[..8])
    }

    fn rewrite(root: &Path, build: &Path, file: &str, text: &str) -> String {
        rewrite_svg_includes(text, root, build, Path::new(file)).unwrap()
    }

    #[test]
    fn rewrites_svg_with_extension_keeping_options() {
        let root = tempfile::tempdir().unwrap();
        let bytes = write_svg(root.path(), "figs/a.svg");
        let build = tempfile::tempdir().unwrap();
        let out = rewrite(
            root.path(),
            build.path(),
            "main.tex",
            r"\includegraphics[width=\linewidth]{figs/a.svg}",
        );
        let want = expected_ref(&bytes, "a");
        assert_eq!(
            out,
            format!("\\includegraphics[width=\\linewidth]{{{want}}}")
        );
        assert!(!out.contains(".svg"));
    }

    #[test]
    fn bare_stem_with_only_svg_resolves_to_svg() {
        let root = tempfile::tempdir().unwrap();
        let bytes = write_svg(root.path(), "figs/a.svg");
        let build = tempfile::tempdir().unwrap();
        let out = rewrite(
            root.path(),
            build.path(),
            "main.tex",
            r"\includegraphics{figs/a}",
        );
        assert_eq!(
            out,
            format!("\\includegraphics{{{}}}", expected_ref(&bytes, "a"))
        );
    }

    #[test]
    fn bare_stem_with_pdf_beside_svg_is_left_alone() {
        let root = tempfile::tempdir().unwrap();
        write_svg(root.path(), "figs/a.svg");
        std::fs::write(root.path().join("figs/a.pdf"), b"%PDF-dummy").unwrap();
        let build = tempfile::tempdir().unwrap();
        let input = r"\includegraphics{figs/a}";
        assert_eq!(rewrite(root.path(), build.path(), "main.tex", input), input);
    }

    #[test]
    fn png_extension_is_left_alone() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("figs")).unwrap();
        std::fs::write(root.path().join("figs/a.png"), b"png-dummy").unwrap();
        let build = tempfile::tempdir().unwrap();
        let input = r"\includegraphics[width=3cm]{figs/a.png}";
        assert_eq!(rewrite(root.path(), build.path(), "main.tex", input), input);
    }

    #[test]
    fn commented_include_is_left_alone() {
        let root = tempfile::tempdir().unwrap();
        let build = tempfile::tempdir().unwrap();
        // `x.svg` does not even exist: a missed comment strip would fail
        // the build with "SVG not found" instead of passing through.
        let input = r"% \includegraphics{x.svg}";
        assert_eq!(rewrite(root.path(), build.path(), "main.tex", input), input);
    }

    #[test]
    fn two_occurrences_on_one_line_both_rewrite() {
        let root = tempfile::tempdir().unwrap();
        let bytes_a = write_svg(root.path(), "figs/a.svg");
        let bytes_b = write_svg(root.path(), "figs/b.svg");
        let build = tempfile::tempdir().unwrap();
        let out = rewrite(
            root.path(),
            build.path(),
            "main.tex",
            r"\includegraphics{figs/a.svg} and \includegraphics{figs/b.svg}",
        );
        assert_eq!(
            out,
            format!(
                "\\includegraphics{{{}}} and \\includegraphics{{{}}}",
                expected_ref(&bytes_a, "a"),
                expected_ref(&bytes_b, "b")
            )
        );
    }

    #[test]
    fn option_group_with_braces_survives() {
        let root = tempfile::tempdir().unwrap();
        let bytes = write_svg(root.path(), "figs/a.svg");
        let build = tempfile::tempdir().unwrap();
        let out = rewrite(
            root.path(),
            build.path(),
            "main.tex",
            r"\includegraphics[width={0.5\linewidth},height=3cm]{figs/a.svg}",
        );
        let want = expected_ref(&bytes, "a");
        assert_eq!(
            out,
            format!("\\includegraphics[width={{0.5\\linewidth}},height=3cm]{{{want}}}")
        );
    }

    #[test]
    fn conversion_returns_vector_pdf_for_text_svg() {
        let (bytes, ext) = super::super::convert_svg_or_fallback("test", SIMPLE_SVG).unwrap();
        assert_eq!(ext, "pdf");
        assert!(bytes.starts_with(b"%PDF-"), "expected PDF magic bytes");
        let text = String::from_utf8_lossy(&bytes);
        assert!(!text.contains("/Subtype /Image"), "raster XObject found");
        assert!(!text.contains("/Subtype/Image"), "raster XObject found");
    }

    #[test]
    fn deeply_nested_svg_falls_back_to_png() {
        let svg = deeply_nested_svg();
        let (bytes, ext) = super::super::convert_svg_or_fallback("test", &svg).unwrap();
        assert_eq!(ext, "png");
        assert!(
            bytes.starts_with(&[137, 80, 78, 71, 13, 10, 26, 10]),
            "expected PNG signature"
        );
    }

    #[test]
    fn missing_svg_error_names_file_and_line() {
        let root = tempfile::tempdir().unwrap();
        let build = tempfile::tempdir().unwrap();
        let err = rewrite_svg_includes(
            "hello\n\\includegraphics{figs/missing.svg}\n",
            root.path(),
            build.path(),
            Path::new("sec.tex"),
        )
        .unwrap_err();
        assert_eq!(
            err.to_string(),
            "sec.tex:2: SVG not found: figs/missing.svg"
        );
    }

    #[test]
    fn process_rewrites_includes_in_input_reachable_files() {
        let root = tempfile::tempdir().unwrap();
        let bytes = write_svg(root.path(), "figs/a.svg");
        let main = "\\documentclass{article}\n\\begin{document}\n\\input{sec}\n\\end{document}\n";
        let sec = "See \\includegraphics[width=\\linewidth]{figs/a.svg}.\n";
        std::fs::write(root.path().join("main.tex"), main).unwrap();
        std::fs::write(root.path().join("sec.tex"), sec).unwrap();
        let build = tempfile::tempdir().unwrap();
        let entry = crate::diagrams::process(
            root.path(),
            "main.tex",
            build.path(),
            crate::diagrams::style::DiagramStyle::Default,
        )
        .unwrap();
        assert_eq!(entry, build.path().join("main.tex"));
        let want = expected_ref(&bytes, "a");
        let sec_build = std::fs::read_to_string(build.path().join("sec.tex")).unwrap();
        assert!(
            sec_build.contains(&format!("{{{want}}}")),
            "build copy must reference the artefact: {sec_build}"
        );
        assert!(
            build.path().join(format!("{want}.pdf")).exists(),
            "converted PDF must exist in svg-includes/"
        );
        assert_eq!(
            std::fs::read(root.path().join("sec.tex")).unwrap(),
            sec.as_bytes(),
            "the author's source must stay byte-identical"
        );
    }

    #[test]
    fn svg_include_builds_to_selectable_text() {
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
        let root = tempfile::tempdir().unwrap();
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="100"><rect x="10" y="10" width="50" height="30" fill="blue"/><text x="70" y="50" font-size="20">SvgVectorText42</text></svg>"#;
        std::fs::create_dir_all(root.path().join("figs")).unwrap();
        std::fs::write(root.path().join("figs/vec.svg"), svg).unwrap();
        std::fs::write(
            root.path().join("main.tex"),
            "\\documentclass{article}\n\\usepackage{graphicx}\n\\begin{document}\n\\input{sec}\n\\end{document}\n",
        )
        .unwrap();
        std::fs::write(
            root.path().join("sec.tex"),
            "Figure: \\includegraphics[width=0.8\\linewidth]{figs/vec.svg}\n",
        )
        .unwrap();
        let build = tempfile::tempdir().unwrap();
        crate::diagrams::process(
            root.path(),
            "main.tex",
            build.path(),
            crate::diagrams::style::DiagramStyle::Default,
        )
        .unwrap();
        crate::compiler::compile(
            build.path(),
            "main.tex",
            false,
            None,
            &crate::highlight::LineMap::default(),
        )
        .unwrap();
        let text = crate::pdftext::extract_text(&build.path().join("main.pdf")).unwrap();
        // The extractor letter-spaces some embedded fonts, so compare
        // without whitespace: a raster fallback would yield no text at all.
        let flat: String = text.split_whitespace().collect();
        assert!(
            flat.contains("SvgVectorText42"),
            "SVG text must stay selectable in the built PDF: {text:?}"
        );
    }
}
