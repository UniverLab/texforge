//! `texforge preview` command implementation.
//!
//! Rasterizes the compiled PDF into PNG pages with the pure-Rust rasterizer
//! and prints per-page ink coverage, so blank, nearly-blank and overfull pages
//! surface without opening the document.

use std::io::BufWriter;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use crate::domain::project::Project;
use crate::raster::PdfDocument;
use crate::utils::sanitize_filename;

/// Rasterize the compiled PDF to PNG pages.
pub fn execute(page: Option<usize>, scale: f32, out: Option<PathBuf>) -> Result<()> {
    let project = Project::load()?;
    execute_for_project(&project, page, scale, out)
}

/// The work behind [`execute`], taking a project directly so tests can run it
/// without touching the process-global current directory.
fn execute_for_project(
    project: &Project,
    page: Option<usize>,
    scale: f32,
    out: Option<PathBuf>,
) -> Result<()> {
    if !scale.is_finite() || scale <= 0.0 {
        bail!("--scale must be a finite positive number");
    }

    let pdf_path = project.root.join(format!(
        "{}.pdf",
        sanitize_filename(&project.config.document.title)
    ));
    if !pdf_path.exists() {
        bail!(
            "No compiled PDF at {} — run `texforge build` first",
            pdf_path.display()
        );
    }

    let out_dir = match out {
        Some(dir) if dir.is_absolute() => dir,
        Some(dir) => project.root.join(dir),
        None => project.root.join("preview"),
    };
    std::fs::create_dir_all(&out_dir)
        .with_context(|| format!("failed to create {}", out_dir.display()))?;

    let document = PdfDocument::open(&pdf_path)?;
    let pages = document.page_count();
    if pages == 0 {
        bail!("{} has no pages", pdf_path.display());
    }

    let page_indexes = match page {
        Some(n) if n == 0 || n > pages => {
            bail!("--page {n} is out of range (document has {pages} pages)");
        }
        Some(n) => vec![n - 1],
        None => (0..pages).collect(),
    };

    println!(
        "Rasterizing {} page(s) at {scale:.2}x -> {}",
        page_indexes.len(),
        out_dir.display()
    );
    let width_digits = pages.to_string().len();
    for index in page_indexes {
        let rendered = document.render_page(index, scale)?;
        let name = format!(
            "page-{:0width$}.png",
            rendered.index + 1,
            width = width_digits
        );
        let path = out_dir.join(&name);
        write_png(&path, rendered.width, rendered.height, &rendered.rgba)?;
        println!(
            "  {name}  {}x{}  ink {:.1}%",
            rendered.width,
            rendered.height,
            rendered.ink_coverage * 100.0
        );
    }

    Ok(())
}

/// Write `rgba` to `path` as a PNG.
fn write_png(path: &Path, width: usize, height: usize, rgba: &[u8]) -> Result<()> {
    let file = std::fs::File::create(path)
        .with_context(|| format!("failed to create {}", path.display()))?;
    let mut encoder = png::Encoder::new(BufWriter::new(file), width as u32, height as u32);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder
        .write_header()
        .with_context(|| format!("failed to encode {}", path.display()))?;
    writer
        .write_image_data(rgba)
        .with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::domain::project::{BuildConfig, DocumentConfig, ProjectConfig};

    const FIXTURE: &[u8] = include_bytes!("../../tests/fixtures/two-page.pdf");

    /// A project whose compiled PDF is the committed two-page fixture. The
    /// title sanitizes to `preview-doc`, so the PDF must be named accordingly.
    fn project() -> (tempfile::TempDir, Project) {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("project.toml"),
            "[document]\ntitle = \"Preview Doc\"\nauthor = \"A\"\ntemplate = \"general\"\n\n[build]\nentry = \"main.tex\"\n",
        )
        .unwrap();
        fs::write(dir.path().join("preview-doc.pdf"), FIXTURE).unwrap();
        let project = Project {
            root: dir.path().to_path_buf(),
            config: ProjectConfig {
                document: DocumentConfig {
                    title: "Preview Doc".to_string(),
                    author: "A".to_string(),
                    template: "general".to_string(),
                },
                build: BuildConfig {
                    entry: "main.tex".to_string(),
                    bibliography: None,
                    reproducible: None,
                },
            },
        };
        (dir, project)
    }

    /// Run the command with an injected project, avoiding process-global
    /// current-directory state in parallel tests.
    fn run(project: &Project, page: Option<usize>, scale: f32, out: Option<PathBuf>) -> Result<()> {
        execute_for_project(project, page, scale, out)
    }

    /// Decode a PNG and return `(width, height, rgba)`.
    fn decode(path: &Path) -> (u32, u32, Vec<u8>) {
        let file = fs::File::open(path).unwrap();
        let decoder = png::Decoder::new(file);
        let mut reader = decoder.read_info().unwrap();
        let mut buf = vec![0u8; reader.output_buffer_size()];
        let info = reader.next_frame(&mut buf).unwrap();
        let (w, h) = (info.width, info.height);
        buf.truncate((w * h * 4) as usize);
        (w, h, buf)
    }

    #[test]
    fn writes_pngs_for_all_pages() {
        let (dir, project) = project();
        run(&project, None, 1.0, None).unwrap();
        for name in ["page-1.png", "page-2.png"] {
            let path = dir.path().join("preview").join(name);
            let (w, h, rgba) = decode(&path);
            assert_eq!((w, h), (612, 842), "{name}");
            assert!(rgba.chunks_exact(4).all(|px| px[3] == 255));
        }
    }

    #[test]
    fn single_page_flag_limits_output() {
        let (dir, project) = project();
        run(&project, Some(1), 1.0, None).unwrap();
        let preview = dir.path().join("preview");
        assert!(preview.join("page-1.png").exists());
        assert!(!preview.join("page-2.png").exists());
    }

    #[test]
    fn custom_out_dir_is_used() {
        let (dir, project) = project();
        let out = dir.path().join("renders");
        run(&project, None, 1.0, Some(out.clone())).unwrap();
        assert!(out.join("page-1.png").exists());
        assert!(!dir.path().join("preview").exists());
    }

    #[test]
    fn scale_flag_resizes_the_output() {
        let (dir, project) = project();
        run(&project, Some(1), 0.5, None).unwrap();
        let (w, h, _) = decode(&dir.path().join("preview/page-1.png"));
        assert_eq!((w, h), (306, 421));
    }

    #[test]
    fn missing_pdf_is_a_helpful_error() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("project.toml"),
            "[document]\ntitle = \"Preview Doc\"\nauthor = \"A\"\ntemplate = \"general\"\n\n[build]\nentry = \"main.tex\"\n",
        )
        .unwrap();
        let project = Project {
            root: dir.path().to_path_buf(),
            config: ProjectConfig {
                document: DocumentConfig {
                    title: "Preview Doc".to_string(),
                    author: "A".to_string(),
                    template: "general".to_string(),
                },
                build: BuildConfig {
                    entry: "main.tex".to_string(),
                    bibliography: None,
                    reproducible: None,
                },
            },
        };
        let err = run(&project, None, 1.0, None).unwrap_err();
        assert!(err.to_string().contains("run `texforge build` first"));
    }

    #[test]
    fn out_of_range_page_errors() {
        let (_dir, project) = project();
        let err = run(&project, Some(9), 1.0, None).unwrap_err();
        assert!(err.to_string().contains("out of range"));
    }

    #[test]
    fn non_positive_scale_errors() {
        let (_dir, project) = project();
        let err = run(&project, Some(1), 0.0, None).unwrap_err();
        assert!(err.to_string().contains("finite positive"));
    }
}
