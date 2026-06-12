//! Pre-processor for embedded diagram environments.
//!
//! Intercepts `\begin{mermaid}[opts]...\end{mermaid}` blocks, renders them
//! to PNG, and replaces each block with a proper `figure` environment.
//!
//! Works on copies in `build/` — the original .tex files are never modified.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use anyhow::{Context, Result};

/// Copy all .tex files to `build_dir`, rendering embedded diagrams in the copies.
/// Also mirrors non-.tex assets so tectonic can resolve relative paths.
/// Returns the path to the build copy of `entry`.
pub fn process(root: &Path, entry: &str, build_dir: &Path) -> Result<PathBuf> {
    std::fs::create_dir_all(build_dir)?;

    let diagrams_dir = build_dir.join("diagrams");
    std::fs::create_dir_all(&diagrams_dir)?;

    // Process .tex files
    let tex_files = collect_tex_files(root, entry);
    for src in &tex_files {
        let rel = src.strip_prefix(root).unwrap_or(src);
        let dest = build_dir.join(rel);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = std::fs::read_to_string(src)?;
        let processed = render_diagrams(&content, &diagrams_dir)
            .with_context(|| format!("Failed to render diagrams in {}", src.display()))?;
        std::fs::write(&dest, processed)?;
    }

    // Mirror asset files so tectonic resolves relative paths
    crate::utils::mirror_assets(root, build_dir)?;

    Ok(build_dir.join(entry))
}

/// Replace all `\begin{mermaid}[opts]...\end{mermaid}` with figure environments.
fn render_diagrams(content: &str, diagrams_dir: &Path) -> Result<String> {
    let content = render_env(content, "mermaid", diagrams_dir, |src| {
        let svg = render_mermaid_with_config(src)
            .map_err(|e| anyhow::anyhow!("Mermaid render error: {}", e))?;
        svg_to_png(&svg).context("Failed to convert mermaid SVG to PNG")
    })?;
    let content = render_env(&content, "graphviz", diagrams_dir, |src| {
        let svg = render_graphviz(src)?;
        svg_to_png(&svg).context("Failed to convert graphviz SVG to PNG")
    })?;
    Ok(content)
}

/// Render Mermaid diagram with improved configuration for better layout.
fn render_mermaid_with_config(src: &str) -> Result<String> {
    // Try with default configuration first
    mermaid_rs_renderer::render(src).map_err(|e| {
        // If default fails, try with explicit configuration
        anyhow::anyhow!(
            "Mermaid render error: {}. Consider checking diagram syntax.",
            e
        )
    })
}

/// Generic environment renderer: replaces `\begin{env}[opts]...\end{env}` with figure.
///
/// Rendered PNGs are named after a hash of the diagram source, so unchanged
/// diagrams are reused across rebuilds (watch mode) instead of re-rendered.
pub(crate) fn render_env(
    content: &str,
    env: &str,
    diagrams_dir: &Path,
    render_fn: impl Fn(&str) -> Result<Vec<u8>>,
) -> Result<String> {
    let begin_tag = format!("\\begin{{{}}}", env);
    let end_tag = format!("\\end{{{}}}", env);

    let mut result = String::new();
    let mut remaining: &str = content;

    while let Some(start) = remaining.find(&begin_tag) {
        result.push_str(&remaining[..start]);

        let after_begin = &remaining[start + begin_tag.len()..];
        let (opts, after_opts) = parse_opts(after_begin);

        let end = find_end_tag(after_opts, &end_tag, env)?;
        let diagram_src = after_opts[..end].trim();

        validate_pos_option(&opts, env)?;

        let filename = format!("{}-{:016x}.png", env, content_hash(diagram_src));
        if !diagrams_dir.join(&filename).exists() {
            let png = render_fn(diagram_src)?;
            std::fs::write(diagrams_dir.join(&filename), png)?;
        }
        let fig_env = build_figure_environment(&opts, env, &filename)?;

        result.push_str(&fig_env);
        remaining = &after_opts[end + end_tag.len()..];
    }

    result.push_str(remaining);
    Ok(result)
}

