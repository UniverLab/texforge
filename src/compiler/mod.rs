//! LaTeX compilation engine — wraps Tectonic.

use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result};

/// Compile a LaTeX project to PDF using Tectonic.
/// `root` is the working directory; output PDF goes into `root/` itself.
pub fn compile(root: &Path, entry: &str) -> Result<()> {
    let tectonic = find_tectonic()?;
    let entry_path = root.join(entry);

    let output = Command::new(&tectonic)
        .arg(&entry_path)
        .arg("--outdir")
        .arg(root)
        .arg("--keep-logs")
        .current_dir(root)
        .output()
        .with_context(|| format!("Failed to run tectonic at {}", tectonic.display()))?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let raw = format!("{}{}", stdout, stderr);

    let errors = parse_errors(&raw);
    if errors.is_empty() {
        anyhow::bail!("Compilation failed:\n{}", raw.trim());
    }

    let mut msg = String::from("Compilation failed:\n\n");
    for e in &errors {
        msg.push_str(&format!(
            "ERROR [{}:{}]\n  {}\n\n",
            e.file, e.line, e.message
        ));
    }
    anyhow::bail!("{}", msg.trim());
}

struct CompileError {
    file: String,
    line: usize,
    message: String,
}

/// Parse tectonic/TeX error output into structured errors.
fn parse_errors(raw: &str) -> Vec<CompileError> {
    let mut errors = Vec::new();

    for line in raw.lines() {
        let trimmed = line.trim();

        if let Some(rest) = trimmed.strip_prefix("error:") {
            parse_tectonic_error(rest, &mut errors);
            continue;
        }
        if let Some(msg) = trimmed.strip_prefix("! ") {
            errors.push(CompileError {
                file: String::new(),
                line: 0,
                message: msg.to_string(),
            });
        }
        if let Some(num_str) = trimmed.strip_prefix("l.") {
            let num_part: String = num_str.chars().take_while(|c| c.is_ascii_digit()).collect();
            if let Ok(n) = num_part.parse::<usize>() {
                if let Some(last) = errors.last_mut() {
                    last.line = n;
                }
            }
        }
    }

    errors
}

fn parse_tectonic_error(rest: &str, errors: &mut Vec<CompileError>) {
    let rest = rest.trim();
    if let Some((loc, msg)) = rest.split_once(": ") {
        if let Some((file, line_str)) = loc.rsplit_once(':') {
            if let Ok(line_num) = line_str.parse::<usize>() {
                errors.push(CompileError {
                    file: file.trim().to_string(),
                    line: line_num,
                    message: msg.trim().to_string(),
                });
                return;
            }
        }
    }
    errors.push(CompileError {
        file: String::new(),
        line: 0,
        message: rest.to_string(),
    });
}

/// Find the tectonic binary in PATH or known locations, auto-installing if needed.
fn find_tectonic() -> Result<std::path::PathBuf> {
    if let Some(path) = locate_tectonic() {
        return Ok(path);
    }
    eprintln!("Tectonic not found. Installing automatically...");
    let dest = tectonic_managed_path()?;
    install_tectonic(&dest)?;
    Ok(dest)
}

/// Locate tectonic in PATH or known install locations without installing.
fn locate_tectonic() -> Option<std::path::PathBuf> {
    // Check PATH using platform-appropriate which/where
    #[cfg(unix)]
    let which_cmd = "which";
    #[cfg(not(unix))]
    let which_cmd = "where";

    if let Ok(output) = Command::new(which_cmd).arg("tectonic").output() {
        if output.status.success() {
            let path = String::from_utf8_lossy(&output.stdout)
                .lines()
                .next()
                .unwrap_or("")
                .trim()
                .to_string();
            if !path.is_empty() {
                return Some(path.into());
            }
        }
    }

    // Check known locations
    [
        tectonic_managed_path().ok(),
        dirs::home_dir().map(|h| h.join(".cargo/bin").join(TECTONIC_BIN)),
        Some("/usr/local/bin/tectonic".into()),
        Some("/opt/homebrew/bin/tectonic".into()),
    ]
    .into_iter()
    .flatten()
    .find(|p| p.exists())
}

/// Tectonic binary filename — Windows requires the .exe extension to execute it.
#[cfg(windows)]
const TECTONIC_BIN: &str = "tectonic.exe";
#[cfg(not(windows))]
const TECTONIC_BIN: &str = "tectonic";

fn tectonic_managed_path() -> Result<std::path::PathBuf> {
    dirs::home_dir()
        .map(|h| h.join(".texforge").join("bin").join(TECTONIC_BIN))
        .ok_or_else(|| anyhow::anyhow!("Could not determine home directory"))
}

/// Download and install tectonic to the given path.
fn install_tectonic(dest: &std::path::Path) -> Result<()> {
    let target = current_target()?;
    let version = "0.15.0";
    let (filename, is_zip) = if target.contains("windows") {
        (format!("tectonic-{}-{}.zip", version, target), true)
    } else {
        (format!("tectonic-{}-{}.tar.gz", version, target), false)
    };

    let url = format!(
        "https://github.com/tectonic-typesetting/tectonic/releases/download/tectonic%40{}/{}",
        version, filename
    );

    eprintln!("Downloading tectonic {}...", version);

    let response = reqwest::blocking::Client::new()
        .get(&url)
        .header("User-Agent", "texforge")
        .send()
        .context("Failed to download tectonic")?;

    if !response.status().is_success() {
        anyhow::bail!(
            "Failed to download tectonic: HTTP {}\nURL: {}",
            response.status(),
            url
        );
    }

    let bytes = response.bytes()?;

    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }

    if is_zip {
        install_from_zip(&bytes, dest)?;
    } else {
        install_from_targz(&bytes, dest)?;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dest, std::fs::Permissions::from_mode(0o755))?;
    }

    eprintln!("  ◇ Tectonic installed to {}", dest.display());
    Ok(())
}

