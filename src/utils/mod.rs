//! Utility functions.

use std::path::Path;

use anyhow::Context;

/// Get the `TexForge` data directory (~/.texforge)
pub fn data_dir() -> anyhow::Result<std::path::PathBuf> {
    let home =
        dirs::home_dir().ok_or_else(|| anyhow::anyhow!("Could not determine home directory"))?;
    let data_dir = home.join(".texforge");
    std::fs::create_dir_all(&data_dir)?;
    Ok(data_dir)
}

/// Get the templates directory (~/.texforge/templates)
pub fn templates_dir() -> anyhow::Result<std::path::PathBuf> {
    let dir = data_dir()?.join("templates");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Find all .tex files in a directory, excluding build/.
pub fn find_tex_files(root: &Path) -> anyhow::Result<Vec<std::path::PathBuf>> {
    let mut files = Vec::new();
    let build_dir = root.join("build");

    for entry in walkdir::WalkDir::new(root)
        .follow_links(true)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if path.starts_with(&build_dir) {
            continue;
        }
        if entry.file_type().is_file() {
            if let Some(ext) = path.extension() {
                if ext == "tex" {
                    files.push(path.to_path_buf());
                }
            }
        }
    }

    Ok(files)
}

/// Find all `.bib` files under `root` (excluding the legacy `build/` dir).
pub fn find_bib_files(root: &Path) -> anyhow::Result<Vec<std::path::PathBuf>> {
    let mut files = Vec::new();
    let build_dir = root.join("build");

    for entry in walkdir::WalkDir::new(root)
        .follow_links(true)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if path.starts_with(&build_dir) {
            continue;
        }
        if entry.file_type().is_file() && path.extension().is_some_and(|ext| ext == "bib") {
            files.push(path.to_path_buf());
        }
    }

    Ok(files)
}

/// Sanitize a document title into a valid filename (lowercase, alphanumeric + hyphens).
pub fn sanitize_filename(title: &str) -> String {
    title
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

/// Mirror asset files into the build dir so tectonic can resolve relative paths.
/// Skips .tex files (the diagram pre-processor writes processed copies), hidden
/// entries, and a legacy top-level `build/` directory. On Unix each file is
/// symlinked with an absolute target (the build dir may live in the system temp
/// dir, so relative links would dangle); on Windows files are copied.
pub fn mirror_assets(root: &Path, build_dir: &Path) -> anyhow::Result<()> {
    let root = root
        .canonicalize()
        .with_context(|| format!("Failed to resolve {}", root.display()))?;

    let walker = walkdir::WalkDir::new(&root)
        .min_depth(1)
        .into_iter()
        .filter_entry(|e| {
            let name = e.file_name().to_string_lossy();
            let is_legacy_build = e.depth() == 1 && name == "build";
            !name.starts_with('.') && !is_legacy_build && e.path() != build_dir
        });

    for entry in walker.filter_map(|e| e.ok()) {
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("tex") {
            continue;
        }
        let rel = path.strip_prefix(&root).unwrap();
        let dest = build_dir.join(rel);
        if dest.symlink_metadata().is_ok() {
            continue;
        }
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        link_or_copy_file(path, &dest)?;
    }
    Ok(())
}

#[cfg(unix)]
fn link_or_copy_file(src: &Path, dest: &Path) -> anyhow::Result<()> {
    std::os::unix::fs::symlink(src, dest)
        .with_context(|| format!("Failed to symlink {} -> {}", dest.display(), src.display()))
}

#[cfg(not(unix))]
fn link_or_copy_file(src: &Path, dest: &Path) -> anyhow::Result<()> {
    std::fs::copy(src, dest)
        .with_context(|| format!("Failed to copy {} -> {}", src.display(), dest.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_basic_title() {
        assert_eq!(
            sanitize_filename("My Thesis: Final Version"),
            "my-thesis-final-version"
        );
    }

    #[test]
    fn sanitize_collapses_separators() {
        assert_eq!(sanitize_filename("a  --  b"), "a-b");
    }

    #[test]
    fn sanitize_keeps_unicode_alphanumerics() {
        assert_eq!(sanitize_filename("Análisis Numérico"), "análisis-numérico");
    }

    #[test]
    fn mirror_assets_links_nested_assets_in_tex_dirs() {
        let src = tempfile::tempdir().unwrap();
        let build = tempfile::tempdir().unwrap();
        let chapters = src.path().join("chapters");
        std::fs::create_dir_all(&chapters).unwrap();
        std::fs::write(chapters.join("ch1.tex"), "x").unwrap();
        std::fs::write(chapters.join("img.png"), [1u8, 2]).unwrap();
        std::fs::write(src.path().join("refs.bib"), "@misc{a}").unwrap();

        mirror_assets(src.path(), build.path()).unwrap();

        let img = build.path().join("chapters/img.png");
        assert!(img.exists(), "nested asset should be mirrored");
        assert_eq!(std::fs::read(&img).unwrap(), vec![1u8, 2]);
        assert!(build.path().join("refs.bib").exists());
        assert!(
            !build.path().join("chapters/ch1.tex").exists(),
            ".tex files must not be mirrored"
        );
    }

    #[test]
    fn mirror_assets_skips_hidden_and_legacy_build() {
        let src = tempfile::tempdir().unwrap();
        let build = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(src.path().join("build")).unwrap();
        std::fs::write(src.path().join("build/old.pdf"), "x").unwrap();
        std::fs::write(src.path().join(".hidden"), "x").unwrap();

        mirror_assets(src.path(), build.path()).unwrap();

        assert!(!build.path().join("build").exists());
        assert!(!build.path().join(".hidden").exists());
    }
}