/// Stable-enough 64-bit hash of diagram source for cache filenames.
fn content_hash(src: &str) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    src.hash(&mut hasher);
    hasher.finish()
}

/// Find the end tag position and validate it exists.
fn find_end_tag(after_opts: &str, end_tag: &str, env: &str) -> Result<usize> {
    after_opts
        .find(end_tag)
        .with_context(|| format!("\\begin{{{}}} without matching \\end{{{}}}", env, env))
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
fn render_graphviz(src: &str) -> Result<String> {
    use layout::backends::svg::SVGWriter;
    use layout::gv::DotParser;
    use layout::gv::GraphBuilder;
    use layout::topo::layout::VisualGraph;

    let mut parser = DotParser::new(src);
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

/// Parse `[key=val, key2=val2]` into a map. Returns `(map, rest_of_str)`.
pub(crate) fn parse_opts(s: &str) -> (HashMap<String, String>, &str) {
    let s = s.trim_start_matches('\n').trim_start_matches('\r');
    if !s.starts_with('[') {
        return (HashMap::new(), s);
    }
    let Some(end) = s.find(']') else {
        return (HashMap::new(), s);
    };
    let inner = &s[1..end];
    let rest = &s[end + 1..];

    let mut map = HashMap::new();
    for part in inner.split(',') {
        let part = part.trim();
        if let Some((k, v)) = part.split_once('=') {
            map.insert(k.trim().to_string(), v.trim().to_string());
        }
    }
    (map, rest)
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

/// Shared font database — building it scans system font directories (very slow
/// on WSL, where /mnt/c/Windows/Fonts goes through the 9P filesystem), so it is
/// built once and reused for every diagram.
fn shared_fontdb() -> Arc<resvg::usvg::fontdb::Database> {
    static FONTDB: OnceLock<Arc<resvg::usvg::fontdb::Database>> = OnceLock::new();
    FONTDB.get_or_init(|| Arc::new(build_fontdb())).clone()
}

/// Build a font database with system fonts and platform-specific fallbacks.
fn build_fontdb() -> resvg::usvg::fontdb::Database {
    use resvg::usvg::fontdb::Database;

    let mut db = Database::new();
    load_system_and_platform_fonts(&mut db);
    load_fallback_font_directories(&mut db);
    configure_font_families(&mut db);

    db
}

/// Load system fonts and platform-specific fonts (Windows/WSL).
fn load_system_and_platform_fonts(db: &mut resvg::usvg::fontdb::Database) {
    db.load_system_fonts();

    // On WSL / Windows, also load the Windows font directory
    let win_fonts = std::path::Path::new("/mnt/c/Windows/Fonts");
    if win_fonts.is_dir() {
        db.load_fonts_dir(win_fonts);
    }
}

/// Load fallback font directories if no fonts were found.
fn load_fallback_font_directories(db: &mut resvg::usvg::fontdb::Database) {
    // If the DB still has no fonts at all, try common directories explicitly.
    if db.is_empty() {
        for dir in ["/usr/share/fonts", "/usr/local/share/fonts"] {
            let p = std::path::Path::new(dir);
            if p.is_dir() {
                db.load_fonts_dir(p);
            }
        }
    }
}

/// Configure font families based on available fonts.
fn configure_font_families(db: &mut resvg::usvg::fontdb::Database) {
    // Collect the set of available family names once (avoids borrow conflicts).
    let available: std::collections::HashSet<String> = db
        .faces()
        .flat_map(|f| f.families.iter().map(|(name, _)| name.clone()))
        .collect();

    // Map generic CSS families to the first concrete font we find in the DB.
    configure_sans_serif_family(db, &available);
    configure_serif_family(db, &available);
    configure_monospace_family(db, &available);
}

/// Configure sans-serif font family.
fn configure_sans_serif_family(
    db: &mut resvg::usvg::fontdb::Database,
    available: &std::collections::HashSet<String>,
) {
    let sans = ["Arial", "DejaVu Sans", "Liberation Sans", "Noto Sans"];
    if let Some(f) = sans.iter().find(|n| available.contains(**n)) {
        db.set_sans_serif_family(*f);
    }
}

/// Configure serif font family.
fn configure_serif_family(
    db: &mut resvg::usvg::fontdb::Database,
    available: &std::collections::HashSet<String>,
) {
    let serif = [
        "Times New Roman",
        "DejaVu Serif",
        "Liberation Serif",
        "Noto Serif",
    ];
    if let Some(f) = serif.iter().find(|n| available.contains(**n)) {
        db.set_serif_family(*f);
    }
}

/// Configure monospace font family.
fn configure_monospace_family(
    db: &mut resvg::usvg::fontdb::Database,
    available: &std::collections::HashSet<String>,
) {
    let mono = [
        "Courier New",
        "DejaVu Sans Mono",
        "Liberation Mono",
        "Noto Sans Mono",
    ];
    if let Some(f) = mono.iter().find(|n| available.contains(**n)) {
        db.set_monospace_family(*f);
    }
}

/// Rasterization scale for SVG → PNG. Mermaid SVGs are sized in CSS pixels
/// (~96 dpi); 3x yields ~300 dpi when the figure is included at \linewidth,
/// which is print quality.
const RASTER_SCALE: f32 = 3.0;

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

    #[test]
    fn parse_opts_no_brackets_returns_empty_map() {
        let (map, rest) = parse_opts("hello");
        assert!(map.is_empty());
        assert_eq!(rest, "hello");
    }

    #[test]
    fn parse_opts_width_and_pos() {
        let (map, _) = parse_opts("[width=0.5\\linewidth, pos=t]");
        assert_eq!(map.get("width").map(String::as_str), Some("0.5\\linewidth"));
        assert_eq!(map.get("pos").map(String::as_str), Some("t"));
    }

    #[test]
    fn parse_opts_caption() {
        let (map, _) = parse_opts("[caption=My diagram]");
        assert_eq!(map.get("caption").map(String::as_str), Some("My diagram"));
    }

    #[test]
    fn parse_opts_label_and_height() {
        let (map, _) = parse_opts("[label=fig:my-diagram, height=5cm]");
        assert_eq!(map.get("label").map(String::as_str), Some("fig:my-diagram"));
        assert_eq!(map.get("height").map(String::as_str), Some("5cm"));
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
        let svg = render_graphviz(dot).unwrap();
        assert!(
            svg.contains("<svg"),
            "expected SVG output, got: {}",
            &svg[..100.min(svg.len())]
        );
    }

    #[test]
    fn render_env_no_blocks_unchanged() {
        let content = "hello world";
        let dir = tempfile::tempdir().unwrap();
        let result = render_env(content, "graphviz", dir.path(), |_| Ok(vec![])).unwrap();
        assert_eq!(result, content);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn render_env_invalid_pos_returns_error() {
        let content = "\\begin{graphviz}[pos=Z]\ndigraph G{}\n\\end{graphviz}";
        let dir = tempfile::tempdir().unwrap();
        let err = render_env(content, "graphviz", dir.path(), |_| Ok(vec![1, 2, 3])).unwrap_err();
        assert!(err.to_string().contains("pos='Z'"));
    }

    #[test]
    fn render_env_reuses_cached_diagram() {
        let content = "\\begin{graphviz}\ndigraph G{ A -> B }\n\\end{graphviz}";
        let dir = tempfile::tempdir().unwrap();
        let calls = std::cell::Cell::new(0u32);
        // render twice into the same dir — second pass must hit the cache
        for _ in 0..2 {
            render_env(content, "graphviz", dir.path(), |_| {
                calls.set(calls.get() + 1);
                Ok(vec![1, 2, 3])
            })
            .unwrap();
        }
        assert_eq!(calls.get(), 1);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
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
}
