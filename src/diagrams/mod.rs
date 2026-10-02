//! Pre-processor for embedded diagram environments.
//!
//! Intercepts `\begin{mermaid}[opts]...\end{mermaid}` blocks, renders them
//! to vector PDF (falling back to PNG if the PDF conversion fails), and
//! replaces each block with a proper `figure` environment.
//!
//! Works on copies in `build/` — the original .tex files are never modified.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::texutil;

mod fonts;
mod preamble;
pub mod style;
use fonts::{shared_fontdb, shared_svg2pdf_fontdb};
use style::DiagramStyle;

/// Copy all .tex files to `build_dir`, rendering embedded diagrams in the copies.
/// Also mirrors non-.tex assets so tectonic can resolve relative paths.
/// `default_style` is the document-wide style (`project.toml`'s `[diagrams]
/// style`, or `DiagramStyle::Default`); a `style=` on the environment itself
/// overrides it. Returns the path to the build copy of `entry`.
pub fn process(
    root: &Path,
    entry: &str,
    build_dir: &Path,
    default_style: DiagramStyle,
) -> Result<PathBuf> {
    std::fs::create_dir_all(build_dir)?;

    let diagrams_dir = build_dir.join("diagrams");
    std::fs::create_dir_all(&diagrams_dir)?;

    // Process .tex files
    let tex_files = collect_tex_files(root, entry);
    let mut replaced_any = false;
    let mut uses_float_h = false;
    for src in &tex_files {
        let rel = src.strip_prefix(root).unwrap_or(src);
        let dest = build_dir.join(rel);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = std::fs::read_to_string(src)?;
        let (processed, file_uses_h) =
            render_diagrams_facts(&content, &diagrams_dir, default_style)
                .with_context(|| format!("Failed to render diagrams in {}", src.display()))?;
        // Byte-compare: a replacement always changes the bytes (a
        // `\begin{mermaid}` tag can never equal a `figure` environment),
        // and a file without diagram blocks comes through untouched.
        replaced_any |= processed != content;
        uses_float_h |= file_uses_h;
        std::fs::write(&dest, processed)?;
    }

    // Mirror asset files so tectonic resolves relative paths
    crate::utils::mirror_assets(root, build_dir)?;

    // The rewrite introduces `\includegraphics` (and `[H]` when a diagram
    // opted into `pos=H`): if the author's preamble doesn't load the packages
    // those need, add them to the build copy of the entry — the original
    // sources are never written.
    if replaced_any {
        let pkgs = preamble::packages_to_insert(root, entry, uses_float_h);
        if !pkgs.is_empty() {
            preamble::insert_packages(&build_dir.join(entry), &pkgs)?;
        }
    }

    Ok(build_dir.join(entry))
}

/// Replace all `\begin{mermaid}[opts]...\end{mermaid}` with figure environments.
///
/// Test-only view of [`render_diagrams_facts`] that discards the `float`
/// requirement report; production code keeps the report so `process` can
/// satisfy the packages the rewrite introduced.
#[cfg(test)]
fn render_diagrams(
    content: &str,
    diagrams_dir: &Path,
    default_style: DiagramStyle,
) -> Result<String> {
    render_diagrams_facts(content, diagrams_dir, default_style).map(|(text, _)| text)
}

/// Same as the (test-only) `render_diagrams`, also reporting whether any
/// replaced figure was emitted with `[H]` — i.e. whether the rewritten file
/// will need the `float` package. The three environments are chained and the
/// report OR-ed.
fn render_diagrams_facts(
    content: &str,
    diagrams_dir: &Path,
    default_style: DiagramStyle,
) -> Result<(String, bool)> {
    let (content, mermaid_h) = render_env_facts(
        content,
        "mermaid",
        diagrams_dir,
        default_style,
        |src, sty| {
            let svg = render_mermaid_with_config(src, sty)?;
            convert_svg_or_fallback("mermaid", &svg)
        },
    )?;
    let (content, graphviz_h) = render_env_facts(
        &content,
        "graphviz",
        diagrams_dir,
        default_style,
        |src, sty| {
            let svg = render_graphviz(src, sty)?;
            convert_svg_or_fallback("graphviz", &svg)
        },
    )?;
    let (content, d2_h) =
        render_env_facts(&content, "d2", diagrams_dir, default_style, |src, sty| {
            let svg = render_d2(src, sty)?;
            convert_svg_or_fallback("d2", &svg)
        })?;
    Ok((content, mermaid_h || graphviz_h || d2_h))
}

/// Render a Mermaid diagram, applying `style`'s theme and layout spacing.
fn render_mermaid_with_config(src: &str, sty: DiagramStyle) -> Result<String> {
    mermaid_rs_renderer::render_with_options(src, style::mermaid_options(sty))
        .map_err(|e| anyhow::anyhow!("Mermaid render error: {}", e))
}

