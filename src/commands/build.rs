//! `texforge build` command implementation.

use std::collections::HashMap;
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use anyhow::{Context, Result};
use notify::{RecursiveMode, Watcher};

use crate::compiler;
use crate::diagrams;
use crate::domain::project::{Project, Reproducible};
use crate::highlight;
use crate::highlight::{HighlightStyle, HighlightTheme, ListingFont};
use crate::raster::PdfDocument;
use crate::utils::sanitize_filename;

/// Options for writing a stable live-preview PNG after each successful watch rebuild.
pub struct LivePreview {
    /// 1-based page number to rasterize.
    pub page: usize,
    /// Fixed output path; relative paths resolve against the project root.
    /// `None` means `preview.png` in the project root.
    pub out: Option<PathBuf>,
}

/// Resolve the `SOURCE_DATE_EPOCH` value for a build. The CLI flag wins over
/// `project.toml`; a flag with no value pins the fixed default epoch; a config
/// value pins its own epoch or the default when enabled without one.
pub(crate) fn resolve_epoch(cli: Option<Option<u64>>, config: Option<Reproducible>) -> Option<u64> {
    match cli {
        Some(Some(epoch)) => Some(epoch),
        Some(None) => Some(compiler::DEFAULT_EPOCH),
        None => match config {
            Some(Reproducible::Enabled(true)) => Some(compiler::DEFAULT_EPOCH),
            Some(Reproducible::Epoch(epoch)) => Some(epoch),
            Some(Reproducible::Enabled(false)) | None => None,
        },
    }
}

/// Resolve the document-wide default diagram style from `project.toml`'s
/// `[diagrams] style` key. A `style=` on the environment itself overrides
/// this. An unrecognised name fails the build rather than silently falling
/// back to `default`.
fn resolve_default_style(project: &Project) -> Result<diagrams::style::DiagramStyle> {
    match project
        .config
        .diagrams
        .as_ref()
        .and_then(|d| d.style.as_deref())
    {
        Some(name) => diagrams::style::DiagramStyle::parse(name),
        None => Ok(diagrams::style::DiagramStyle::Default),
    }
}

/// Resolve `project.toml`'s `[highlight]` section into the code-listing
/// pass's settings, mirroring [`resolve_default_style`]: an absent section
/// keeps every default (github theme, light style, no `lstlisting` rewrite, no
/// gutter), and an unrecognised theme, style, or font fails the build by name
/// instead of silently falling back to a default.
fn resolve_highlight(project: &Project) -> Result<highlight::Settings> {
    let Some(section) = project.config.highlight.as_ref() else {
        return Ok(highlight::Settings::default());
    };
    Ok(highlight::Settings {
        theme: match section.theme.as_deref() {
            Some(name) => HighlightTheme::parse(name)?,
            None => HighlightTheme::default(),
        },
        style: match section.style.as_deref() {
            Some(name) => HighlightStyle::parse(name)?,
            None => HighlightStyle::default(),
        },
        by_lang: resolve_by_lang(&section.by_lang)?,
        lstlisting: section.lstlisting.unwrap_or(false),
        numbers: section.numbers.unwrap_or(false),
        caption_name: section.caption_name.clone(),
        list_name: section.list_name.clone(),
        font: match section.font.as_deref() {
            Some(name) => Some(ListingFont::parse(name)?),
            None => None,
        },
        fallback_language: None,
    })
}

/// Parse `[highlight.by_lang]`, normalising each key with the same
/// [`highlight::language_key`] a block's `lang=` value goes through — that is
/// what makes one entry match every alias of its language. An unknown style
/// value fails the build naming the offending value.
fn resolve_by_lang(table: &HashMap<String, String>) -> Result<HashMap<String, HighlightStyle>> {
    let mut resolved = HashMap::with_capacity(table.len());
    for (lang, style) in table {
        resolved.insert(highlight::language_key(lang), HighlightStyle::parse(style)?);
    }
    Ok(resolved)
}

/// Like [`resolve_highlight`], but also fills `fallback_language` from the
/// global `~/.texforge/config.toml` `defaults.language` — the same source
/// the spell checker uses.
fn resolve_highlight_settings(project: &Project) -> Result<highlight::Settings> {
    let mut settings = resolve_highlight(project)?;
    settings.fallback_language = crate::config::load().ok().and_then(|c| c.defaults.language);
    Ok(settings)
}

/// Default engine-log path for a project build: `build/<entry-stem>.log`
/// under the project root.
pub(crate) fn default_log_path(project: &Project) -> PathBuf {
    let stem = Path::new(&project.config.build.entry)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "main".to_string());
    project.root.join("build").join(format!("{stem}.log"))
}

/// Resolve `--log`: an explicit path wins (relative paths resolve against
/// the current directory), otherwise the caller's default applies.
pub(crate) fn resolve_log_path(log: Option<&Path>, default: PathBuf) -> PathBuf {
    match log {
        Some(path) if path.is_absolute() => path.to_path_buf(),
        Some(path) => std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path),
        None => default,
    }
}

/// Print the overfull-box report after the existing warning output. Silent
/// when no box exceeds 1pt, so clean builds print exactly what they print
/// today.
fn print_overfull(warnings: &[compiler::CompileWarning], verbose: bool) {
    let out = compiler::log::format_overfull(warnings, verbose);
    if !out.is_empty() {
        print!("{out}");
    }
}

