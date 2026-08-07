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