/// Generic environment renderer: replaces `\begin{env}[opts]...\end{env}` with figure.
///
/// Test-only view of [`render_env_facts`] that discards the `float`
/// requirement report (existing tests only care about the rewritten text).
#[cfg(test)]
pub(crate) fn render_env(
    content: &str,
    env: &str,
    diagrams_dir: &Path,
    default_style: DiagramStyle,
    render_fn: impl Fn(&str, DiagramStyle) -> Result<(Vec<u8>, &'static str)>,
) -> Result<String> {
    render_env_facts(content, env, diagrams_dir, default_style, render_fn).map(|(text, _)| text)
}

/// Generic environment renderer that also reports whether any replaced
/// figure was emitted with `[H]` (an explicit `pos=H`) — i.e. whether the
/// rewritten text needs the `float` package.
///
/// Rendered artefacts are named after a hash of the diagram source, so
/// unchanged diagrams are reused across rebuilds (watch mode) instead of
/// re-rendered. `render_fn` returns the encoded bytes and the extension
/// (`"pdf"` on the vector path, `"png"` when it fell back to rasterizing).
fn render_env_facts(
    content: &str,
    env: &str,
    diagrams_dir: &Path,
    default_style: DiagramStyle,
    render_fn: impl Fn(&str, DiagramStyle) -> Result<(Vec<u8>, &'static str)>,
) -> Result<(String, bool)> {
    let begin_tag = format!("\\begin{{{}}}", env);
    let end_tag = format!("\\end{{{}}}", env);

    let mut result = String::new();
    let mut remaining: &str = content;
    let mut uses_float_h = false;

    while let Some(start) = remaining.find(&begin_tag) {
        result.push_str(&remaining[..start]);

        let after_begin = &remaining[start + begin_tag.len()..];
        let (opts, after_opts) = parse_opts(after_begin, env)?;

        let end = find_end_tag(after_opts, &end_tag, env)?;
        let diagram_src = after_opts[..end].trim();

        validate_pos_option(&opts, env)?;
        // Only an explicit `pos=H` reaches `[H]` in the figure: an omitted
        // `pos` emits a plain `\begin{figure}` (build_figure_default_no_pos),
        // so it needs no `float`.
        uses_float_h |= opts.get("pos").map(String::as_str) == Some("H");
        let diagram_style = resolve_style(&opts, env, default_style)?;

        let base = format!("{}-{:016x}", env, content_hash(diagram_src, diagram_style));
        let filename = match cached_filename(diagrams_dir, &base) {
            Some(filename) => filename,
            None => {
                let (bytes, ext) = render_fn(diagram_src, diagram_style)?;
                let filename = format!("{base}.{ext}");
                std::fs::write(diagrams_dir.join(&filename), bytes)?;
                filename
            }
        };
        let fig_env = build_figure_environment(&opts, env, &filename)?;

        result.push_str(&fig_env);
        remaining = &after_opts[end + end_tag.len()..];
    }

    result.push_str(remaining);
    Ok((result, uses_float_h))
}

/// Look for an already-rendered artefact for `base`, vector form first.
fn cached_filename(diagrams_dir: &Path, base: &str) -> Option<String> {
    ["pdf", "png"].into_iter().find_map(|ext| {
        let filename = format!("{base}.{ext}");
        diagrams_dir.join(&filename).exists().then_some(filename)
    })
}

/// Stable-enough 64-bit hash of diagram source and style for cache filenames.
/// Folding the style in means changing `style=` re-renders instead of
/// serving a stale cached artefact.
fn content_hash(src: &str, style: DiagramStyle) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    src.hash(&mut hasher);
    (style as u8).hash(&mut hasher);
    hasher.finish()
}

/// Resolve the style for one diagram: the environment's `style=` option
/// wins, then the `project.toml` default. An unknown name is an error
/// naming the offending value and the valid alternatives.
fn resolve_style(
    opts: &HashMap<String, String>,
    env: &str,
    default_style: DiagramStyle,
) -> Result<DiagramStyle> {
    match opts.get("style") {
        None => Ok(default_style),
        Some(name) => DiagramStyle::parse(name).map_err(|_| {
            anyhow::anyhow!(
                "Invalid {} option style='{}' — valid values are: {}",
                env,
                name,
                style::VALID_STYLE_NAMES.join(", ")
            )
        }),
    }
}

/// Find the end tag position and validate it exists.
fn find_end_tag(after_opts: &str, end_tag: &str, env: &str) -> Result<usize> {
    texutil::find_end_tag(after_opts, end_tag, env)
}

/// Validate the pos option is one of the allowed values.
fn validate_pos_option(opts: &HashMap<String, String>, env: &str) -> Result<()> {
    let pos = opts.get("pos").map(String::as_str).unwrap_or("H");
    if !["H", "t", "b", "h", "p"].contains(&pos) {
        anyhow::bail!(
            "Invalid {} option pos='{}' — valid values are: H, t, b, h, p",
            env,
            pos
        );
    }
    Ok(())
}

/// Build the figure environment LaTeX code.
fn build_figure_environment(
    opts: &HashMap<String, String>,
    _env: &str,
    filename: &str,
) -> Result<String> {
    let pos = opts.get("pos").map(String::as_str);
    let width = opts.get("width").map(String::as_str);
    let height = opts.get("height").map(String::as_str);
    let scale = opts.get("scale").map(String::as_str);
    let keepaspectratio = opts.contains_key("keepaspectratio");
    let label = opts.get("label").map(String::as_str);
    let rel_path = format!("diagrams/{}", filename);

    let mut include_opts = Vec::new();
    if let Some(s) = scale {
        include_opts.push(format!("scale={s}"));
    } else {
        if let Some(w) = width {
            include_opts.push(format!("width={w}"));
        }
        if let Some(h) = height {
            include_opts.push(format!("height={h}"));
        }
    }
    if keepaspectratio {
        include_opts.push("keepaspectratio".to_string());
    }
    let include_str = if include_opts.is_empty() {
        "width=\\linewidth".to_string()
    } else {
        include_opts.join(",")
    };

    let pos_str = pos.map(|p| format!("[{p}]")).unwrap_or_default();
    let mut fig = format!(
        "\\begin{{figure}}{pos_str}\n  \\centering\n  \\includegraphics[{include_str}]{{{rel_path}}}\n"
    );

    add_caption_if_present(opts, &mut fig)?;
    if let Some(lbl) = label {
        fig.push_str(&format!("  \\label{{{lbl}}}\n"));
    }
    fig.push_str("\\end{figure}");

    Ok(fig)
}

/// Add caption to figure environment if present in options.
fn add_caption_if_present(opts: &HashMap<String, String>, fig: &mut String) -> Result<()> {
    if let Some(cap) = opts.get("caption") {
        fig.push_str(&format!("  \\caption{{{}}}\n", cap));
    }
    Ok(())
}

/// Render a DOT/Graphviz diagram to SVG using layout-rs (pure Rust).
///
/// `layout-rs` has no theme concept: `style` is applied by injecting
/// `node [...]; edge [...];` default-attribute statements into the DOT
/// source before parsing (see [`style::graphviz_inject`]).
fn render_graphviz(src: &str, sty: DiagramStyle) -> Result<String> {
    use layout::backends::svg::SVGWriter;
    use layout::gv::DotParser;
    use layout::gv::GraphBuilder;
    use layout::topo::layout::VisualGraph;

    let styled_src = style::graphviz_inject(src, sty);
    let mut parser = DotParser::new(&styled_src);
    let graph = parser.process().map_err(|e| {
        parser.print_error();
        anyhow::anyhow!("Graphviz parse error: {}", e)
    })?;

    let mut builder = GraphBuilder::new();
    builder.visit_graph(&graph);
    let mut vg: VisualGraph = builder.get();

    let mut svg = SVGWriter::new();
    vg.do_it(false, false, false, &mut svg);
    Ok(svg.finalize())
}

/// Render a D2 diagram to SVG using d2-little (pure Rust port of the d2lang pipeline).
///
/// `style` is applied by prepending a `vars: { d2-config: ... }` block (see
/// [`style::d2_prefix`]) — the only way to reach `theme-overrides`, which
/// `CompileOptions` has no field for.
fn render_d2(src: &str, sty: DiagramStyle) -> Result<String> {
    let styled_src = format!("{}{}", style::d2_prefix(sty), src);
    let svg =
        d2_little::d2_to_svg(&styled_src).map_err(|e| anyhow::anyhow!("D2 render error: {}", e))?;
    String::from_utf8(svg).context("D2 produced non-UTF8 SVG")
}

/// Option keys read anywhere in this module. An option outside this list is
/// almost always a typo, so [`parse_opts`] warns instead of dropping it.
const KNOWN_OPTION_KEYS: &[&str] = &[
    "style",
    "pos",
    "width",
    "height",
    "scale",
    "keepaspectratio",
    "label",
    "caption",
];

/// Parse `[key=val, key2=val2]` into a map. Returns `(map, rest_of_str)`.
///
/// Thin wrapper over [`crate::texutil::parse_opts`], which diagrams share with
/// the code-listing pass (`crate::highlight`). The message label keeps the
/// historical `"mermaid diagram: …"` wording; see [`crate::texutil::parse_opts`]
/// for the brace-handling rules.
pub(crate) fn parse_opts<'a>(s: &'a str, env: &str) -> Result<(HashMap<String, String>, &'a str)> {
    texutil::parse_opts(s, &format!("{env} diagram"), KNOWN_OPTION_KEYS)
}