/// Fail when any overfull box exceeds `--max-overfull`. Callers run this
/// after the PDF is written, so the PDF survives the non-zero exit.
pub(crate) fn enforce_max_overfull(
    warnings: &[compiler::CompileWarning],
    max_overfull: Option<f32>,
) -> Result<()> {
    if let Some(max) = max_overfull {
        if let Some(worst) = compiler::log::exceeds_max(warnings, max) {
            anyhow::bail!("{}", compiler::log::overfull_gate_message(worst, max));
        }
    }
    Ok(())
}

/// Run the diagrams + highlight + Tectonic pipeline for `project` with its
/// sources copied into `build_dir`, returning the built PDF inside
/// `build_dir` with its warnings and the engine's complete log text. Shared
/// by the normal build and the template-folder build, which stages a
/// rendered copy and points a [`Project`] at it.
///
/// The log is written to `log_path` after every compile, successful or not;
/// the overfull-box report prints after the existing warning output.
pub(crate) fn compile_project(
    project: &Project,
    build_dir: &Path,
    verbose: bool,
    epoch: Option<u64>,
    log_path: &Path,
) -> Result<(PathBuf, Vec<compiler::CompileWarning>, String)> {
    let default_style = resolve_default_style(project)?;
    let highlight_cfg = resolve_highlight_settings(project)?;

    diagrams::process(
        &project.root,
        &project.config.build.entry,
        build_dir,
        default_style,
    )?;
    // After diagrams: the code pass rewrites what diagrams just copied, and
    // its warning line numbers are build-copy coordinates (like Tectonic's).
    let line_map = highlight::process(
        &project.root,
        build_dir,
        &project.config.build.entry,
        highlight_cfg,
    )?;
    let entry_filename = Path::new(&project.config.build.entry)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| project.config.build.entry.clone());
    let (warnings, raw) =
        match compiler::compile(build_dir, &entry_filename, verbose, epoch, &line_map) {
            Ok(ok) => ok,
            Err(e) => {
                if let Some(failure) = e.downcast_ref::<compiler::CompileFailure>() {
                    compiler::log::write_log(log_path, &failure.raw)?;
                }
                return Err(e);
            }
        };
    compiler::log::write_log(log_path, &raw)?;
    print_overfull(&warnings, verbose);

    Ok((
        build_dir.join(
            Path::new(&project.config.build.entry)
                .with_extension("pdf")
                .file_name()
                .unwrap(),
        ),
        warnings,
        raw,
    ))
}

/// Compile project to PDF using a temp directory, output named after the document title.
pub fn execute(
    verbose: bool,
    reproducible: Option<Option<u64>>,
    log: Option<&Path>,
    max_overfull: Option<f32>,
) -> Result<()> {
    if !Path::new("project.toml").exists() && Path::new("template.toml").exists() {
        anyhow::bail!(
            "This folder is a template: `texforge build --values <file>` is needed \
             to build a template folder"
        );
    }
    let project = Project::load()?;
    let titulo = project.config.document.title.clone();
    println!("Building project: {titulo}");

    let epoch = resolve_epoch(reproducible, project.config.build.reproducible);
    if epoch.is_some() {
        println!("  ◇ reproducible build (SOURCE_DATE_EPOCH pinned)");
    }

    let temp_dir = tempfile::tempdir()?;
    let build_dir = temp_dir.path();
    println!("  ◇ temp: {}", build_dir.display());

    let log_path = resolve_log_path(log, default_log_path(&project));
    let (pdf_src, warnings, _raw) =
        compile_project(&project, build_dir, verbose, epoch, &log_path)?;

    let pdf_name = format!("{}.pdf", sanitize_filename(&titulo));
    let pdf_dest = project.root.join(&pdf_name);
    std::fs::copy(&pdf_src, &pdf_dest)?;
    println!("  ◇ {}", pdf_dest.display());

    enforce_max_overfull(&warnings, max_overfull)?;

    Ok(())
}

