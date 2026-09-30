//! Build-copy → source line map for warning attribution.
//!
//! Tectonic reports warnings against the finished build copy (after the
//! listing pass rewrote blocks and injected the preamble), which is not the
//! line the author sees. While rewriting, the pass records for each output
//! line the pass-input line it came from; [`LineMap`] serves those lookups.
//! Files the pass never touched need no entry — absent means identity.

use std::collections::HashMap;

/// Maps finished-build-copy lines back to the coordinates the pass received:
/// relative path → finished line (1-based) → pass-input line (1-based).
#[derive(Debug, Default)]
pub struct LineMap {
    files: HashMap<String, Vec<usize>>,
}

impl LineMap {
    /// The origin vector for `file`, creating it on first use.
    pub(crate) fn file_mut(&mut self, file: &str) -> &mut Vec<usize> {
        self.files.entry(file.to_string()).or_default()
    }

    /// Look up the source line for finished-copy `file:line`. The file name
    /// is normalised (a leading `./` is stripped; when it has no `.tex`-like
    /// extension `file + ".tex"` is tried) because Tectonic prints `\input`
    /// targets without their extension. Returns the mapped file key and the
    /// source line, or `None` when the file was never rewritten.
    pub fn get(&self, file: &str, line: usize) -> Option<(&str, usize)> {
        let key = self.normalize(file)?;
        let vec = self.files.get(key)?;
        if line == 0 || line > vec.len() {
            return None;
        }
        Some((key, vec[line - 1]))
    }

    /// Whether any file was mapped at all.
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// Shift one file's map for `injected_lines` preamble lines inserted
    /// before its `anchor` line (1-based, in post-rewrite coordinates):
    /// lines above the anchor keep their mapping, the injected lines point
    /// at the anchor itself, and everything at or below it moves down.
    /// Files without an entry are identity above and below the anchor.
    pub(crate) fn shift_for_injection(&mut self, file: &str, anchor: usize, injected_lines: usize) {
        let old_len = self.files.get(file).map(Vec::len).unwrap_or(0);
        let old = |line: usize| -> usize {
            self.files
                .get(file)
                .and_then(|vec| vec.get(line - 1).copied())
                .unwrap_or(line)
        };
        let anchor_source = if anchor == 0 { 1 } else { old(anchor) };
        let mut shifted = Vec::with_capacity(old_len + injected_lines);
        for final_line in 1..=old_len + injected_lines {
            if final_line < anchor {
                shifted.push(old(final_line));
            } else if final_line < anchor + injected_lines {
                shifted.push(anchor_source);
            } else {
                shifted.push(old(final_line - injected_lines));
            }
        }
        *self.file_mut(file) = shifted;
    }

    fn normalize(&self, file: &str) -> Option<&str> {
        let stripped = file.strip_prefix("./").unwrap_or(file);
        if self.files.contains_key(stripped) {
            return self.files.get_key_value(stripped).map(|(k, _)| k.as_str());
        }
        if !has_tex_like_extension(stripped) {
            let with_ext = format!("{stripped}.tex");
            if self.files.contains_key(&with_ext) {
                return self.files.get_key_value(&with_ext).map(|(k, _)| k.as_str());
            }
        }
        None
    }
}

/// Whether `file` already names a TeX-like source (extension match is
/// case-insensitive, mirroring the compiler's file-open heuristic).
fn has_tex_like_extension(file: &str) -> bool {
    let lower = file.to_ascii_lowercase();
    [".tex", ".sty", ".cls", ".def", ".cfg", ".bib"]
        .iter()
        .any(|ext| lower.ends_with(ext))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unmapped_files_are_identity() {
        let map = LineMap::default();
        assert!(map.is_empty());
        assert_eq!(map.get("main.tex", 12), None);
    }

    #[test]
    fn mapped_lines_round_trip() {
        let mut map = LineMap::default();
        map.file_mut("main.tex").extend([1, 2, 5, 5, 6]);
        assert_eq!(map.get("main.tex", 3), Some(("main.tex", 5)));
        assert_eq!(map.get("main.tex", 6), None);
        assert_eq!(map.get("main.tex", 0), None);
    }

    #[test]
    fn extensionless_and_dotted_lookups_normalize() {
        let mut map = LineMap::default();
        map.file_mut("body.tex").extend([1, 4]);
        assert_eq!(map.get("body", 2), Some(("body.tex", 4)));
        assert_eq!(map.get("./body", 2), Some(("body.tex", 4)));
        assert_eq!(map.get("./body.tex", 1), Some(("body.tex", 1)));
        assert_eq!(map.get("other", 1), None);
    }

    #[test]
    fn injection_shift_moves_lines_below_the_anchor() {
        let mut map = LineMap::default();
        map.file_mut("main.tex").extend([1, 2, 3, 6, 7]);
        map.shift_for_injection("main.tex", 3, 2);
        assert_eq!(map.get("main.tex", 1), Some(("main.tex", 1)));
        assert_eq!(map.get("main.tex", 2), Some(("main.tex", 2)));
        // The two injected lines point at the anchor's own source line.
        assert_eq!(map.get("main.tex", 3), Some(("main.tex", 3)));
        assert_eq!(map.get("main.tex", 4), Some(("main.tex", 3)));
        // Everything at or below the anchor moved down by two.
        assert_eq!(map.get("main.tex", 5), Some(("main.tex", 3)));
        assert_eq!(map.get("main.tex", 6), Some(("main.tex", 6)));
    }
}
