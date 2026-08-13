//! Shared helpers for scanning LaTeX text.
//!
//! These were extracted from [`crate::linter`] so that the linter and the
//! tokenizer ([`crate::texparse`]) agree on comment stripping and `\input`
//! traversal instead of maintaining two independent copies.

use std::path::{Path, PathBuf};

/// Result of a recursive `\input` traversal.
#[derive(Debug, Default)]
pub struct TexFileCollection {
    /// All `.tex` files reachable from the entry point, in traversal order.
    pub files: Vec<PathBuf>,
    /// `\input` targets that referenced an already-visited file. Each entry is
    /// the `(entry, resolved_path)` pair that triggered the cycle.
    pub circular: Vec<(String, PathBuf)>,
}

/// Resolve a tex input path, adding `.tex` extension if missing.
pub fn resolve_tex_path(root: &Path, input: &str) -> PathBuf {
    let p = root.join(input);
    if p.extension().is_some() {
        p
    } else {
        p.with_extension("tex")
    }
}

/// Remove empty LaTeX groups (`{}`) from a source token. They produce no
/// glyph — `workf{}lows` is the recommended fix for the ligature `workflows`,
/// so it must be searched for as `workflows`, not penalized for following
/// the tool's own suggestion. Shared by the PDF fidelity check and the spell
/// checker so both treat the idiom the same way.
pub fn strip_empty_groups(word: &str) -> String {
    word.replace("{}", "")
}

/// Strip a LaTeX comment from a line: everything after an unescaped `%`.
pub fn strip_comment(line: &str) -> String {
    let mut result = String::with_capacity(line.len());
    let mut prev_backslash = false;

    for c in line.chars() {
        if c == '%' && !prev_backslash {
            break;
        }
        prev_backslash = c == '\\';
        result.push(c);
    }

    result
}

/// Extract arguments from `\command{arg}` and `\command[opts]{arg}` occurrences in a line.
pub fn extract_commands<'a>(line: &'a str, cmd: &str) -> Vec<&'a str> {
    let mut results = Vec::new();
    let pattern = format!("\\{}", cmd);
    let mut search = line;

    while let Some(pos) = search.find(&pattern) {
        let after = &search[pos + pattern.len()..];
        // Skip optional args [...]
        let after = if after.starts_with('[') {
            match after.find(']') {
                Some(end) => &after[end + 1..],
                None => break,
            }
        } else {
            after
        };
        if after.starts_with('{') {
            if let Some(end) = after.find('}') {
                let arg = after[1..end].trim();
                if !arg.is_empty() {
                    results.push(arg);
                }
                search = &after[end + 1..];
                continue;
            }
        }
        search = after;
    }

    results
}

/// Recursively collect `.tex` files referenced by `\input{}` from `entry`.
pub fn collect_tex_files(root: &Path, entry: &str) -> TexFileCollection {
    let mut collection = TexFileCollection::default();
    collect_tex_files_inner(root, entry, &mut collection);
    collection
}

fn collect_tex_files_inner(root: &Path, entry: &str, collection: &mut TexFileCollection) {
    let path = resolve_tex_path(root, entry);
    if !path.exists() {
        return;
    }
    if collection.files.contains(&path) {
        collection.circular.push((entry.to_string(), path));
        return;
    }
    collection.files.push(path.clone());

    if let Ok(content) = std::fs::read_to_string(&path) {
        for line in content.lines() {
            let line = strip_comment(line);
            for input in extract_commands(&line, "input") {
                collect_tex_files_inner(root, input, collection);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_empty_groups_joins_the_ligature_workaround() {
        assert_eq!(strip_empty_groups("workf{}lows"), "workflows");
        assert_eq!(strip_empty_groups("Artif{}icial"), "Artificial");
        assert_eq!(strip_empty_groups("MLf{}low"), "MLflow");
    }

    #[test]
    fn strip_empty_groups_handles_leading_trailing_and_doubled_groups() {
        assert_eq!(strip_empty_groups("{}Word"), "Word");
        assert_eq!(strip_empty_groups("Word{}"), "Word");
        assert_eq!(strip_empty_groups("Mid{}dle{}Point"), "MiddlePoint");
    }

    #[test]
    fn strip_empty_groups_leaves_non_empty_groups_alone() {
        assert_eq!(strip_empty_groups("\\textit{foo}"), "\\textit{foo}");
        assert_eq!(strip_empty_groups("plain"), "plain");
    }
}