/// Watch for .tex file changes and rebuild with debounce.
pub fn watch(
    delay_secs: u64,
    verbose: bool,
    reproducible: Option<Option<u64>>,
    live_preview: Option<&LivePreview>,
    log: Option<&Path>,
    max_overfull: Option<f32>,
) -> Result<()> {
    let project = Project::load()?;
    let epoch = resolve_epoch(reproducible, project.config.build.reproducible);
    let debounce = Duration::from_secs(delay_secs);
    let cooldown = Duration::from_secs(2);
    let preview_path =
        live_preview.map(|opts| resolve_preview_path(&project.root, opts.out.as_deref()));

    let status_row = print_watch_header(
        &project.config.document.title,
        delay_secs,
        preview_path.as_deref(),
    );

    let temp_dir = tempfile::tempdir()?;
    let build_dir = temp_dir.path().to_path_buf();

    let started = std::time::Instant::now();
    let result = run_build_with_preview(
        &project,
        &build_dir,
        verbose,
        epoch,
        live_preview,
        log,
        max_overfull,
    );
    redraw_status(&result, 1, started, status_row);

    let (tx, rx) = mpsc::channel();
    let mut watcher = notify::recommended_watcher(move |res| {
        if let Ok(event) = res {
            let _ = tx.send(event);
        }
    })?;

    watcher.watch(&project.root, RecursiveMode::Recursive)?;

    let mut pending = false;
    let mut last_event = std::time::Instant::now();
    let mut last_build = std::time::Instant::now();
    let mut build_count = 1u32;
    let mut last_result = result;
    let mut last_tick = std::time::Instant::now();

    loop {
        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(event) => {
                if should_rebuild(
                    is_relevant_watch_event(&event.paths, &build_dir),
                    last_build.elapsed(),
                    cooldown,
                ) {
                    pending = true;
                    last_event = std::time::Instant::now();
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(_) => break,
        }

        if last_tick.elapsed() >= Duration::from_secs(1) {
            last_tick = std::time::Instant::now();
            redraw_status(&last_result, build_count, started, status_row);
        }

        if pending && last_event.elapsed() >= debounce {
            pending = false;
            build_count += 1;
            last_result = run_build_with_preview(
                &project,
                &build_dir,
                verbose,
                epoch,
                live_preview,
                log,
                max_overfull,
            );
            last_build = std::time::Instant::now();
            redraw_status(&last_result, build_count, started, status_row);
        }
    }

    Ok(())
}

/// Whether the watch loop rebuilds now: the event touched a source and the
/// post-build cooldown has passed. Split from the loop so the two halves —
/// *which* paths matter and *when* a rebuild may fire — are unit-testable;
/// the loop itself only wires OS file events to the terminal.
fn should_rebuild(relevant: bool, elapsed: Duration, cooldown: Duration) -> bool {
    relevant && elapsed > cooldown
}

/// Whether a debounced watch event should trigger a rebuild: at least one
/// changed path is a `.tex` source outside the temporary build directory
/// (auxiliary, PDF and preview outputs must never retrigger the loop).
fn is_relevant_watch_event(paths: &[PathBuf], build_dir: &Path) -> bool {
    paths
        .iter()
        .any(|p| !p.starts_with(build_dir) && p.extension().and_then(|e| e.to_str()) == Some("tex"))
}

/// Draw the banner and title, and return the screen row the status block is
/// drawn on: one below the title, whichever font the banner picked.
fn print_watch_header(title: &str, delay_secs: u64, preview: Option<&Path>) -> usize {
    print!("\x1B[2J\x1B[H");
    let status_row = crate::banner::print() + 2;
    println!("  {title} — watching  ({delay_secs}s debounce  Ctrl+C to stop)");
    if let Some(path) = preview {
        println!("  live preview → {}", path.display());
    }
    status_row
}

fn redraw_status(
    result: &WatchResult,
    build_count: u32,
    started: std::time::Instant,
    status_row: usize,
) {
    print!("\x1B[{status_row};0H\x1B[J");
    let e = started.elapsed().as_secs();
    let session = format!("{:02}:{:02}:{:02}", e / 3600, (e % 3600) / 60, e % 60);
    println!();
    println!("  session  \x1B[36m{session}\x1B[0m   builds  \x1B[36m{build_count}\x1B[0m");
    println!();
    match result {
        WatchResult::Ok(pdf) => println!("  \x1B[32m{pdf}  ok\x1B[0m"),
        WatchResult::Err(err) => {
            println!("  \x1B[31merror:\x1B[0m");
            for line in err.lines() {
                println!("    {line}");
            }
        }
    }
    use std::io::Write;
    let _ = std::io::stdout().flush();
}

enum WatchResult {
    Ok(String),
    Err(String),
}

fn run_build(
    project: &Project,
    build_dir: &Path,
    verbose: bool,
    epoch: Option<u64>,
    log: Option<&Path>,
    max_overfull: Option<f32>,
) -> WatchResult {
    let _ = std::fs::create_dir_all(build_dir);
    let default_style = match resolve_default_style(project) {
        Ok(style) => style,
        Err(e) => return WatchResult::Err(e.to_string()),
    };
    let highlight_cfg = match resolve_highlight_settings(project) {
        Ok(cfg) => cfg,
        Err(e) => return WatchResult::Err(e.to_string()),
    };
    if let Err(e) = diagrams::process(
        &project.root,
        &project.config.build.entry,
        build_dir,
        default_style,
    ) {
        return WatchResult::Err(e.to_string());
    }
    let line_map = match highlight::process(
        &project.root,
        build_dir,
        &project.config.build.entry,
        highlight_cfg,
    ) {
        Ok(map) => map,
        Err(e) => return WatchResult::Err(e.to_string()),
    };
    let entry_filename = Path::new(&project.config.build.entry)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| project.config.build.entry.clone());
    let log_path = resolve_log_path(log, default_log_path(project));
    let (warnings, raw) =
        match compiler::compile(build_dir, &entry_filename, verbose, epoch, &line_map) {
            Ok(ok) => ok,
            Err(e) => {
                if let Some(failure) = e.downcast_ref::<compiler::CompileFailure>() {
                    if let Err(e) = compiler::log::write_log(&log_path, &failure.raw) {
                        return WatchResult::Err(e.to_string());
                    }
                }
                return WatchResult::Err(e.to_string());
            }
        };
    if let Err(e) = compiler::log::write_log(&log_path, &raw) {
        return WatchResult::Err(e.to_string());
    }
    print_overfull(&warnings, verbose);
    let pdf_name = format!("{}.pdf", sanitize_filename(&project.config.document.title));
    let pdf_dest = project.root.join(&pdf_name);
    let pdf_src = build_dir.join(
        Path::new(&project.config.build.entry)
            .with_extension("pdf")
            .file_name()
            .unwrap(),
    );
    match std::fs::copy(&pdf_src, &pdf_dest) {
        Ok(_) => match enforce_max_overfull(&warnings, max_overfull) {
            Ok(()) => WatchResult::Ok(pdf_name),
            Err(e) => WatchResult::Err(e.to_string()),
        },
        Err(e) => WatchResult::Err(e.to_string()),
    }
}