fn install_from_targz(bytes: &[u8], dest: &std::path::Path) -> Result<()> {
    let decoder = flate2::read::GzDecoder::new(bytes);
    let mut archive = tar::Archive::new(decoder);
    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.to_string_lossy().to_string();
        if path.ends_with("tectonic") || path == "tectonic" {
            std::io::copy(&mut entry, &mut std::fs::File::create(dest)?)?;
            return Ok(());
        }
    }
    anyhow::bail!("tectonic binary not found in archive")
}

fn install_from_zip(bytes: &[u8], dest: &std::path::Path) -> Result<()> {
    let cursor = std::io::Cursor::new(bytes);
    let mut archive = zip::ZipArchive::new(cursor)?;
    for i in 0..archive.len() {
        let mut file = archive.by_index(i)?;
        if file.name().ends_with("tectonic.exe") || file.name() == "tectonic.exe" {
            std::io::copy(&mut file, &mut std::fs::File::create(dest)?)?;
            return Ok(());
        }
    }
    anyhow::bail!("tectonic.exe not found in archive")
}

fn current_target() -> Result<&'static str> {
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    return Ok("x86_64-unknown-linux-musl");
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    return Ok("aarch64-unknown-linux-musl");
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    return Ok("x86_64-apple-darwin");
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    return Ok("aarch64-apple-darwin");
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    return Ok("x86_64-pc-windows-msvc");
    #[cfg(not(any(
        all(target_os = "linux", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "aarch64"),
        all(target_os = "macos", target_arch = "x86_64"),
        all(target_os = "macos", target_arch = "aarch64"),
        all(target_os = "windows", target_arch = "x86_64"),
    )))]
    anyhow::bail!("Unsupported platform for automatic tectonic installation")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_errors_tectonic_style() {
        let raw = "error: main.tex:42: undefined control sequence \\foo";
        let errors = parse_errors(raw);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].file, "main.tex");
        assert_eq!(errors[0].line, 42);
        assert_eq!(errors[0].message, "undefined control sequence \\foo");
    }

    #[test]
    fn parse_errors_bang_style() {
        let raw = "! Undefined control sequence.\nl.10 \\badcmd";
        let errors = parse_errors(raw);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].message, "Undefined control sequence.");
        assert_eq!(errors[0].line, 10);
    }

    #[test]
    fn parse_errors_bang_no_line() {
        let raw = "! Missing $ inserted.";
        let errors = parse_errors(raw);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].message, "Missing $ inserted.");
        assert_eq!(errors[0].line, 0);
    }

    #[test]
    fn parse_errors_multiple() {
        let raw = "error: a.tex:1: first error\nerror: b.tex:5: second error";
        let errors = parse_errors(raw);
        assert_eq!(errors.len(), 2);
        assert_eq!(errors[0].file, "a.tex");
        assert_eq!(errors[1].file, "b.tex");
        assert_eq!(errors[1].line, 5);
    }

    #[test]
    fn parse_errors_empty() {
        let errors = parse_errors("");
        assert!(errors.is_empty());
    }

    #[test]
    fn parse_errors_unrecognized_line() {
        let raw = "some random output\nnot an error";
        let errors = parse_errors(raw);
        assert!(errors.is_empty());
    }

    #[test]
    fn parse_tectonic_error_with_location() {
        let mut errors = Vec::new();
        parse_tectonic_error("main.tex:10: undefined control sequence", &mut errors);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].file, "main.tex");
        assert_eq!(errors[0].line, 10);
        assert_eq!(errors[0].message, "undefined control sequence");
    }

    #[test]
    fn parse_tectonic_error_without_colon_location() {
        let mut errors = Vec::new();
        parse_tectonic_error("some generic message", &mut errors);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].file, "");
        assert_eq!(errors[0].line, 0);
        assert_eq!(errors[0].message, "some generic message");
    }

    #[test]
    fn parse_tectonic_error_non_numeric_line() {
        let mut errors = Vec::new();
        parse_tectonic_error("file.tex:abc: bad", &mut errors);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].file, "");
        assert_eq!(errors[0].line, 0);
    }

    #[test]
    fn parse_errors_mixed_styles() {
        let raw = "error: a.tex:1: first\n! Second error.\nl.20 \\second";
        let errors = parse_errors(raw);
        assert_eq!(errors.len(), 2);
        assert_eq!(errors[0].file, "a.tex");
        assert_eq!(errors[1].line, 20);
    }

    #[test]
    fn current_target_returns_known_value() {
        let target = current_target().unwrap();
        assert!(!target.is_empty());
        assert!(target.contains("linux") || target.contains("macos") || target.contains("windows"));
    }

    #[test]
    fn find_tectonic_returns_path() {
        let result = find_tectonic();
        // This test just verifies the function doesn't panic;
        // tectonic may or may not be installed.
        if let Ok(path) = result {
            assert!(!path.as_os_str().is_empty());
        }
    }
}
