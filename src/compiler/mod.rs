//! LaTeX compilation engine — wraps Tectonic.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result};

use crate::linter::Severity;

/// Fixed `SOURCE_DATE_EPOCH` value used when reproducible mode is enabled
/// without an explicit epoch. Deliberately not "now": a fixed value is what
/// makes the same source compile to the same PDF.
pub const DEFAULT_EPOCH: u64 = 1700000000;

/// Compile a LaTeX project to PDF using Tectonic.
/// `root` is the working directory; output PDF goes into `root/` itself.
///
/// When `epoch` is `Some`, the `SOURCE_DATE_EPOCH` environment variable is set
/// for the Tectonic invocation so that identical source plus an identical
/// Tectonic version yields an identical PDF. `None` leaves the build as-is.
///
/// On a successful build, warnings emitted by the engine are parsed and
/// printed to the console — summarized per file, or every occurrence with
/// `verbose` — without changing the success exit code.
pub fn compile(root: &Path, entry: &str, verbose: bool, epoch: Option<u64>) -> Result<()> {
    let tectonic = find_tectonic()?;

    let output = tectonic_command(&tectonic, root, entry, epoch)
        .output()
        .with_context(|| format!("Failed to run tectonic at {}", tectonic.display()))?;

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let raw = format!("{}{}", stdout, stderr);

    let warnings = parse_warnings(&raw);

    if output.status.success() {
        print_warnings(&warnings, verbose);
        return Ok(());
    }

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

/// Build the Tectonic invocation. Reproducible builds set `SOURCE_DATE_EPOCH`;
/// everything else in the invocation is identical either way.
fn tectonic_command(
    tectonic: &std::path::Path,
    root: &Path,
    entry: &str,
    epoch: Option<u64>,
) -> Command {
    let mut cmd = Command::new(tectonic);
    cmd.arg(root.join(entry))
        .arg("--outdir")
        .arg(root)
        .arg("--keep-logs")
        .current_dir(root);
    if let Some(epoch) = epoch {
        cmd.env("SOURCE_DATE_EPOCH", epoch.to_string());
    }
    cmd
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

/// A single TeX/Tectonic warning parsed from compiler output.
#[derive(Debug, PartialEq, Eq)]
struct CompileWarning {
    file: String,
    line: usize,
    severity: Severity,
    kind: &'static str,
    message: String,
}

/// Recognized warning shapes: (substring fragment, canonical kind name).
/// Keep this list in one place so new shapes are a one-line extension.
const WARNING_PATTERNS: &[(&str, &str)] = &[
    ("Underfull \\hbox (badness", "underfull hbox"),
    ("Underfull \\vbox (badness", "underfull vbox"),
    ("Overfull \\hbox", "overfull hbox"),
    ("Overfull \\vbox", "overfull vbox"),
    ("LaTeX Warning:", "latex warning"),
];

/// Classify a single line into a warning kind, or `None` when it does not
/// match any known shape. Unrecognized lines are ignored silently.
fn warning_kind(line: &str) -> Option<&'static str> {
    for (fragment, kind) in WARNING_PATTERNS {
        if line.contains(fragment) {
            return Some(kind);
        }
    }
    if line.starts_with("Package ") && line.contains(" Warning:") {
        return Some("package warning");
    }
    None
}

/// Pull a line number out of TeX warning markers: `at lines 41--42` and
/// `on input line 42`. Returns 0 when none is reported.
fn extract_line(line: &str) -> usize {
    for marker in ["at lines ", "on input line "] {
        if let Some(idx) = line.find(marker) {
            let rest = &line[idx + marker.len()..];
            let num: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            if let Ok(n) = num.parse() {
                return n;
            }
        }
    }
    0
}

/// Heuristic: is a `(path` token an engine file-open marker (as opposed to
/// other parenthesized chatter)? Files end in a known TeX extension.
fn is_source_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    [".tex", ".sty", ".cls", ".def", ".cfg", ".bib"]
        .iter()
        .any(|ext| lower.ends_with(ext))
}

/// Parse TeX/Tectonic stdout/stderr into structured warnings.
///
/// Attribution: the engine prints file opens as `(path` and closes as `)`;
/// warnings seen while a file is open are credited to that file. Line numbers
/// come from `at lines N--M` / `on input line N` markers when the engine
/// reports them.
fn parse_warnings(raw: &str) -> Vec<CompileWarning> {
    let mut warnings = Vec::new();
    let mut file_stack: Vec<String> = Vec::new();

    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed == ")" {
            file_stack.pop();
            continue;
        }
        if let Some(path) = trimmed.strip_prefix('(') {
            if is_source_path(path) {
                file_stack.push(path.to_string());
                continue;
            }
        }
        let Some(kind) = warning_kind(trimmed) else {
            continue;
        };
        warnings.push(CompileWarning {
            file: file_stack.last().cloned().unwrap_or_default(),
            line: extract_line(trimmed),
            severity: Severity::Warning,
            kind,
            message: trimmed.to_string(),
        });
    }

    warnings
}