/// Rebuild, then on success write the live-preview PNG. A failed rebuild
/// leaves any previous preview image untouched.
fn run_build_with_preview(
    project: &Project,
    build_dir: &Path,
    verbose: bool,
    epoch: Option<u64>,
    live_preview: Option<&LivePreview>,
    log: Option<&Path>,
    max_overfull: Option<f32>,
) -> WatchResult {
    let result = run_build(project, build_dir, verbose, epoch, log, max_overfull);
    if let (WatchResult::Ok(ref pdf_name), Some(opts)) = (&result, live_preview) {
        let pdf_path = project.root.join(pdf_name);
        let out = resolve_preview_path(&project.root, opts.out.as_deref());
        // Rasterize failures must not erase the last good frame.
        let _ = write_live_preview(&pdf_path, opts.page, &out);
    }
    result
}

fn resolve_preview_path(project_root: &Path, out: Option<&Path>) -> PathBuf {
    match out {
        Some(path) if path.is_absolute() => path.to_path_buf(),
        Some(path) => project_root.join(path),
        None => project_root.join("preview.png"),
    }
}

/// Sidecar path used while encoding; renamed onto `target` only when complete.
fn preview_temp_path(target: &Path) -> PathBuf {
    let mut tmp = target.as_os_str().to_owned();
    tmp.push(".tmp");
    PathBuf::from(tmp)
}

/// Rasterize one PDF page to `out` via temp file + rename (atomic on the same FS).
fn write_live_preview(pdf_path: &Path, page: usize, out: &Path) -> Result<()> {
    if page == 0 {
        anyhow::bail!("--preview-page must be at least 1");
    }
    let document = PdfDocument::open(pdf_path)?;
    let pages = document.page_count();
    if page > pages {
        anyhow::bail!("--preview-page {page} is out of range (document has {pages} pages)");
    }
    let rendered = document.render_page(page - 1, 1.0)?;
    write_png_atomic(out, rendered.width, rendered.height, &rendered.rgba)
}

