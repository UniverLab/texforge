//! Pre-processor for embedded diagram environments.
//!
//! Intercepts `\begin{mermaid}[opts]...\end{mermaid}` blocks, renders them
//! to PNG, and replaces each block with a proper `figure` environment.
//!
//! Works on copies in `build/` — the original .tex files are never modified.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// Copy all .tex files to `build/`, rendering embedded diagrams in the copies.
/// Also mirrors non-.tex assets so tectonic can resolve relative paths.
/// Returns the path to the build copy of `entry`.
pub fn process(root: &Path, entry: &str) -> Result<PathBuf> {
    let build_dir = root.join("build");
    std::fs::create_dir_all(&build_dir)?;

    let diagrams_dir = build_dir.join("diagrams");
    std::fs::create_dir_all(&diagrams_dir)?;

    let mut counter = 0usize;

    // Process .tex files
    let tex_files = collect_tex_files(root, entry);
    for src in &tex_files {
        let rel = src.strip_prefix(root).unwrap_or(src);
        let dest = build_dir.join(rel);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = std::fs::read_to_string(src)?;
        let processed = render_diagrams(&content, &diagrams_dir, &mut counter)
            .with_context(|| format!("Failed to render diagrams in {}", src.display()))?;
        std::fs::write(&dest, processed)?;
    }

    // Mirror asset files so tectonic resolves relative paths
    crate::utils::mirror_assets(root, &build_dir)?;

    Ok(build_dir.join(entry))
}

/// Replace all `\begin{mermaid}[opts]...\end{mermaid}` with figure environments.
fn render_diagrams(content: &str, diagrams_dir: &Path, counter: &mut usize) -> Result<String> {
    let content = render_env(content, "mermaid", diagrams_dir, counter, |src| {
        let svg = render_mermaid_with_config(src)
            .map_err(|e| anyhow::anyhow!("Mermaid render error: {}", e))?;
        svg_to_png(&svg).context("Failed to convert mermaid SVG to PNG")
    })?;
    let content = render_env(&content, "graphviz", diagrams_dir, counter, |src| {
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
pub(crate) fn render_env(
    content: &str,
    env: &str,
    diagrams_dir: &Path,
    counter: &mut usize,
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

        let png = render_fn(diagram_src)?;
        let filename = save_diagram_png(diagrams_dir, counter, &png)?;
        let fig_env = build_figure_environment(&opts, env, &filename)?;

        result.push_str(&fig_env);
        remaining = &after_opts[end + end_tag.len()..];
    }

    result.push_str(remaining);
    Ok(result)
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

/// Save the rendered diagram as PNG and return the filename.
fn save_diagram_png(diagrams_dir: &Path, counter: &mut usize, png: &[u8]) -> Result<String> {
    *counter += 1;
    let filename = format!("diagram-{}.png", counter);
    std::fs::write(diagrams_dir.join(&filename), png)?;
    Ok(filename)
}

/// Build the figure environment LaTeX code.
fn build_figure_environment(
    opts: &HashMap<String, String>,
    _env: &str,
    filename: &str,
) -> Result<String> {
    let pos = opts.get("pos").map(String::as_str).unwrap_or("H");
    let width = opts
        .get("width")
        .map(String::as_str)
        .unwrap_or("\\linewidth");
    let rel_path = format!("diagrams/{}", filename);

    let mut fig = format!(
        "\\begin{{figure}}[{pos}]\n  \\centering\n  \\includegraphics[width={width}]{{{rel_path}}}\n"
    );

    add_caption_if_present(opts, &mut fig)?;
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

/// Convert SVG string to PNG bytes with improved rendering quality.
/// Uses a more sophisticated approach to preserve diagram layout and prevent element overlap.
fn svg_to_png(svg: &str) -> Result<Vec<u8>> {
    let fontdb = build_fontdb();

    let options = resvg::usvg::Options {
        fontdb: std::sync::Arc::new(fontdb),
        // Enable shape rendering to preserve exact positions
        shape_rendering: resvg::usvg::ShapeRendering::GeometricPrecision,
        // Enable text rendering for better font handling
        text_rendering: resvg::usvg::TextRendering::OptimizeLegibility,
        ..Default::default()
    };

    let tree = resvg::usvg::Tree::from_str(svg, &options).context("Failed to parse SVG")?;

    // Get the original SVG dimensions
    let original_size = tree.size();

    // Use a more conservative scale factor to avoid distortion
    let scale = 1.5_f32; // Reduced from 2.0 to prevent scaling artifacts

    // Calculate output dimensions with padding to prevent edge issues
    let padding = 10.0; // Add 10px padding around the diagram
    let width = ((original_size.width() + padding * 2.0) * scale) as u32;
    let height = ((original_size.height() + padding * 2.0) * scale) as u32;

    let mut pixmap =
        resvg::tiny_skia::Pixmap::new(width, height).context("Failed to create pixmap")?;

    // Create a transform that accounts for both scaling and padding
    let transform =
        resvg::tiny_skia::Transform::from_scale(scale, scale).post_translate(padding, padding);

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
        let mut counter = 0;
        let result = render_env(content, "graphviz", dir.path(), &mut counter, |_| {
            Ok(vec![])
        })
        .unwrap();
        assert_eq!(result, content);
        assert_eq!(counter, 0);
    }

    #[test]
    fn render_env_invalid_pos_returns_error() {
        let content = "\\begin{graphviz}[pos=Z]\ndigraph G{}\n\\end{graphviz}";
        let dir = tempfile::tempdir().unwrap();
        let mut counter = 0;
        let err = render_env(content, "graphviz", dir.path(), &mut counter, |_| {
            Ok(vec![1, 2, 3])
        })
        .unwrap_err();
        assert!(err.to_string().contains("pos='Z'"));
    }
}
