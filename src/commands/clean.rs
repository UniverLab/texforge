//! `texforge clean` command implementation.

use anyhow::Result;

use crate::domain::project::Project;
use crate::utils::sanitize_filename;

/// Remove generated PDF files and the legacy build/ directory from the project root.
pub fn execute() -> Result<()> {
    let project = Project::load()?;
    let titulo = &project.config.documento.titulo;
    let pdf_name = format!("{}.pdf", sanitize_filename(titulo));
    let pdf_path = project.root.join(&pdf_name);
    let legacy_build = project.root.join("build");

    let mut cleaned = false;

    if pdf_path.exists() {
        std::fs::remove_file(&pdf_path)?;
        println!("  ◇ {pdf_name} removed");
        cleaned = true;
    }

    if legacy_build.is_dir() {
        std::fs::remove_dir_all(&legacy_build)?;
        println!("  ◇ build/ removed");
        cleaned = true;
    }

    if !cleaned {
        println!("Nothing to clean.");
    }
    Ok(())
}