/// Encode a PNG to a sibling `.tmp` file, then rename over `path` so viewers
/// never observe a half-written image.
fn write_png_atomic(path: &Path, width: usize, height: usize, rgba: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
    }
    let tmp = preview_temp_path(path);
    // Drop any leftover temp from a previous interrupted write before starting.
    let _ = std::fs::remove_file(&tmp);
    write_png(&tmp, width, height, rgba).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })?;
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e).with_context(|| format!("failed to publish {}", path.display()));
    }
    Ok(())
}

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
    use super::*;

    fn ensure_rustls() {
        let _ = rustls::crypto::ring::default_provider().install_default();
    }

    #[test]
    fn flag_without_value_wins_with_default_epoch() {
        assert_eq!(
            resolve_epoch(Some(None), Some(Reproducible::Epoch(123))),
            Some(compiler::DEFAULT_EPOCH)
        );
    }

    #[test]
    fn flag_with_value_wins_over_config() {
        assert_eq!(
            resolve_epoch(Some(Some(123)), Some(Reproducible::Enabled(true))),
            Some(123)
        );
    }

    #[test]
    fn config_enabled_uses_default_epoch() {
        assert_eq!(
            resolve_epoch(None, Some(Reproducible::Enabled(true))),
            Some(compiler::DEFAULT_EPOCH)
        );
    }

    #[test]
    fn config_explicit_epoch_used_when_no_flag() {
        assert_eq!(
            resolve_epoch(None, Some(Reproducible::Epoch(1700000000))),
            Some(1700000000)
        );
    }

    #[test]
    fn config_disabled_or_absent_is_off() {
        assert_eq!(
            resolve_epoch(None, Some(Reproducible::Enabled(false))),
            None
        );
        assert_eq!(resolve_epoch(None, None), None);
    }

    #[test]
    fn watch_event_for_tex_source_outside_build_dir_triggers_rebuild() {
        let build_dir = Path::new("/tmp/texforge-build-xyz");
        assert!(is_relevant_watch_event(
            &[PathBuf::from("/proj/main.tex")],
            build_dir
        ));
    }

    #[test]
    fn watch_event_for_aux_output_or_build_dir_path_is_ignored() {
        let build_dir = PathBuf::from("/tmp/texforge-build-xyz");
        assert!(!is_relevant_watch_event(
            &[PathBuf::from("/proj/main.aux")],
            &build_dir
        ));
        assert!(!is_relevant_watch_event(
            &[PathBuf::from("/proj/main.pdf")],
            &build_dir
        ));
        assert!(!is_relevant_watch_event(
            &[build_dir.join("main.tex")],
            &build_dir
        ));
        assert!(!is_relevant_watch_event(&[], &build_dir));
    }

    #[test]
    fn rebuild_fires_only_for_a_source_after_the_cooldown() {
        let cooldown = Duration::from_secs(2);
        assert!(should_rebuild(true, Duration::from_secs(3), cooldown));
        // The cooldown is strict: exactly at the boundary the previous
        // build still owns the terminal.
        assert!(!should_rebuild(true, Duration::from_secs(2), cooldown));
        assert!(!should_rebuild(true, Duration::from_secs(1), cooldown));
        assert!(!should_rebuild(false, Duration::from_secs(3), cooldown));
        assert!(!should_rebuild(false, Duration::from_secs(0), cooldown));
    }

    fn project_with_diagrams_style(style: Option<&str>) -> Project {
        Project {
            root: PathBuf::from("."),
            config: crate::domain::project::ProjectConfig {
                document: crate::domain::project::DocumentConfig {
                    title: "T".to_string(),
                    author: "A".to_string(),
                    template: "general".to_string(),
                },
                build: crate::domain::project::BuildConfig {
                    entry: "main.tex".to_string(),
                    bibliography: None,
                    reproducible: None,
                },
                diagrams: style.map(|s| crate::domain::project::DiagramsConfig {
                    style: Some(s.to_string()),
                }),
                highlight: None,
            },
        }
    }

    #[test]
    fn default_style_is_used_when_project_toml_has_none() {
        let project = project_with_diagrams_style(None);
        assert_eq!(
            resolve_default_style(&project).unwrap(),
            diagrams::style::DiagramStyle::Default
        );
    }

    #[test]
    fn project_toml_style_becomes_the_default() {
        let project = project_with_diagrams_style(Some("editorial"));
        assert_eq!(
            resolve_default_style(&project).unwrap(),
            diagrams::style::DiagramStyle::Editorial
        );
    }

    #[test]
    fn invalid_project_toml_style_fails() {
        let project = project_with_diagrams_style(Some("editoral"));
        let err = resolve_default_style(&project).unwrap_err();
        assert!(err.to_string().contains("editoral"));
    }

    fn project_with_highlight(
        theme: Option<&str>,
        lstlisting: Option<bool>,
        numbers: Option<bool>,
        caption_name: Option<&str>,
        list_name: Option<&str>,
    ) -> Project {
        let mut project = project_with_diagrams_style(None);
        project.config.highlight = Some(crate::domain::project::HighlightConfig {
            theme: theme.map(str::to_string),
            lstlisting,
            numbers,
            caption_name: caption_name.map(str::to_string),
            list_name: list_name.map(str::to_string),
            ..Default::default()
        });
        project
    }

    /// A project whose `[highlight]` section carries the two style keys —
    /// the document-wide `style` and a `[highlight.by_lang]` table.
    fn project_with_highlight_styles(style: Option<&str>, by_lang: &[(&str, &str)]) -> Project {
        let mut project = project_with_diagrams_style(None);
        project.config.highlight = Some(crate::domain::project::HighlightConfig {
            style: style.map(str::to_string),
            by_lang: by_lang
                .iter()
                .map(|(lang, value)| ((*lang).to_string(), (*value).to_string()))
                .collect(),
            ..Default::default()
        });
        project
    }

    #[test]
    fn highlight_defaults_when_the_section_is_absent() {
        let project = project_with_diagrams_style(None);
        let settings = resolve_highlight(&project).unwrap();
        assert_eq!(settings, highlight::Settings::default());
        assert_eq!(settings.theme, HighlightTheme::Github);
        assert!(!settings.lstlisting);
        assert!(!settings.numbers);
    }

    #[test]
    fn highlight_section_is_honoured() {
        let project = project_with_highlight(Some("one-light"), Some(true), Some(true), None, None);
        let settings = resolve_highlight(&project).unwrap();
        assert_eq!(settings.theme, HighlightTheme::OneLight);
        assert!(settings.lstlisting);
        assert!(settings.numbers);
    }

    #[test]
    fn partial_highlight_section_keeps_other_defaults() {
        let project = project_with_highlight(None, Some(true), None, None, None);
        let settings = resolve_highlight(&project).unwrap();
        assert_eq!(settings.theme, HighlightTheme::Github);
        assert!(settings.lstlisting);
        assert!(!settings.numbers);
    }

    #[test]
    fn invalid_highlight_theme_fails_naming_valid_ones() {
        let project = project_with_highlight(Some("dracula"), None, None, None, None);
        let err = resolve_highlight(&project).unwrap_err().to_string();
        assert!(err.contains("dracula"), "{err}");
        for name in ["github", "one-light"] {
            assert!(err.contains(name), "missing {name}: {err}");
        }
    }

    #[test]
    fn highlight_section_maps_caption_and_list_names() {
        let project = project_with_highlight(None, None, None, Some("Snippet"), Some("Snippets"));
        let settings = resolve_highlight(&project).unwrap();
        assert_eq!(settings.caption_name.as_deref(), Some("Snippet"));
        assert_eq!(settings.list_name.as_deref(), Some("Snippets"));
    }

    #[test]
    fn resolve_highlight_leaves_fallback_language_none() {
        let project = project_with_highlight(None, None, None, Some("Snippet"), None);
        let settings = resolve_highlight(&project).unwrap();
        assert_eq!(settings.fallback_language, None);
    }

    #[test]
    fn highlight_document_style_is_honoured_and_defaults_to_light() {
        let project = project_with_highlight_styles(Some("dark-mono"), &[]);
        let settings = resolve_highlight(&project).unwrap();
        assert_eq!(settings.style, HighlightStyle::DarkMono);
        assert!(settings.by_lang.is_empty());

        let project = project_with_highlight_styles(None, &[]);
        let settings = resolve_highlight(&project).unwrap();
        assert_eq!(settings.style, HighlightStyle::Light);
    }

    /// The key goes through the same alias normalisation a block's `lang=`
    /// value does, so `bash = "dark"` is one entry however it is spelled.
    #[test]
    fn by_lang_keys_are_normalised_to_the_syntax_name() {
        let project =
            project_with_highlight_styles(None, &[("Bash", "dark"), ("tex", "light-mono")]);
        let settings = resolve_highlight(&project).unwrap();
        assert_eq!(settings.by_lang.get("bash"), Some(&HighlightStyle::Dark));
        assert_eq!(
            settings.by_lang.get("latex"),
            Some(&HighlightStyle::LightMono)
        );
        assert_eq!(settings.by_lang.len(), 2);
    }

    #[test]
    fn invalid_style_fails_naming_the_value_and_the_valid_ones() {
        let project = project_with_highlight_styles(Some("neon"), &[]);
        let err = resolve_highlight(&project).unwrap_err().to_string();
        assert!(err.contains("neon"), "{err}");
        for name in ["light", "light-mono", "dark", "dark-mono"] {
            assert!(err.contains(name), "missing {name}: {err}");
        }
    }

    #[test]
    fn invalid_by_lang_value_fails_the_build() {
        let project = project_with_highlight_styles(None, &[("bash", "solarized")]);
        let err = resolve_highlight(&project).unwrap_err().to_string();
        assert!(err.contains("solarized"), "{err}");
        assert!(err.contains("dark-mono"), "{err}");
    }

    /// Font parsing and defaults.
    #[test]
    fn highlight_font_parses_and_defaults() {
        let mut project = project_with_highlight(Some("github"), None, None, None, None);
        project.config.highlight.as_mut().unwrap().font = Some("inconsolata".to_string());
        let settings = resolve_highlight(&project).unwrap();
        assert_eq!(settings.font, Some(ListingFont::Inconsolata));

        // Absent section → None (absent key)
        let mut project = project_with_highlight(None, None, None, None, None);
        project.config.highlight = None;
        let settings = resolve_highlight(&project).unwrap();
        assert_eq!(settings.font, None);

        // Present section, absent key → None
        let mut project = project_with_highlight(None, None, None, None, None);
        project.config.highlight.as_mut().unwrap().font = None;
        let settings = resolve_highlight(&project).unwrap();
        assert_eq!(settings.font, None);

        // Explicit "document" → Some(Document)
        let mut project = project_with_highlight(None, None, None, None, None);
        project.config.highlight.as_mut().unwrap().font = Some("document".to_string());
        let settings = resolve_highlight(&project).unwrap();
        assert_eq!(settings.font, Some(ListingFont::Document));
    }

    /// Invalid font fails naming the value and all six valid names.
    #[test]
    fn invalid_highlight_font_fails_naming_valid_ones() {
        let mut project = project_with_highlight(None, None, None, None, None);
        project.config.highlight.as_mut().unwrap().font = Some("jetbrains-mono".to_string());
        let err = resolve_highlight(&project).unwrap_err().to_string();
        assert!(err.contains("jetbrains-mono"), "{err}");
        for name in [
            "document",
            "inconsolata",
            "source-code-pro",
            "dejavu-sans-mono",
            "plex-mono",
            "fira-mono",
        ] {
            assert!(err.contains(name), "missing {name}: {err}");
        }
    }

    fn tectonic_available() -> bool {
        crate::compiler::locate_tectonic().is_some()
    }

    /// A minimal project fixture: a single self-contained .tex file.
    fn fixture() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("main.tex"),
            "\\documentclass{article}\n\\begin{document}\nReproducible world.\n\\end{document}\n",
        )
        .unwrap();
        dir
    }

    #[test]
    fn reproducible_builds_are_byte_identical() {
        // Spawns tectonic: the child inherits the process env, so hold
        // ENV_LOCK or a concurrent HOME swap gives it a cold bundle cache.
        let _env = crate::test_sync::ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !tectonic_available() {
            eprintln!("skipping: tectonic not available in environment");
            return;
        }
        let _tectonic = crate::test_support::tectonic_lock();
        let dir = fixture();
        compiler::compile(
            dir.path(),
            "main.tex",
            false,
            Some(compiler::DEFAULT_EPOCH),
            &crate::highlight::LineMap::default(),
        )
        .unwrap();
        let first = std::fs::read(dir.path().join("main.pdf")).unwrap();
        compiler::compile(
            dir.path(),
            "main.tex",
            false,
            Some(compiler::DEFAULT_EPOCH),
            &crate::highlight::LineMap::default(),
        )
        .unwrap();
        let second = std::fs::read(dir.path().join("main.pdf")).unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn explicit_epoch_builds_are_byte_identical() {
        // Spawns tectonic: the child inherits the process env, so hold
        // ENV_LOCK or a concurrent HOME swap gives it a cold bundle cache.
        let _env = crate::test_sync::ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !tectonic_available() {
            eprintln!("skipping: tectonic not available in environment");
            return;
        }
        let _tectonic = crate::test_support::tectonic_lock();
        let dir = fixture();
        compiler::compile(
            dir.path(),
            "main.tex",
            false,
            Some(1700000000),
            &crate::highlight::LineMap::default(),
        )
        .unwrap();
        let first = std::fs::read(dir.path().join("main.pdf")).unwrap();
        compiler::compile(
            dir.path(),
            "main.tex",
            false,
            Some(1700000000),
            &crate::highlight::LineMap::default(),
        )
        .unwrap();
        let second = std::fs::read(dir.path().join("main.pdf")).unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn non_reproducible_build_still_succeeds() {
        // Spawns tectonic: the child inherits the process env, so hold
        // ENV_LOCK or a concurrent HOME swap gives it a cold bundle cache.
        let _env = crate::test_sync::ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !tectonic_available() {
            eprintln!("skipping: tectonic not available in environment");
            return;
        }
        let _tectonic = crate::test_support::tectonic_lock();
        let dir = fixture();
        compiler::compile(
            dir.path(),
            "main.tex",
            false,
            None,
            &crate::highlight::LineMap::default(),
        )
        .unwrap();
        assert!(dir.path().join("main.pdf").exists());
    }

    const FIXTURE_PDF: &[u8] = include_bytes!("../../tests/fixtures/two-page.pdf");

    #[test]
    fn write_live_preview_is_atomic_and_complete() {
        let dir = tempfile::tempdir().unwrap();
        let pdf = dir.path().join("doc.pdf");
        std::fs::write(&pdf, FIXTURE_PDF).unwrap();
        let out = dir.path().join("preview.png");

        write_live_preview(&pdf, 1, &out).unwrap();

        let tmp = preview_temp_path(&out);
        assert!(!tmp.exists(), "temp sidecar must be gone after publish");
        assert!(out.exists());

        let file = std::fs::File::open(&out).unwrap();
        let decoder = png::Decoder::new(file);
        let mut reader = decoder.read_info().unwrap();
        let mut buf = vec![0u8; reader.output_buffer_size()];
        let info = reader.next_frame(&mut buf).unwrap();
        assert_eq!((info.width, info.height), (612, 842));
        assert_eq!(buf.len(), (info.width * info.height * 4) as usize);
    }

    #[test]
    fn write_live_preview_replaces_previous_frame_atomically() {
        let dir = tempfile::tempdir().unwrap();
        let pdf = dir.path().join("doc.pdf");
        std::fs::write(&pdf, FIXTURE_PDF).unwrap();
        let out = dir.path().join("live.png");

        write_live_preview(&pdf, 1, &out).unwrap();
        let first = std::fs::read(&out).unwrap();
        write_live_preview(&pdf, 2, &out).unwrap();
        let second = std::fs::read(&out).unwrap();

        assert!(!preview_temp_path(&out).exists());
        assert_ne!(first, second, "page 2 should replace page 1 bytes");
        // Target remains a complete PNG after the second cycle.
        let file = std::fs::File::open(&out).unwrap();
        let decoder = png::Decoder::new(file);
        let mut reader = decoder.read_info().unwrap();
        let mut buf = vec![0u8; reader.output_buffer_size()];
        reader.next_frame(&mut buf).unwrap();
    }

    #[test]
    fn failed_rebuild_leaves_previous_preview_untouched() {
        ensure_rustls();
        if !tectonic_available() {
            eprintln!("skipping: tectonic not available in environment");
            return;
        }
        let _tectonic = crate::test_support::tectonic_lock();
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("project.toml"),
            "[document]\ntitle = \"Live Preview\"\nauthor = \"A\"\ntemplate = \"general\"\n\n[build]\nentry = \"main.tex\"\n",
        )
        .unwrap();
        // Intentionally broken so the rebuild fails before any PNG write.
        std::fs::write(dir.path().join("main.tex"), "\\documentclass{article\n").unwrap();
        let out = dir.path().join("preview.png");
        std::fs::write(&out, b"previous-good-frame").unwrap();

        let project = Project {
            root: dir.path().to_path_buf(),
            config: crate::domain::project::ProjectConfig {
                document: crate::domain::project::DocumentConfig {
                    title: "Live Preview".to_string(),
                    author: "A".to_string(),
                    template: "general".to_string(),
                },
                build: crate::domain::project::BuildConfig {
                    entry: "main.tex".to_string(),
                    bibliography: None,
                    reproducible: None,
                },
                diagrams: None,
                highlight: None,
            },
        };
        let build_dir = dir.path().join(".texforge-build");
        let opts = LivePreview {
            page: 1,
            out: Some(out.clone()),
        };
        let result =
            run_build_with_preview(&project, &build_dir, false, None, Some(&opts), None, None);
        assert!(matches!(result, WatchResult::Err(_)));
        assert_eq!(std::fs::read(&out).unwrap(), b"previous-good-frame");
        assert!(!preview_temp_path(&out).exists());
    }

    #[test]
    fn resolve_preview_path_defaults_and_joins_relative() {
        let root = Path::new("/proj");
        assert_eq!(
            resolve_preview_path(root, None),
            PathBuf::from("/proj/preview.png")
        );
        assert_eq!(
            resolve_preview_path(root, Some(Path::new("out/live.png"))),
            PathBuf::from("/proj/out/live.png")
        );
        assert_eq!(
            resolve_preview_path(root, Some(Path::new("/abs/preview.png"))),
            PathBuf::from("/abs/preview.png")
        );
    }

    #[test]
    fn out_of_range_preview_page_errors_without_touching_target() {
        let dir = tempfile::tempdir().unwrap();
        let pdf = dir.path().join("doc.pdf");
        std::fs::write(&pdf, FIXTURE_PDF).unwrap();
        let out = dir.path().join("preview.png");
        std::fs::write(&out, b"keep-me").unwrap();

        let err = write_live_preview(&pdf, 9, &out).unwrap_err();
        assert!(err.to_string().contains("out of range"));
        assert_eq!(std::fs::read(&out).unwrap(), b"keep-me");
        assert!(!preview_temp_path(&out).exists());
    }

    /// A project whose body holds one unbreakable long `\texttt` line.
    fn overfull_project() -> (tempfile::TempDir, Project) {
        let dir = tempfile::tempdir().unwrap();
        let long = "x".repeat(150);
        let main = format!(
            "\\documentclass{{article}}\n\\begin{{document}}\n\\texttt{{{long}}}\n\\end{{document}}\n"
        );
        std::fs::write(dir.path().join("main.tex"), &main).unwrap();
        std::fs::write(
            dir.path().join("project.toml"),
            "[document]\ntitle = \"Overfull\"\nauthor = \"A\"\ntemplate = \"general\"\n\n[build]\nentry = \"main.tex\"\n",
        )
        .unwrap();
        let project = Project {
            root: dir.path().to_path_buf(),
            config: crate::domain::project::ProjectConfig {
                document: crate::domain::project::DocumentConfig {
                    title: "Overfull".to_string(),
                    author: "A".to_string(),
                    template: "general".to_string(),
                },
                build: crate::domain::project::BuildConfig {
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

    /// An unbreakable long `\texttt` line overflows: the engine log is kept
    /// at `build/<stem>.log` and the overfull report prints a summary line
    /// plus one `file:line` detail. Skips without Tectonic.
    #[test]
    fn overfull_build_writes_log_and_reports_boxes() {
        // Spawns tectonic: the child inherits the process env, so hold
        // ENV_LOCK or a concurrent HOME swap gives it a cold bundle cache.
        let _env = crate::test_sync::ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !tectonic_available() {
            eprintln!("skipping: tectonic not available in environment");
            return;
        }
        let _tectonic = crate::test_support::tectonic_lock();
        let (_dir, project) = overfull_project();
        let build_dir = tempfile::tempdir().unwrap();
        let log_path = default_log_path(&project);
        assert_eq!(log_path, project.root.join("build").join("main.log"));

        let (pdf_src, warnings, _raw) =
            compile_project(&project, build_dir.path(), false, None, &log_path).unwrap();
        assert!(pdf_src.exists(), "the engine must still write its PDF");

        let log = std::fs::read_to_string(&log_path)
            .unwrap_or_else(|_| panic!("{} must exist", log_path.display()));
        assert!(
            log.contains("Overfull"),
            "the engine log must keep the box: {}",
            log_path.display()
        );

        // This is the exact string `print_overfull` emits after the
        // existing warning output; the parser-level unit tests pin the
        // per-box sorting, cap and labels.
        let report = compiler::log::format_overfull(&warnings, false);
        assert!(
            report.contains("overfull box(es), worst "),
            "a summary line must appear: {report}"
        );
        let at = report
            .find("main.tex:")
            .expect("the widest box must name its source file and line");
        assert!(
            report[at + "main.tex:".len()..]
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_digit()),
            "the detail line must carry a source line number: {report}"
        );
    }

    /// `--max-overfull` fails the build after the PDF is written, and
    /// `--log` redirects the engine log. Skips without Tectonic.
    #[test]
    fn max_overfull_fails_after_writing_pdf_and_log_redirects() {
        // Lock order is ENV then CWD: no other test holds both, so this
        // cannot deadlock against the CWD-only or ENV-only holders.
        let _env = crate::test_sync::ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !tectonic_available() {
            eprintln!("skipping: tectonic not available in environment");
            return;
        }
        let _tectonic = crate::test_support::tectonic_lock();
        let (_dir, project) = overfull_project();
        let _cwd = crate::test_sync::CWD_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let orig = std::env::current_dir().unwrap();
        std::env::set_current_dir(&project.root).unwrap();

        // A plain build exits 0 and keeps the default log.
        execute(false, None, None, None).unwrap();
        let pdf = project.root.join("overfull.pdf");
        assert!(pdf.exists(), "a plain build writes the PDF");
        assert!(
            project.root.join("build").join("main.log").exists(),
            "a plain build keeps the default log"
        );

        // A tight gate fails naming the flag, but the PDF is still written.
        let err = execute(false, None, None, Some(1.0))
            .unwrap_err()
            .to_string();
        assert!(err.contains("exceeds --max-overfull"), "{err}");
        assert!(pdf.exists(), "the PDF survives the gate failure");

        // `--log` redirects the engine log to the given path.
        execute(false, None, Some(Path::new("custom.log")), None).unwrap();
        assert!(
            project.root.join("custom.log").exists(),
            "the redirected log must exist"
        );

        std::env::set_current_dir(orig).unwrap();
    }

    #[test]
    fn default_log_path_uses_the_entry_stem() {
        let (_dir, project) = overfull_project();
        assert_eq!(
            default_log_path(&project),
            project.root.join("build").join("main.log")
        );
    }

    #[test]
    fn resolve_log_path_prefers_the_flag_and_joins_relative() {
        let default = PathBuf::from("/proj/build/main.log");
        assert_eq!(
            resolve_log_path(Some(Path::new("/abs/engine.log")), default.clone()),
            PathBuf::from("/abs/engine.log")
        );
        let cwd = std::env::current_dir().unwrap();
        assert_eq!(
            resolve_log_path(Some(Path::new("custom.log")), default.clone()),
            cwd.join("custom.log")
        );
        assert_eq!(resolve_log_path(None, default.clone()), default);
    }
}
