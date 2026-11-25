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
    let removed = remove_stale_page_images(&out_dir, page)?;
    println!("  removed {removed} stale page image(s)");
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

/// Delete stale `page-<n>.png` files from `out_dir` before rasterizing.
///
/// With `page` unset (all pages), every file matching `^page-\d+\.png$` is
/// removed; with `page` set to `n`, only the files for that same page number
/// under any zero padding (`page-3.png`, `page-03.png`, `page-003.png`, ...)
/// are removed. Only the output folder itself is scanned, never recursively,
/// and nothing else is touched. Returns the number of files deleted.
fn remove_stale_page_images(out_dir: &Path, page: Option<usize>) -> Result<usize> {
    let entries = std::fs::read_dir(out_dir)
        .with_context(|| format!("failed to read {}", out_dir.display()))?;
    let mut removed = 0;
    for entry in entries {
        let entry = entry.with_context(|| format!("failed to read {}", out_dir.display()))?;
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_file() {
            continue;
        }
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let Some(digits) = name
            .strip_prefix("page-")
            .and_then(|rest| rest.strip_suffix(".png"))
        else {
            continue;
        };
        if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        if let Some(n) = page {
            let value: usize = if digits.trim_start_matches('0').is_empty() {
                0
            } else {
                digits.trim_start_matches('0').parse().unwrap_or(usize::MAX)
            };
            if value != n {
                continue;
            }
        }
        if std::fs::remove_file(entry.path()).is_ok() {
            removed += 1;
        }
    }
    Ok(removed)
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
        project_with_pdf(FIXTURE)
    }

    /// A ten-page blank US-Letter PDF built with lopdf, for stale-output tests
    /// where the zero-padded names (`page-01..page-10`) differ from the
    /// unpadded ones (`page-1.png`, `page-2.png`).
    fn ten_page_pdf() -> Vec<u8> {
        use lopdf::{dictionary, Document, Object};
        let mut doc = Document::with_version("1.7");
        let pages_id = doc.new_object_id();
        let mut kids = Vec::new();
        for _ in 0..10 {
            let page_id = doc.new_object_id();
            kids.push(Object::Reference(page_id));
            doc.objects.insert(
                page_id,
                Object::Dictionary(dictionary! {
                    "Type" => "Page",
                    "Parent" => Object::Reference(pages_id),
                    "MediaBox" => vec![
                        Object::Integer(0),
                        Object::Integer(0),
                        Object::Integer(612),
                        Object::Integer(792),
                    ],
                }),
            );
        }
        doc.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Kids" => Object::Array(kids),
                "Count" => 10,
            }),
        );
        let catalog_id = doc.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => Object::Reference(pages_id),
        });
        doc.trailer.set("Root", catalog_id);
        let mut buf = Vec::new();
        doc.save_to(&mut buf).unwrap();
        buf
    }

    /// A project whose compiled PDF is `pdf_bytes`.
    fn project_with_pdf(pdf_bytes: &[u8]) -> (tempfile::TempDir, Project) {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("project.toml"),
            "[document]\ntitle = \"Preview Doc\"\nauthor = \"A\"\ntemplate = \"general\"\n\n[build]\nentry = \"main.tex\"\n",
        )
        .unwrap();
        fs::write(dir.path().join("preview-doc.pdf"), pdf_bytes).unwrap();
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
                diagrams: None,
                highlight: None,
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
            assert!(rgba.as_chunks::<4>().0.iter().all(|px| px[3] == 255));
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
                diagrams: None,
                highlight: None,
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

    #[test]
    fn stale_page_images_removed_for_all_pages() {
        let pdf = ten_page_pdf();
        let (dir, project) = project_with_pdf(&pdf);
        let preview = dir.path().join("preview");
        fs::create_dir_all(preview.join("sub")).unwrap();
        fs::write(preview.join("page-1.png"), b"stale").unwrap();
        fs::write(preview.join("page-2.png"), b"stale").unwrap();
        fs::write(preview.join("keep.txt"), b"keep").unwrap();
        fs::write(preview.join("sub/page-1.png"), b"stale").unwrap();

        run(&project, None, 1.0, None).unwrap();

        let mut names: Vec<String> = fs::read_dir(&preview)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        let mut expected: Vec<String> = (1..=10).map(|n| format!("page-{n:02}.png")).collect();
        expected.push("keep.txt".to_string());
        expected.push("sub".to_string());
        expected.sort();
        assert_eq!(names, expected);
        assert_eq!(fs::read(preview.join("keep.txt")).unwrap(), b"keep");
        assert!(preview.join("sub/page-1.png").exists());
        for n in 1..=10 {
            let (w, h, _) = decode(&preview.join(format!("page-{n:02}.png")));
            assert_eq!((w, h), (612, 792), "page-{n:02}.png");
        }
    }

    #[test]
    fn page_flag_removes_only_that_page() {
        let pdf = ten_page_pdf();
        let (dir, project) = project_with_pdf(&pdf);
        let preview = dir.path().join("preview");
        fs::create_dir_all(&preview).unwrap();
        fs::write(preview.join("page-3.png"), b"stale3").unwrap();
        fs::write(preview.join("page-003.png"), b"stale003").unwrap();
        fs::write(preview.join("page-4.png"), b"keep4").unwrap();

        run(&project, Some(3), 1.0, None).unwrap();

        assert!(!preview.join("page-3.png").exists());
        assert!(!preview.join("page-003.png").exists());
        assert_eq!(fs::read(preview.join("page-4.png")).unwrap(), b"keep4");
        let (w, h, _) = decode(&preview.join("page-03.png"));
        assert_eq!((w, h), (612, 792));
    }
}
