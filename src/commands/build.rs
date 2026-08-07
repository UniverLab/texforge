//! `texforge build` command implementation.

use std::path::Path;
use std::sync::mpsc;
use std::time::Duration;

use anyhow::Result;
use notify::{RecursiveMode, Watcher};

use crate::commands::init::BANNER;
use crate::compiler;
use crate::diagrams;
use crate::domain::project::{Project, Reproducible};
use crate::utils::sanitize_filename;

/// Resolve the `SOURCE_DATE_EPOCH` value for a build. The CLI flag wins over
/// `project.toml`; a flag with no value pins the fixed default epoch; a config
/// value pins its own epoch or the default when enabled without one.
fn resolve_epoch(cli: Option<Option<u64>>, config: Option<Reproducible>) -> Option<u64> {
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

/// Compile project to PDF using a temp directory, output named after the document title.
pub fn execute(verbose: bool, reproducible: Option<Option<u64>>) -> Result<()> {
    let project = Project::load()?;
    let titulo = &project.config.document.title;
    println!("Building project: {titulo}");

    let epoch = resolve_epoch(reproducible, project.config.build.reproducible);
    if epoch.is_some() {
        println!("  ◇ reproducible build (SOURCE_DATE_EPOCH pinned)");
    }

    let temp_dir = tempfile::tempdir()?;
    let build_dir = temp_dir.path();
    println!("  ◇ temp: {}", build_dir.display());

    diagrams::process(&project.root, &project.config.build.entry, build_dir)?;
    let entry_filename = Path::new(&project.config.build.entry)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| project.config.build.entry.clone());
    compiler::compile(build_dir, &entry_filename, verbose, epoch)?;

    let pdf_name = format!("{}.pdf", sanitize_filename(titulo));
    let pdf_dest = project.root.join(&pdf_name);
    let pdf_src = build_dir.join(
        Path::new(&project.config.build.entry)
            .with_extension("pdf")
            .file_name()
            .unwrap(),
    );
    std::fs::copy(&pdf_src, &pdf_dest)?;
    println!("  ◇ {}", pdf_dest.display());

    Ok(())
}

/// Watch for .tex file changes and rebuild with debounce.
pub fn watch(delay_secs: u64, verbose: bool, reproducible: Option<Option<u64>>) -> Result<()> {
    let project = Project::load()?;
    let epoch = resolve_epoch(reproducible, project.config.build.reproducible);
    let debounce = Duration::from_secs(delay_secs);
    let cooldown = Duration::from_secs(2);

    print_watch_header(&project.config.document.title, delay_secs);

    let temp_dir = tempfile::tempdir()?;
    let build_dir = temp_dir.path().to_path_buf();

    let started = std::time::Instant::now();
    let result = run_build(&project, &build_dir, verbose, epoch);
    redraw_status(&result, 1, started);

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
                let relevant = event.paths.iter().any(|p| {
                    !p.starts_with(&build_dir)
                        && p.extension().and_then(|e| e.to_str()) == Some("tex")
                });
                if relevant && last_build.elapsed() > cooldown {
                    pending = true;
                    last_event = std::time::Instant::now();
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(_) => break,
        }

        if last_tick.elapsed() >= Duration::from_secs(1) {
            last_tick = std::time::Instant::now();
            redraw_status(&last_result, build_count, started);
        }

        if pending && last_event.elapsed() >= debounce {
            pending = false;
            build_count += 1;
            last_result = run_build(&project, &build_dir, verbose, epoch);
            last_build = std::time::Instant::now();
            redraw_status(&last_result, build_count, started);
        }
    }

    Ok(())
}

fn print_watch_header(title: &str, delay_secs: u64) {
    print!("\x1B[2J\x1B[H");
    println!("{BANNER}");
    println!("  {title} — watching  ({delay_secs}s debounce  Ctrl+C to stop)");
}

fn redraw_status(result: &WatchResult, build_count: u32, started: std::time::Instant) {
    print!("\x1B[15;0H\x1B[J");
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
) -> WatchResult {
    let _ = std::fs::create_dir_all(build_dir);
    if let Err(e) = diagrams::process(&project.root, &project.config.build.entry, build_dir) {
        return WatchResult::Err(e.to_string());
    }
    let entry_filename = Path::new(&project.config.build.entry)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| project.config.build.entry.clone());
    match compiler::compile(build_dir, &entry_filename, verbose, epoch) {
        Ok(()) => {
            let pdf_name = format!("{}.pdf", sanitize_filename(&project.config.document.title));
            let pdf_dest = project.root.join(&pdf_name);
            let pdf_src = build_dir.join(
                Path::new(&project.config.build.entry)
                    .with_extension("pdf")
                    .file_name()
                    .unwrap(),
            );
            match std::fs::copy(&pdf_src, &pdf_dest) {
                Ok(_) => WatchResult::Ok(pdf_name),
                Err(e) => WatchResult::Err(e.to_string()),
            }
        }
        Err(e) => WatchResult::Err(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        if !tectonic_available() {
            eprintln!("skipping: tectonic not available in environment");
            return;
        }
        let dir = fixture();
        compiler::compile(dir.path(), "main.tex", false, Some(compiler::DEFAULT_EPOCH)).unwrap();
        let first = std::fs::read(dir.path().join("main.pdf")).unwrap();
        compiler::compile(dir.path(), "main.tex", false, Some(compiler::DEFAULT_EPOCH)).unwrap();
        let second = std::fs::read(dir.path().join("main.pdf")).unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn explicit_epoch_builds_are_byte_identical() {
        if !tectonic_available() {
            eprintln!("skipping: tectonic not available in environment");
            return;
        }
        let dir = fixture();
        compiler::compile(dir.path(), "main.tex", false, Some(1700000000)).unwrap();
        let first = std::fs::read(dir.path().join("main.pdf")).unwrap();
        compiler::compile(dir.path(), "main.tex", false, Some(1700000000)).unwrap();
        let second = std::fs::read(dir.path().join("main.pdf")).unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn non_reproducible_build_still_succeeds() {
        if !tectonic_available() {
            eprintln!("skipping: tectonic not available in environment");
            return;
        }
        let dir = fixture();
        compiler::compile(dir.path(), "main.tex", false, None).unwrap();
        assert!(dir.path().join("main.pdf").exists());
    }
}