/// Collect .tex files reachable from entry via \input.
fn collect_tex_files(root: &Path, entry: &str) -> Vec<PathBuf> {
    let mut files = Vec::new();
    collect_recursive(root, entry, &mut files);
    files
}

fn collect_recursive(root: &Path, entry: &str, files: &mut Vec<PathBuf>) {
    let path = resolve_tex(root, entry);
    if !path.exists() || files.contains(&path) {
        return;
    }
    files.push(path.clone());
    if let Ok(content) = std::fs::read_to_string(&path) {
        for line in content.lines() {
            for input in extract_inputs(line) {
                collect_recursive(root, input, files);
            }
        }
    }
}

fn extract_inputs(line: &str) -> Vec<&str> {
    let mut results = Vec::new();
    let mut search = line;
    while let Some(pos) = search.find("\\input{") {
        let after = &search[pos + 7..];
        if let Some(end) = after.find('}') {
            results.push(after[..end].trim());
            search = &after[end + 1..];
        } else {
            break;
        }
    }
    results
}

fn resolve_tex(root: &Path, input: &str) -> PathBuf {
    let p = root.join(input);
    if p.extension().is_some() {
        p
    } else {
        p.with_extension("tex")
    }
}

/// Rasterization scale for SVG → PNG. Mermaid SVGs are sized in CSS pixels
/// (~96 dpi); 3x yields ~300 dpi when the figure is included at \linewidth,
/// which is print quality.
const RASTER_SCALE: f32 = 3.0;

/// Convert SVG string to PDF bytes, embedding it as vector art (with
/// selectable text) rather than rasterizing it.
///
/// Takes the SVG as a string rather than a pre-parsed `usvg::Tree`: `svg2pdf`
/// depends on `usvg ^0.45` while the rest of texforge is on `usvg 0.48`, and
/// those are distinct incompatible types. Parsing here, with svg2pdf's own
/// bundled usvg, avoids needing to bridge the two.
fn svg_to_pdf(svg: &str) -> Result<Vec<u8>> {
    let options = svg2pdf::usvg::Options {
        fontdb: shared_svg2pdf_fontdb(),
        shape_rendering: svg2pdf::usvg::ShapeRendering::GeometricPrecision,
        text_rendering: svg2pdf::usvg::TextRendering::OptimizeLegibility,
        ..Default::default()
    };

    let tree =
        svg2pdf::usvg::Tree::from_str(svg, &options).context("Failed to parse SVG for PDF")?;

    svg2pdf::to_pdf(
        &tree,
        svg2pdf::ConversionOptions::default(),
        svg2pdf::PageOptions::default(),
    )
    .map_err(|e| anyhow::anyhow!("SVG to PDF conversion failed: {}", e))
}

/// Convert a diagram's SVG to a vector PDF; fall back to a rasterized PNG,
/// with a warning naming the diagram, if the PDF conversion fails.
///
/// Returns the encoded bytes alongside the extension actually produced, so
/// callers can name the cached artefact accordingly.
fn convert_svg_or_fallback(env: &str, svg: &str) -> Result<(Vec<u8>, &'static str)> {
    match svg_to_pdf(svg) {
        Ok(pdf) => Ok((pdf, "pdf")),
        Err(e) => {
            eprintln!(
                "warning: {env} diagram: SVG to PDF conversion failed ({e}), falling back to PNG"
            );
            let png = svg_to_png(svg).context("Failed to convert diagram SVG to PNG (fallback)")?;
            Ok((png, "png"))
        }
    }
}