/// Render warnings for the console: one line per file with counts by kind,
/// or every occurrence when `verbose` is set.
fn format_warnings(warnings: &[CompileWarning], verbose: bool) -> String {
    if warnings.is_empty() {
        return String::new();
    }
    let mut out = format!("WARNINGS ({})\n", warnings.len());
    if verbose {
        for w in warnings {
            let loc = if w.line == 0 {
                format!("[{}]", w.file)
            } else {
                format!("[{}:{}]", w.file, w.line)
            };
            out.push_str(&format!(
                "  {} {loc} {}: {}\n",
                w.severity, w.kind, w.message
            ));
        }
    } else {
        let mut by_file: BTreeMap<&str, BTreeMap<&str, usize>> = BTreeMap::new();
        for w in warnings {
            let file = if w.file.is_empty() {
                "(unknown)"
            } else {
                w.file.as_str()
            };
            *by_file.entry(file).or_default().entry(w.kind).or_default() += 1;
        }
        for (file, kinds) in by_file {
            let counts: Vec<String> = kinds
                .into_iter()
                .map(|(kind, n)| format!("{n} {kind}"))
                .collect();
            out.push_str(&format!("  {file}: {}\n", counts.join(", ")));
        }
    }
    out
}

/// Print warnings to the console on a successful build.
fn print_warnings(warnings: &[CompileWarning], verbose: bool) {
    let formatted = format_warnings(warnings, verbose);
    if !formatted.is_empty() {
        println!("\n{}", formatted.trim_end());
    }
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
pub(crate) fn locate_tectonic() -> Option<std::path::PathBuf> {
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
    fn command_sets_source_date_epoch_when_pinned() {
        let cmd = tectonic_command(
            Path::new("tectonic"),
            Path::new("/tmp/root"),
            "main.tex",
            Some(DEFAULT_EPOCH),
        );
        let epochs: Vec<_> = cmd
            .get_envs()
            .filter_map(|(key, value)| {
                if key == "SOURCE_DATE_EPOCH" {
                    value.map(|b| b.to_os_string())
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(
            epochs,
            vec![std::ffi::OsString::from(DEFAULT_EPOCH.to_string())]
        );
    }

    #[test]
    fn command_omits_source_date_epoch_when_not_pinned() {
        let cmd = tectonic_command(
            Path::new("tectonic"),
            Path::new("/tmp/root"),
            "main.tex",
            None,
        );
        let has_epoch = cmd.get_envs().any(|(key, _)| key == "SOURCE_DATE_EPOCH");
        assert!(!has_epoch);
    }

    #[test]
    fn default_epoch_is_fixed_and_not_zero() {
        // Reproducibility needs a fixed value, not "now" — and not the
        // pre-1970-style sentinel that would be indistinguishable from unset.
        assert_eq!(DEFAULT_EPOCH, 1700000000);
    }

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

    #[test]
    fn tectonic_managed_path_returns_home_texforge() {
        let result = tectonic_managed_path();
        assert!(result.is_ok());
        let path = result.unwrap();
        assert!(path.to_string_lossy().contains(".texforge"));
        assert!(path.to_string_lossy().contains("bin"));
    }

    #[test]
    fn locate_tectonic_returns_option() {
        // Should not panic; may or may not find tectonic
        let _ = locate_tectonic();
    }

    #[test]
    fn parse_errors_tectonic_with_complex_message() {
        let raw = "error: main.tex:100: undefined control sequence \\foo\\bar";
        let errors = parse_errors(raw);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].file, "main.tex");
        assert_eq!(errors[0].line, 100);
        assert!(errors[0].message.contains("\\foo\\bar"));
    }

    #[test]
    fn parse_errors_l_line_without_number() {
        let raw = "! Error.\nl.abc \\badcmd";
        let errors = parse_errors(raw);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].message, "Error.");
        // l.abc doesn't parse as a number, so line stays 0
        assert_eq!(errors[0].line, 0);
    }

    #[test]
    fn parse_tectonic_error_no_colon_in_rest() {
        // "error:" followed by text with no ": " separator
        let mut errors = Vec::new();
        parse_tectonic_error("just a message without colon", &mut errors);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].message, "just a message without colon");
        assert_eq!(errors[0].file, "");
        assert_eq!(errors[0].line, 0);
    }

    #[test]
    fn parse_errors_whitespace_handling() {
        let raw = "  error: a.tex:5: msg  \n  ! Another.\n  l.10 x";
        let errors = parse_errors(raw);
        assert_eq!(errors.len(), 2);
        assert_eq!(errors[0].file, "a.tex");
        assert_eq!(errors[1].line, 10);
    }

    #[test]
    fn parse_errors_empty_lines_between() {
        let raw = "\n\nerror: f.tex:1: e\n\n\n";
        let errors = parse_errors(raw);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].file, "f.tex");
    }

    #[test]
    fn current_target_contains_valid_arch() {
        let target = current_target().unwrap();
        assert!(target.contains("x86_64") || target.contains("aarch64") || target.contains("arm"));
    }

    #[test]
    fn parse_errors_deeply_nested_path() {
        let raw = "error: /some/long/path/to/file.tex:42: bad thing";
        let errors = parse_errors(raw);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].file, "/some/long/path/to/file.tex");
        assert_eq!(errors[0].line, 42);
    }

    #[test]
    fn parse_warnings_underfull_hbox() {
        let raw = "(main.tex\nUnderfull \\hbox (badness 1856) in paragraph at lines 58--59\n)";
        let warnings = parse_warnings(raw);
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].file, "main.tex");
        assert_eq!(warnings[0].line, 58);
        assert_eq!(warnings[0].severity, Severity::Warning);
        assert_eq!(warnings[0].kind, "underfull hbox");
        assert!(warnings[0].message.contains("badness 1856"));
    }

    #[test]
    fn parse_warnings_overfull_hbox() {
        let raw = "Overfull \\hbox (12.0pt too wide) in paragraph at lines 41--42";
        let warnings = parse_warnings(raw);
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].kind, "overfull hbox");
        assert_eq!(warnings[0].line, 41);
        assert_eq!(warnings[0].file, "");
    }

    #[test]
    fn parse_warnings_package_warning() {
        let raw =
            "Package hyperref Warning: Token not allowed in a PDF string (Unicode) on input line 12.";
        let warnings = parse_warnings(raw);
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].kind, "package warning");
        assert_eq!(warnings[0].line, 12);
        assert!(warnings[0].message.contains("hyperref"));
    }

    #[test]
    fn parse_warnings_latex_warning() {
        let raw = "LaTeX Warning: Reference `fig1' on page 1 undefined on input line 22.";
        let warnings = parse_warnings(raw);
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].kind, "latex warning");
        assert_eq!(warnings[0].line, 22);
    }

    #[test]
    fn parse_warnings_ignores_unrecognized_lines() {
        let raw = "some random output\n[]\\OT1/cmr/m/n/10 foo\nnot a warning";
        let warnings = parse_warnings(raw);
        assert!(warnings.is_empty());
    }

    #[test]
    fn parse_warnings_empty_input() {
        assert!(parse_warnings("").is_empty());
    }

    #[test]
    fn parse_warnings_file_tracking() {
        let raw = "(main.tex\n(chapter.tex\nUnderfull \\hbox (badness 1000) in paragraph at lines 1--2\n)\nUnderfull \\hbox (badness 2000) in paragraph at lines 3--4\n)";
        let warnings = parse_warnings(raw);
        assert_eq!(warnings.len(), 2);
        assert_eq!(warnings[0].file, "chapter.tex");
        assert_eq!(warnings[1].file, "main.tex");
    }

    #[test]
    fn parse_warnings_badness_variant_ignored_when_not_badness() {
        let raw = "Underfull \\hbox (2.0pt too wide) in paragraph at lines 5--6";
        let warnings = parse_warnings(raw);
        assert!(warnings.is_empty());
    }

    #[test]
    fn extract_line_from_markers() {
        assert_eq!(extract_line("in paragraph at lines 41--42"), 41);
        assert_eq!(extract_line("on input line 7."), 7);
        assert_eq!(extract_line("no marker here"), 0);
    }

    #[test]
    fn format_warnings_summary_groups_by_file() {
        let warnings = vec![
            CompileWarning {
                file: "main.tex".into(),
                line: 58,
                severity: Severity::Warning,
                kind: "underfull hbox",
                message: "Underfull \\hbox (badness 1856)".into(),
            },
            CompileWarning {
                file: "main.tex".into(),
                line: 90,
                severity: Severity::Warning,
                kind: "underfull hbox",
                message: "Underfull \\hbox (badness 2000)".into(),
            },
            CompileWarning {
                file: "skills.tex".into(),
                line: 12,
                severity: Severity::Warning,
                kind: "package warning",
                message: "Package hyperref Warning: ...".into(),
            },
        ];
        let out = format_warnings(&warnings, false);
        assert!(out.starts_with("WARNINGS (3)\n"));
        assert!(out.contains("  main.tex: 2 underfull hbox"));
        assert!(out.contains("  skills.tex: 1 package warning"));
    }

    #[test]
    fn format_warnings_verbose_lists_every_occurrence() {
        let warnings = vec![CompileWarning {
            file: "main.tex".into(),
            line: 58,
            severity: Severity::Warning,
            kind: "underfull hbox",
            message: "Underfull \\hbox (badness 1856) in paragraph at lines 58--59".into(),
        }];
        let out = format_warnings(&warnings, true);
        assert!(out.contains("WARNING [main.tex:58] underfull hbox:"));
        assert!(out.contains("badness 1856"));
    }

    #[test]
    fn format_warnings_empty_is_empty() {
        assert!(format_warnings(&[], false).is_empty());
        assert!(format_warnings(&[], true).is_empty());
    }
}
