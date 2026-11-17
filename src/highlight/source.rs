//! A `code` block's text taken from a project file (`file=` + `lines=`).
//!
//! The `\lstinputlisting` / `\inputminted` shape without their costs: the
//! file is read at build time and its lines are embedded in the generated
//! LaTeX, so the document stays self-contained — no `\inputminted`, no
//! `\lstinputlisting`, no Pygments, no shell-escape, no external file needed
//! at compile time.

use std::collections::HashMap;
use std::path::Path;

use anyhow::{anyhow, Result};

/// Where one block's text comes from and which lines survive.
#[derive(Debug)]
pub(crate) struct Resolved {
    /// The rendered lines, `\r` stripped, range already applied.
    pub(crate) body: String,
    /// The file's own number of the first rendered line (1 for inline).
    pub(crate) first_number: usize,
    /// `true` when the text came from `file=` (warnings point at the
    /// `\begin` line instead of per-line body lines).
    pub(crate) external: bool,
    /// Extension-inferred language (`Some` only when the block has no
    /// explicit `lang=` and the extension maps to a known syntax).
    pub(crate) inferred_lang: Option<String>,
}

/// Resolve one block's text: an inline body, or the `file=` file with the
/// `lines=` range kept. `rel` is the block's source file (`rel:line` error
/// prefix, the existing convention); `root` is the project root.
pub(crate) fn resolve(
    root: &Path,
    rel: &str,
    first_line: usize,
    opts: &HashMap<String, String>,
    inline: &str,
) -> Result<Resolved> {
    let Some(file) = opts.get("file").map(String::as_str) else {
        if let Some(lines) = opts.get("lines") {
            anyhow::bail!("{rel}:{first_line}: lines= requires file= (lines='{lines}')");
        }
        return Ok(Resolved {
            body: inline.to_string(),
            first_number: 1,
            external: false,
            inferred_lang: None,
        });
    };
    if file.is_empty() {
        anyhow::bail!("{rel}:{first_line}: file= needs a path (relative to the project root)");
    }
    let prefix = format!("{rel}:{first_line}: code file '{file}': ");
    let target = resolve_file(root, file).map_err(|e| anyhow!("{prefix}{e}"))?;
    let raw =
        std::fs::read_to_string(&target).map_err(|e| anyhow!("{prefix}cannot read file ({e})"))?;
    let raw = raw.replace('\r', "");
    let all: Vec<&str> = raw.split('\n').collect();
    // A trailing newline terminates the last line; it is not a line of its own.
    let count = match all.last() {
        Some(&"") if all.len() > 1 => all.len() - 1,
        _ => all.len(),
    };
    let (start, end) = parse_range(opts.get("lines").map(String::as_str), count)
        .map_err(|e| anyhow!("{prefix}{e}"))?;
    let body = all[start - 1..end.min(count.max(1))].join("\n");
    let inferred = match opts.get("lang") {
        Some(_) => None,
        None => extension_lang(file),
    };
    Ok(Resolved {
        body,
        first_number: start,
        external: true,
        inferred_lang: inferred,
    })
}

/// Reject an absolute or escaping path; otherwise join it onto the project
/// root. Never resolved against the build directory: the referenced scripts
/// live in the project, not in the build tree.
fn resolve_file(root: &Path, file: &str) -> Result<std::path::PathBuf> {
    let bytes = file.as_bytes();
    let drive_absolute = bytes.len() > 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic();
    if file.starts_with('/') || file.starts_with('\\') || drive_absolute {
        anyhow::bail!("path must stay inside the project root");
    }
    let candidate = root.join(file);
    let root_norm = normalize(root);
    let full_norm = normalize(&candidate);
    if full_norm != root_norm && !full_norm.starts_with(&format!("{root_norm}/")) {
        anyhow::bail!("path must stay inside the project root");
    }
    Ok(candidate)
}