/// Convert SVG string to PNG bytes at print resolution.
fn svg_to_png(svg: &str) -> Result<Vec<u8>> {
    let options = resvg::usvg::Options {
        fontdb: shared_fontdb(),
        shape_rendering: resvg::usvg::ShapeRendering::GeometricPrecision,
        text_rendering: resvg::usvg::TextRendering::OptimizeLegibility,
        ..Default::default()
    };

    let tree = resvg::usvg::Tree::from_str(svg, &options).context("Failed to parse SVG")?;

    let original_size = tree.size();
    let padding = 10.0; // padding (in SVG units) so strokes at the edge aren't clipped
    let width = ((original_size.width() + padding * 2.0) * RASTER_SCALE) as u32;
    let height = ((original_size.height() + padding * 2.0) * RASTER_SCALE) as u32;

    let mut pixmap =
        resvg::tiny_skia::Pixmap::new(width, height).context("Failed to create pixmap")?;

    let transform = resvg::tiny_skia::Transform::from_scale(RASTER_SCALE, RASTER_SCALE)
        .post_translate(padding * RASTER_SCALE, padding * RASTER_SCALE);

    resvg::render(&tree, transform, &mut pixmap.as_mut());

    pixmap.encode_png().context("Failed to encode PNG")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds an SVG whose group nesting is deep enough to exceed svg2pdf's
    /// PDF content-stream nesting guard (`state_nesting_depth() > 28`), while
    /// remaining well within what resvg's rasterizer renders without issue.
    /// Each level carries a decoy sibling rect so usvg can't flatten the
    /// single-child transform chain into one group.
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

    #[test]
    fn svg_to_pdf_produces_pdf_magic_bytes() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><rect width="10" height="10" fill="red"/></svg>"#;
        let pdf = svg_to_pdf(svg).unwrap();
        assert!(
            pdf.starts_with(b"%PDF-"),
            "expected PDF magic bytes, got: {:?}",
            &pdf[..pdf.len().min(20)]
        );
    }

    /// Text in a converted diagram must actually be typeset: the shared
    /// `fontdb` is what lets `svg2pdf` find a face and embed it. Dropping it
    /// from the options renders the same `<text>` node with no font at all
    /// (a 1.5 kB PDF with no `/FontFile`), silently losing every label.
    #[test]
    fn svg_to_pdf_embeds_a_font_for_text_from_the_shared_fontdb() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="50"><text x="10" y="30">Hello</text></svg>"#;
        let pdf = svg_to_pdf(svg).unwrap();
        let text = String::from_utf8_lossy(&pdf);
        assert!(
            text.contains("FontFile"),
            "expected an embedded font program for the <text> node"
        );
    }

    #[test]
    fn svg_to_pdf_fails_on_excessive_nesting_that_png_still_handles() {
        let svg = deeply_nested_svg();
        assert!(
            svg_to_pdf(&svg).is_err(),
            "expected the deeply nested SVG to exceed svg2pdf's nesting guard"
        );
        assert!(
            svg_to_png(&svg).is_ok(),
            "rasterization should not be affected by the PDF nesting guard"
        );
    }

    #[test]
    fn convert_svg_or_fallback_falls_back_to_png_on_pdf_failure() {
        let svg = deeply_nested_svg();
        let (bytes, ext) = convert_svg_or_fallback("graphviz", &svg).unwrap();
        assert_eq!(ext, "png");
        assert!(!bytes.is_empty());
    }

    #[test]
    fn content_hash_stable_across_calls() {
        let src = "digraph G { A -> B }";
        assert_eq!(
            content_hash(src, DiagramStyle::Default),
            content_hash(src, DiagramStyle::Default)
        );
    }

    #[test]
    fn content_hash_differs_by_style() {
        let src = "digraph G { A -> B }";
        assert_ne!(
            content_hash(src, DiagramStyle::Default),
            content_hash(src, DiagramStyle::Editorial)
        );
    }

    #[test]
    fn parse_opts_no_brackets_returns_empty_map() {
        let (map, rest) = parse_opts("hello", "mermaid").unwrap();
        assert!(map.is_empty());
        assert_eq!(rest, "hello");
    }

    #[test]
    fn parse_opts_width_and_pos() {
        let (map, _) = parse_opts("[width=0.5\\linewidth, pos=t]", "mermaid").unwrap();
        assert_eq!(map.get("width").map(String::as_str), Some("0.5\\linewidth"));
        assert_eq!(map.get("pos").map(String::as_str), Some("t"));
    }

    #[test]
    fn parse_opts_caption() {
        let (map, _) = parse_opts("[caption=My diagram]", "mermaid").unwrap();
        assert_eq!(map.get("caption").map(String::as_str), Some("My diagram"));
    }

    #[test]
    fn parse_opts_label_and_height() {
        let (map, _) = parse_opts("[label=fig:my-diagram, height=5cm]", "mermaid").unwrap();
        assert_eq!(map.get("label").map(String::as_str), Some("fig:my-diagram"));
        assert_eq!(map.get("height").map(String::as_str), Some("5cm"));
    }

    #[test]
    fn parse_opts_style_alongside_others_any_order() {
        let (map, _) =
            parse_opts("[pos=t, style=editorial, width=0.5\\linewidth]", "mermaid").unwrap();
        assert_eq!(map.get("style").map(String::as_str), Some("editorial"));
        assert_eq!(map.get("pos").map(String::as_str), Some("t"));
        assert_eq!(map.get("width").map(String::as_str), Some("0.5\\linewidth"));

        let (map, _) = parse_opts("[style=monochrome, caption=A diagram]", "mermaid").unwrap();
        assert_eq!(map.get("style").map(String::as_str), Some("monochrome"));
        assert_eq!(map.get("caption").map(String::as_str), Some("A diagram"));
    }

    #[test]
    fn parse_opts_braced_caption_keeps_commas() {
        let (map, _) = parse_opts("[caption={A, with commas}]", "mermaid").unwrap();
        assert_eq!(
            map.get("caption").map(String::as_str),
            Some("A, with commas")
        );
    }

    #[test]
    fn parse_opts_braced_caption_preserves_nested_braces() {
        let (map, _) = parse_opts("[caption={\\texttt{a, b}}]", "mermaid").unwrap();
        assert_eq!(
            map.get("caption").map(String::as_str),
            Some("\\texttt{a, b}")
        );
    }

    #[test]
    fn parse_opts_unbraced_caption_unchanged() {
        let (map, _) = parse_opts("[caption=simple]", "mermaid").unwrap();
        assert_eq!(map.get("caption").map(String::as_str), Some("simple"));
    }

    #[test]
    fn parse_opts_braced_value_mixed_with_others_any_order() {
        let (map, _) = parse_opts(
            "[style=editorial, width=0.5\\linewidth, caption={x, y}, pos=H]",
            "mermaid",
        )
        .unwrap();
        assert_eq!(map.get("style").map(String::as_str), Some("editorial"));
        assert_eq!(map.get("width").map(String::as_str), Some("0.5\\linewidth"));
        assert_eq!(map.get("caption").map(String::as_str), Some("x, y"));
        assert_eq!(map.get("pos").map(String::as_str), Some("H"));
    }

    #[test]
    fn parse_opts_unknown_key_warns_but_does_not_abort() {
        let (map, _) = parse_opts("[style=default, frobnicate=yes, pos=t]", "mermaid").unwrap();
        assert_eq!(map.get("style").map(String::as_str), Some("default"));
        assert_eq!(map.get("pos").map(String::as_str), Some("t"));
        assert!(!map.contains_key("frobnicate"));
    }

    #[test]
    fn parse_opts_unterminated_brace_fails_naming_environment() {
        let err = parse_opts("[caption={unterminated]", "mermaid").unwrap_err();
        let message = err.to_string();
        assert!(message.contains("mermaid"), "message was: {message}");
        assert!(message.contains("caption"), "message was: {message}");
    }

    /// TF-C: a caption with commas used to be silently truncated at the first
    /// one — this is the exact broken line from the capabilities example
    /// (commit 2b848dc), used verbatim to guard against regressing it.
    #[test]
    fn parse_opts_defect_tf_c_caption_with_commas_not_truncated() {
        let (map, _) = parse_opts(
            "[style=editorial, width=0.55\\linewidth, caption={Preset \\texttt{editorial}: paleta restringida, un solo acento, sin sombras}, pos=H]",
            "mermaid",
        )
        .unwrap();
        assert_eq!(
            map.get("caption").map(String::as_str),
            Some("Preset \\texttt{editorial}: paleta restringida, un solo acento, sin sombras")
        );
        assert_eq!(map.get("pos").map(String::as_str), Some("H"));
    }

    #[test]
    fn build_figure_with_label() {
        let mut opts = HashMap::new();
        opts.insert("caption".to_string(), "Test".to_string());
        opts.insert("label".to_string(), "fig:test".to_string());
        let fig = build_figure_environment(&opts, "mermaid", "d1.png").unwrap();
        assert!(fig.contains("\\label{fig:test}"));
        assert!(fig.contains("\\caption{Test}"));
        assert!(fig.contains("\\begin{figure}"));
    }

    #[test]
    fn build_figure_with_height() {
        let mut opts = HashMap::new();
        opts.insert("height".to_string(), "5cm".to_string());
        let fig = build_figure_environment(&opts, "mermaid", "d1.png").unwrap();
        assert!(fig.contains("height=5cm"));
    }

    #[test]
    fn build_figure_with_width_and_height() {
        let mut opts = HashMap::new();
        opts.insert("width".to_string(), "0.5\\linewidth".to_string());
        opts.insert("height".to_string(), "4cm".to_string());
        let fig = build_figure_environment(&opts, "mermaid", "d1.png").unwrap();
        assert!(fig.contains("width=0.5\\linewidth"));
        assert!(fig.contains("height=4cm"));
    }

    #[test]
    fn build_figure_with_scale() {
        let mut opts = HashMap::new();
        opts.insert("scale".to_string(), "0.8".to_string());
        let fig = build_figure_environment(&opts, "mermaid", "d1.png").unwrap();
        assert!(fig.contains("scale=0.8"));
        assert!(!fig.contains("width="));
    }

    #[test]
    fn build_figure_with_keepaspectratio() {
        let mut opts = HashMap::new();
        opts.insert("width".to_string(), "10cm".to_string());
        opts.insert("height".to_string(), "8cm".to_string());
        opts.insert("keepaspectratio".to_string(), "true".to_string());
        let fig = build_figure_environment(&opts, "mermaid", "d1.png").unwrap();
        assert!(fig.contains("keepaspectratio"));
    }

    #[test]
    fn build_figure_default_no_pos() {
        let opts = HashMap::new();
        let fig = build_figure_environment(&opts, "mermaid", "d1.png").unwrap();
        assert!(fig.contains("\\begin{figure}\n"));
        assert!(!fig.contains("[H]"));
    }

    #[test]
    fn render_graphviz_produces_svg() {
        let dot = "digraph G { A -> B }";
        let svg = render_graphviz(dot, DiagramStyle::Default).unwrap();
        assert!(
            svg.contains("<svg"),
            "expected SVG output, got: {}",
            &svg[..100.min(svg.len())]
        );
    }

    #[test]
    fn render_d2_produces_svg() {
        let svg = render_d2("a -> b -> c", DiagramStyle::Default).unwrap();
        assert!(
            svg.contains("<svg"),
            "expected SVG output, got: {}",
            &svg[..100.min(svg.len())]
        );
    }

    #[test]
    fn render_d2_to_pdf_via_pipeline() {
        let dir = tempfile::tempdir().unwrap();
        let content = "\\begin{d2}[caption=Flow]\nx -> y: go\n\\end{d2}";
        let out = render_diagrams(content, dir.path(), DiagramStyle::Default).unwrap();
        assert!(out.contains("\\includegraphics"));
        assert!(out.contains(".pdf"));
        assert!(out.contains("\\caption{Flow}"));
        // exactly one cached artefact written, and it's the vector form
        let mut entries = std::fs::read_dir(dir.path()).unwrap();
        let entry = entries.next().unwrap().unwrap();
        assert!(entries.next().is_none());
        assert_eq!(entry.path().extension().unwrap(), "pdf");
    }

    #[test]
    fn render_mermaid_to_pdf_via_pipeline() {
        let dir = tempfile::tempdir().unwrap();
        let content = "\\begin{mermaid}\nflowchart LR\n  A --> B\n\\end{mermaid}";
        let out = render_diagrams(content, dir.path(), DiagramStyle::Default).unwrap();
        assert!(out.contains(".pdf"));
    }

    #[test]
    fn render_graphviz_to_pdf_via_pipeline() {
        let dir = tempfile::tempdir().unwrap();
        let content = "\\begin{graphviz}\ndigraph G { A -> B }\n\\end{graphviz}";
        let out = render_diagrams(content, dir.path(), DiagramStyle::Default).unwrap();
        assert!(out.contains(".pdf"));
    }

    #[test]
    fn render_diagrams_with_editorial_style_attribute_succeeds() {
        let dir = tempfile::tempdir().unwrap();
        let content = "\\begin{mermaid}[style=editorial]\nflowchart LR\n  A --> B\n\\end{mermaid}";
        let out = render_diagrams(content, dir.path(), DiagramStyle::Default).unwrap();
        assert!(out.contains(".pdf"));
    }

    #[test]
    fn render_diagrams_unknown_style_attribute_fails_build() {
        let dir = tempfile::tempdir().unwrap();
        let content = "\\begin{mermaid}[style=editoral]\nflowchart LR\n  A --> B\n\\end{mermaid}";
        let err = render_diagrams(content, dir.path(), DiagramStyle::Default).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("editoral"), "message: {msg}");
    }

    #[test]
    fn render_env_build_succeeds_when_pdf_conversion_fails() {
        // Exercises the same fallback a real diagram render would take when
        // its SVG trips svg2pdf's nesting guard: the build must still
        // succeed, emitting a PNG artefact rather than failing outright.
        let content = "\\begin{graphviz}\ndigraph G { A -> B }\n\\end{graphviz}";
        let dir = tempfile::tempdir().unwrap();
        let svg = deeply_nested_svg();

        let result = render_env(
            content,
            "graphviz",
            dir.path(),
            DiagramStyle::Default,
            |_, _| convert_svg_or_fallback("graphviz", &svg),
        );

        assert!(result.is_ok(), "build must succeed via the PNG fallback");
        let out = result.unwrap();
        assert!(out.contains(".png"));
        let entry = std::fs::read_dir(dir.path())
            .unwrap()
            .next()
            .unwrap()
            .unwrap();
        assert_eq!(entry.path().extension().unwrap(), "png");
    }

    #[test]
    fn render_env_no_blocks_unchanged() {
        let content = "hello world";
        let dir = tempfile::tempdir().unwrap();
        let result = render_env(
            content,
            "graphviz",
            dir.path(),
            DiagramStyle::Default,
            |_, _| Ok((vec![], "pdf")),
        )
        .unwrap();
        assert_eq!(result, content);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn render_env_invalid_pos_returns_error() {
        let content = "\\begin{graphviz}[pos=Z]\ndigraph G{}\n\\end{graphviz}";
        let dir = tempfile::tempdir().unwrap();
        let err = render_env(
            content,
            "graphviz",
            dir.path(),
            DiagramStyle::Default,
            |_, _| Ok((vec![1, 2, 3], "pdf")),
        )
        .unwrap_err();
        assert!(err.to_string().contains("pos='Z'"));
    }

    #[test]
    fn render_env_invalid_style_returns_error_listing_valid_names() {
        let content = "\\begin{graphviz}[style=editoral]\ndigraph G{}\n\\end{graphviz}";
        let dir = tempfile::tempdir().unwrap();
        let err = render_env(
            content,
            "graphviz",
            dir.path(),
            DiagramStyle::Default,
            |_, _| Ok((vec![1, 2, 3], "pdf")),
        )
        .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("style='editoral'"), "message: {msg}");
        for name in style::VALID_STYLE_NAMES {
            assert!(msg.contains(name), "message missing '{name}': {msg}");
        }
    }

    #[test]
    fn render_env_reuses_cached_diagram() {
        let content = "\\begin{graphviz}\ndigraph G{ A -> B }\n\\end{graphviz}";
        let dir = tempfile::tempdir().unwrap();
        let calls = std::cell::Cell::new(0u32);
        // render twice into the same dir — second pass must hit the cache
        for _ in 0..2 {
            render_env(
                content,
                "graphviz",
                dir.path(),
                DiagramStyle::Default,
                |_, _| {
                    calls.set(calls.get() + 1);
                    Ok((vec![1, 2, 3], "pdf"))
                },
            )
            .unwrap();
        }
        assert_eq!(calls.get(), 1);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn render_env_falls_back_to_png_extension() {
        let content = "\\begin{graphviz}\ndigraph G{ A -> B }\n\\end{graphviz}";
        let dir = tempfile::tempdir().unwrap();
        let out = render_env(
            content,
            "graphviz",
            dir.path(),
            DiagramStyle::Default,
            |_, _| Ok((vec![1, 2, 3], "png")),
        )
        .unwrap();
        assert!(out.contains(".png"));
        let mut entries = std::fs::read_dir(dir.path()).unwrap();
        let entry = entries.next().unwrap().unwrap();
        assert_eq!(entry.path().extension().unwrap(), "png");
    }

    #[test]
    fn render_env_reuses_cached_png_without_recomputing() {
        let content = "\\begin{graphviz}\ndigraph G{ A -> B }\n\\end{graphviz}";
        let dir = tempfile::tempdir().unwrap();
        // A prior run fell back to PNG for this diagram.
        render_env(
            content,
            "graphviz",
            dir.path(),
            DiagramStyle::Default,
            |_, _| Ok((vec![1, 2, 3], "png")),
        )
        .unwrap();

        let calls = std::cell::Cell::new(0u32);
        render_env(
            content,
            "graphviz",
            dir.path(),
            DiagramStyle::Default,
            |_, _| {
                calls.set(calls.get() + 1);
                Ok((vec![9, 9, 9], "pdf"))
            },
        )
        .unwrap();
        assert_eq!(
            calls.get(),
            0,
            "cached .png should be reused, not re-rendered"
        );
    }

    #[test]
    fn render_env_omitted_style_matches_explicit_default_style() {
        // Omitting `style=` must render exactly as `style=default`: both
        // resolve to `DiagramStyle::Default`, so they hash to the same
        // cache entry and only one artefact is written.
        let dir = tempfile::tempdir().unwrap();
        let content_omitted = "\\begin{graphviz}\ndigraph G{ A -> B }\n\\end{graphviz}";
        let content_explicit =
            "\\begin{graphviz}[style=default]\ndigraph G{ A -> B }\n\\end{graphviz}";

        render_env(
            content_omitted,
            "graphviz",
            dir.path(),
            DiagramStyle::Default,
            |_, sty| {
                assert_eq!(sty, DiagramStyle::Default);
                Ok((vec![1, 2, 3], "pdf"))
            },
        )
        .unwrap();
        render_env(
            content_explicit,
            "graphviz",
            dir.path(),
            DiagramStyle::Default,
            |_, sty| {
                assert_eq!(sty, DiagramStyle::Default);
                Ok((vec![1, 2, 3], "pdf"))
            },
        )
        .unwrap();

        assert_eq!(
            std::fs::read_dir(dir.path()).unwrap().count(),
            1,
            "omitted style= and explicit style=default must share one cache entry"
        );
    }

    #[test]
    fn render_env_project_default_style_used_when_attribute_absent() {
        let dir = tempfile::tempdir().unwrap();
        let content = "\\begin{graphviz}\ndigraph G{ A -> B }\n\\end{graphviz}";
        render_env(
            content,
            "graphviz",
            dir.path(),
            DiagramStyle::Editorial,
            |_, sty| {
                assert_eq!(sty, DiagramStyle::Editorial);
                Ok((vec![1, 2, 3], "pdf"))
            },
        )
        .unwrap();
    }

    #[test]
    fn render_env_attribute_style_overrides_project_default() {
        let dir = tempfile::tempdir().unwrap();
        let content = "\\begin{graphviz}[style=monochrome]\ndigraph G{ A -> B }\n\\end{graphviz}";
        render_env(
            content,
            "graphviz",
            dir.path(),
            DiagramStyle::Editorial,
            |_, sty| {
                assert_eq!(sty, DiagramStyle::Monochrome);
                Ok((vec![1, 2, 3], "pdf"))
            },
        )
        .unwrap();
    }

    #[test]
    fn render_env_different_styles_produce_distinct_cache_entries() {
        let dir = tempfile::tempdir().unwrap();
        let content_default = "\\begin{graphviz}\ndigraph G{ A -> B }\n\\end{graphviz}";
        let content_editorial =
            "\\begin{graphviz}[style=editorial]\ndigraph G{ A -> B }\n\\end{graphviz}";
        render_env(
            content_default,
            "graphviz",
            dir.path(),
            DiagramStyle::Default,
            |_, _| Ok((vec![1], "pdf")),
        )
        .unwrap();
        render_env(
            content_editorial,
            "graphviz",
            dir.path(),
            DiagramStyle::Default,
            |_, _| Ok((vec![2], "pdf")),
        )
        .unwrap();
        assert_eq!(
            std::fs::read_dir(dir.path()).unwrap().count(),
            2,
            "different styles must produce distinct cached artefacts"
        );
    }

    fn hex_colors_for_attr(svg: &str, attr: &str) -> Vec<(u8, u8, u8)> {
        let pattern = format!("{attr}=\"#");
        let mut out = Vec::new();
        let mut rest = svg;
        while let Some(idx) = rest.find(&pattern) {
            let after = &rest[idx + pattern.len()..];
            if after.len() >= 6 {
                let hex = &after[..6];
                if let (Ok(r), Ok(g), Ok(b)) = (
                    u8::from_str_radix(&hex[0..2], 16),
                    u8::from_str_radix(&hex[2..4], 16),
                    u8::from_str_radix(&hex[4..6], 16),
                ) {
                    out.push((r, g, b));
                }
            }
            rest = &after[6.min(after.len())..];
        }
        out
    }

    /// Grayscale means R == G == B for every `fill`/`stroke` hex color —
    /// the requirement `monochrome` exists to guarantee.
    fn assert_grayscale_svg(svg: &str, label: &str) {
        for attr in ["fill", "stroke"] {
            for (r, g, b) in hex_colors_for_attr(svg, attr) {
                assert!(
                    r == g && g == b,
                    "{label}: non-grayscale {attr} color #{r:02X}{g:02X}{b:02X} under monochrome style"
                );
            }
        }
    }

    #[test]
    fn monochrome_style_mermaid_svg_is_grayscale() {
        let svg =
            render_mermaid_with_config("flowchart LR\n  A --> B --> C", DiagramStyle::Monochrome)
                .unwrap();
        assert_grayscale_svg(&svg, "mermaid");
    }

    #[test]
    fn monochrome_style_graphviz_svg_is_grayscale() {
        let svg = render_graphviz("digraph G { A -> B }", DiagramStyle::Monochrome).unwrap();
        assert_grayscale_svg(&svg, "graphviz");
    }

    #[test]
    fn monochrome_style_d2_svg_is_grayscale() {
        let svg = render_d2("a -> b -> c", DiagramStyle::Monochrome).unwrap();
        assert_grayscale_svg(&svg, "d2");
    }

    #[test]
    fn technical_style_renders_across_all_three_renderers() {
        // technical is the trickiest preset: D2 reaches its monospaced-label
        // rule via a base theme-id (301) injected alongside theme-overrides.
        let svg =
            render_mermaid_with_config("flowchart LR\n  A --> B", DiagramStyle::Technical).unwrap();
        assert!(svg.contains("<svg"), "mermaid: {svg}");

        let svg = render_graphviz("digraph G { A -> B }", DiagramStyle::Technical).unwrap();
        assert!(svg.contains("<svg"), "graphviz: {svg}");

        let svg = render_d2("a -> b -> c", DiagramStyle::Technical).unwrap();
        assert!(svg.contains("<svg"), "d2: {svg}");
    }

    #[test]
    fn graphviz_renders_despite_unsupported_background_override() {
        // layout-rs has no document-wide theme or background — StyleAttr is
        // per-node/per-edge only — so a preset it can't fully express must
        // still render successfully rather than fail the build.
        for sty in [
            DiagramStyle::Editorial,
            DiagramStyle::Monochrome,
            DiagramStyle::Technical,
        ] {
            let svg = render_graphviz("digraph G { A -> B }", sty).unwrap();
            assert!(svg.contains("<svg"));
        }
    }

    #[test]
    fn build_figure_default_uses_linewidth() {
        let opts = HashMap::new();
        let fig = build_figure_environment(&opts, "mermaid", "d1.png").unwrap();
        assert!(fig.contains("\\includegraphics[width=\\linewidth]"));
    }

    #[test]
    fn build_figure_with_pos_t() {
        let mut opts = HashMap::new();
        opts.insert("pos".to_string(), "t".to_string());
        let fig = build_figure_environment(&opts, "mermaid", "d1.png").unwrap();
        assert!(fig.contains("\\begin{figure}[t]"));
    }

    // ── FR4: the diagram pass satisfies its own package dependencies ───────

    /// Run the full [`process`] over a one-file project and return the
    /// rewritten temp entry (never the source, which stays byte-identical).
    fn process_fixture(source: &str) -> String {
        let root = tempfile::tempdir().unwrap();
        let build = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("main.tex"), source).unwrap();
        process(root.path(), "main.tex", build.path(), DiagramStyle::Default).unwrap();
        std::fs::read_to_string(build.path().join("main.tex")).unwrap()
    }

    /// The physical line immediately before the `\begin{document}` anchor.
    fn line_before_anchor(out: &str) -> &str {
        let anchor = out.find("\\begin{document}").expect("anchor present");
        out[..anchor]
            .trim_end_matches('\n')
            .rsplit('\n')
            .next()
            .unwrap()
    }

    #[test]
    fn process_inserts_exactly_one_graphicx_for_a_mermaid_document() {
        let out = process_fixture(
            "\\documentclass{article}\n\\usepackage{booktabs}\n\\begin{document}\n\
             \\begin{mermaid}[style=editorial, caption={Flow}]\nflowchart LR\n  A --> B\n\\end{mermaid}\n\
             \\end{document}\n",
        );
        assert_eq!(
            out.matches("\\usepackage{graphicx}").count(),
            1,
            "exactly one inserted graphicx line:\n{out}"
        );
        let inserted = out.find("\\usepackage{graphicx}").unwrap();
        let anchor = out.find("\\begin{document}").unwrap();
        assert!(inserted < anchor, "insertion must precede the anchor");
        assert_eq!(line_before_anchor(&out), "\\usepackage{graphicx}");
        // The rewrite itself still happened.
        assert!(out.contains("\\includegraphics"), "{out}");
    }

    #[test]
    fn process_inserts_nothing_when_the_preamble_loads_amsmath_graphicx() {
        let out = process_fixture(
            "\\documentclass{article}\n\\usepackage{amsmath,graphicx}\n\\begin{document}\n\
             \\begin{graphviz}[caption={Flow}]\ndigraph G { A -> B }\n\\end{graphviz}\n\
             \\end{document}\n",
        );
        assert!(out.contains("\\includegraphics"), "rewritten:\n{out}");
        // Assert on the exact insertion string: the fixture's own comma
        // list contains "graphicx" but is not `\usepackage{graphicx}`.
        assert_eq!(out.matches("\\usepackage{graphicx}").count(), 0, "{out}");
        assert_eq!(out.matches("\\usepackage{float}").count(), 0, "{out}");
    }

    #[test]
    fn process_inserts_nothing_without_a_diagram() {
        let source =
            "\\documentclass{article}\n\\usepackage{booktabs}\n\\begin{document}\nHi.\n\\end{document}\n";
        let out = process_fixture(source);
        assert_eq!(out, source, "build copy must be byte-identical");
    }

    #[test]
    fn process_inserts_float_for_pos_h_when_missing() {
        let out = process_fixture(
            "\\documentclass{article}\n\\begin{document}\n\
             \\begin{graphviz}[pos=H, caption={Flow}]\ndigraph G { A -> B }\n\\end{graphviz}\n\
             \\end{document}\n",
        );
        assert_eq!(out.matches("\\usepackage{graphicx}").count(), 1, "{out}");
        assert_eq!(out.matches("\\usepackage{float}").count(), 1, "{out}");
        let anchor = out.find("\\begin{document}").unwrap();
        assert!(out.find("\\usepackage{float}").unwrap() < anchor, "{out}");
        assert_eq!(line_before_anchor(&out), "\\usepackage{float}");
    }

    #[test]
    fn process_skips_float_when_the_preamble_loads_it_or_pos_is_not_h() {
        // (a) `float` already loaded (with options — a blind duplicate would
        // clash) → no inserted line, graphicx still added.
        let out = process_fixture(
            "\\documentclass{article}\n\\usepackage[table]{float}\n\\begin{document}\n\
             \\begin{graphviz}[pos=H]\ndigraph G { A -> B }\n\\end{graphviz}\n\
             \\end{document}\n",
        );
        assert_eq!(out.matches("\\usepackage{graphicx}").count(), 1, "{out}");
        assert_eq!(out.matches("\\usepackage{float}").count(), 0, "{out}");

        // (b) no `[H]` figure → float is never needed (the D1 negative pin).
        let out = process_fixture(
            "\\documentclass{article}\n\\begin{document}\n\
             \\begin{graphviz}[pos=t]\ndigraph G { A -> B }\n\\end{graphviz}\n\
             \\end{document}\n",
        );
        assert_eq!(out.matches("\\usepackage{graphicx}").count(), 1, "{out}");
        assert_eq!(out.matches("\\usepackage{float}").count(), 0, "{out}");
    }

    #[test]
    fn process_detects_graphicx_in_an_input_preamble_file() {
        let root = tempfile::tempdir().unwrap();
        let build = tempfile::tempdir().unwrap();
        std::fs::write(
            root.path().join("main.tex"),
            "\\documentclass{article}\n\\input{pre}\n\\begin{document}\n\
             \\begin{graphviz}[pos=t]\ndigraph G { A -> B }\n\\end{graphviz}\n\
             \\end{document}\n",
        )
        .unwrap();
        std::fs::write(root.path().join("pre.tex"), "\\usepackage{graphicx}\n").unwrap();
        process(root.path(), "main.tex", build.path(), DiagramStyle::Default).unwrap();
        let out = std::fs::read_to_string(build.path().join("main.tex")).unwrap();
        assert!(out.contains("\\includegraphics"), "rewritten:\n{out}");
        assert_eq!(out.matches("\\usepackage{graphicx}").count(), 0, "{out}");
    }
}