/// Lexical normalization: collapse `.`, `..` and duplicate separators
/// without touching the filesystem (the file may not exist yet when
/// `check` reports it missing).
fn normalize(path: &Path) -> String {
    let mut parts: Vec<&str> = Vec::new();
    let raw = path.to_string_lossy().replace('\\', "/");
    let absolute = raw.starts_with('/');
    for part in raw.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            _ => parts.push(part),
        }
    }
    let joined = parts.join("/");
    if absolute {
        format!("/{joined}")
    } else {
        joined
    }
}

/// Parse `lines=`: `a-b`, `a-`, `-b`, or a single `a` (1-based, inclusive).
/// Defaults to the whole file; an explicit `end` clamps to the file's line
/// count. Every error names the file's real line count so the author can
/// pick a range that exists.
fn parse_range(spec: Option<&str>, count: usize) -> Result<(usize, usize)> {
    let Some(spec) = spec else {
        return Ok((1, count.max(1)));
    };
    let bad =
        || anyhow!("malformed lines='{spec}' — expected a-b, a-, -b or a (file has {count} lines)");
    if spec.is_empty() || spec.chars().any(|c| !c.is_ascii_digit() && c != '-') {
        return Err(bad());
    }
    let (lo, hi) = match spec.split_once('-') {
        None => {
            let line: usize = spec.parse().map_err(|_| bad())?;
            (line, Some(line))
        }
        Some((a, b)) => {
            if a.is_empty() && b.is_empty() {
                return Err(bad());
            }
            let lo = if a.is_empty() {
                1
            } else {
                a.parse().map_err(|_| bad())?
            };
            let hi = if b.is_empty() {
                None
            } else {
                Some(b.parse().map_err(|_| bad())?)
            };
            (lo, hi)
        }
    };
    if lo < 1 || hi.is_some_and(|hi| hi < 1) {
        return Err(bad());
    }
    if let Some(hi) = hi {
        if lo > hi {
            return Err(anyhow!(
                "malformed lines='{spec}' — start {lo} is greater than end {hi} (file has {count} lines)"
            ));
        }
    }
    if lo > count {
        return Err(anyhow!(
            "lines='{spec}' start {lo} is past the end of the file ({count} lines)"
        ));
    }
    Ok((lo, hi.unwrap_or(count).min(count.max(1))))
}

/// The syntax name for a file extension through the embedded syntax set,
/// or `None` when the extension maps to nothing (plain monospace fallback).
fn extension_lang(file: &str) -> Option<String> {
    let ext = file.rsplit('.').next()?;
    if !file.contains('.') || ext.is_empty() {
        return None;
    }
    super::engine::language_for_extension(&ext.to_lowercase()).map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root_with(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (name, content) in files {
            std::fs::write(dir.path().join(name), content).unwrap();
        }
        dir
    }

    fn opts(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect()
    }

    #[test]
    fn inline_body_passes_through_without_options() {
        let dir = root_with(&[]);
        let out = resolve(dir.path(), "main.tex", 3, &opts(&[]), "x = 1").unwrap();
        assert_eq!(out.body, "x = 1");
        assert_eq!(out.first_number, 1);
        assert!(!out.external);
        assert_eq!(out.inferred_lang, None);
    }

    #[test]
    fn lines_without_file_is_an_error() {
        let dir = root_with(&[]);
        let err = resolve(dir.path(), "main.tex", 3, &opts(&[("lines", "1-2")]), "")
            .unwrap_err()
            .to_string();
        assert!(err.contains("main.tex:3"), "{err}");
        assert!(err.contains("lines= requires file="), "{err}");
    }

    #[test]
    fn absolute_and_escaping_paths_are_rejected() {
        let dir = root_with(&[("ok.py", "x = 1\n")]);
        for bad in [
            "/etc/passwd",
            "C:\\evil.py",
            "../outside.py",
            "a/../../outside.py",
        ] {
            let err = resolve(dir.path(), "main.tex", 7, &opts(&[("file", bad)]), "")
                .unwrap_err()
                .to_string();
            assert!(err.contains("main.tex:7"), "{bad}: {err}");
            assert!(err.contains("code file"), "{bad}: {err}");
            assert!(err.contains("inside the project root"), "{bad}: {err}");
        }
        // A `./` prefix stays inside the root and still resolves.
        let out = resolve(
            dir.path(),
            "main.tex",
            1,
            &opts(&[("file", "./ok.py")]),
            "ignored",
        )
        .unwrap();
        assert_eq!(out.body, "x = 1");
    }

    #[test]
    fn range_forms_select_the_right_lines() {
        let dir = root_with(&[("s.py", "l1\nl2\nl3\nl4\nl5\n")]);
        let body = |lines: Option<&str>| {
            let mut o = opts(&[("file", "s.py")]);
            if let Some(spec) = lines {
                o.insert("lines".to_string(), spec.to_string());
            }
            resolve(dir.path(), "main.tex", 1, &o, "").unwrap()
        };
        assert_eq!(body(None).body, "l1\nl2\nl3\nl4\nl5");
        assert_eq!(body(Some("2-4")).body, "l2\nl3\nl4");
        assert_eq!(body(Some("2-4")).first_number, 2);
        assert_eq!(body(Some("3-")).body, "l3\nl4\nl5");
        assert_eq!(body(Some("-2")).body, "l1\nl2");
        assert_eq!(body(Some("4")).body, "l4");
        assert_eq!(body(Some("4")).first_number, 4);
        assert_eq!(body(Some("4-99")).body, "l4\nl5");
    }

    #[test]
    fn malformed_and_inverted_ranges_fail() {
        let dir = root_with(&[("s.py", "l1\nl2\n")]);
        for bad in ["", "x", "1-2-3", "-", "0-2", "2-0", "4-2"] {
            let err = resolve(
                dir.path(),
                "main.tex",
                5,
                &opts(&[("file", "s.py"), ("lines", bad)]),
                "",
            )
            .unwrap_err()
            .to_string();
            assert!(err.contains("main.tex:5"), "{bad}: {err}");
            assert!(err.contains("s.py"), "{bad}: {err}");
            assert!(err.contains("malformed lines="), "{bad}: {err}");
            assert!(err.contains("file has 2 lines"), "{bad}: {err}");
        }
    }

    #[test]
    fn range_start_past_the_end_names_the_line_count() {
        let dir = root_with(&[("s.py", "l1\nl2\n")]);
        for spec in ["9-12", "9-", "9"] {
            let err = resolve(
                dir.path(),
                "main.tex",
                5,
                &opts(&[("file", "s.py"), ("lines", spec)]),
                "",
            )
            .unwrap_err()
            .to_string();
            assert!(err.contains("main.tex:5"), "{spec}: {err}");
            assert!(err.contains("s.py"), "{spec}: {err}");
            assert!(err.contains("past the end of the file"), "{spec}: {err}");
            assert!(!err.contains("malformed"), "{spec}: {err}");
            assert!(err.contains("(2 lines)"), "{spec}: {err}");
        }
    }

    #[test]
    fn empty_file_option_is_an_error() {
        let dir = root_with(&[("body.py", "x\n")]);
        let err = resolve(dir.path(), "main.tex", 4, &opts(&[("file", "")]), "x")
            .unwrap_err()
            .to_string();
        assert!(err.contains("main.tex:4"), "{err}");
        assert!(err.contains("file= needs a path"), "{err}");
    }

    #[test]
    fn missing_file_names_the_cause() {
        let dir = root_with(&[]);
        let err = resolve(dir.path(), "ch.tex", 9, &opts(&[("file", "gone.py")]), "")
            .unwrap_err()
            .to_string();
        assert!(err.contains("ch.tex:9"), "{err}");
        assert!(err.contains("gone.py"), "{err}");
        assert!(err.contains("cannot read"), "{err}");
    }

    #[test]
    fn extension_infers_the_language() {
        assert_eq!(extension_lang("run.py"), Some("Python".to_string()));
        assert_eq!(extension_lang("main.rs"), Some("Rust".to_string()));
        assert_eq!(extension_lang("notes.weird"), None);
        assert_eq!(extension_lang("noext"), None);
    }
}
