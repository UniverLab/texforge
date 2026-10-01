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
    ///
    /// `file` is normalised like [`Self::get`] does for lookups (a leading
    /// `./` is stripped): the pass records origins under the path it
    /// collected (`main.tex`), while the caller shifts under the configured
    /// entry spelling (`./main.tex`) — without this the two would miss and
    /// every warning below the anchor would keep its build-copy line.
    ///
    /// `file_lines` is how many lines the file had *before* injection. It
    /// matters for the entry file when no block of its own was rewritten:
    /// then the pass recorded no origins, so the map for that file is empty.
    /// Sizing the shifted vector from the map alone would cover only the
    /// injected lines and leave every real line below the anchor unmapped
    /// (identity, so the engine's build-copy line would be reported as the
    /// source line). A recorded map is already at least as long as the file,
    /// so `max` keeps the old behaviour there.
    pub(crate) fn shift_for_injection(
        &mut self,
        file: &str,
        anchor: usize,
        injected_lines: usize,
        file_lines: usize,
    ) {
        let file = file.strip_prefix("./").unwrap_or(file);
        let old_len = self
            .files
            .get(file)
            .map_or(file_lines, |vec| vec.len().max(file_lines));
        let old = |line: usize| -> usize {
            self.files
                .get(file)
                .and_then(|vec| vec.get(line - 1).copied())
                .unwrap_or(line)
        };
        let mut shifted = Vec::with_capacity(old_len + injected_lines);
        // One mapping, no region branches: lines above the anchor read
        // their own slot, the injected block and the anchor line read the
        // anchor's slot, and everything below reads the slot above the
        // block. (`anchor` is 1-based — it comes from `inject_entry`'s
        // line count — so the request below can never be line zero.)
        for final_line in 1..=old_len + injected_lines {
            let request = final_line
                .saturating_sub(injected_lines)
                .max(anchor)
                .min(final_line);
            shifted.push(old(request));
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
        assert!(!map.is_empty());
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

    /// The `.tex`-likeness guard: Tectonic prints `\input` targets without
    /// their extension, so an extensionless lookup retries with `.tex` — but
    /// a name that already carries a TeX-like extension must never gain a
    /// second one.
    #[test]
    fn tex_like_extension_names_tex_sources_case_insensitively() {
        for name in [
            "main.tex", "MAIN.TEX", "pkg.sty", "cls.cls", "defs.def", "x.cfg", "refs.bib",
        ] {
            assert!(has_tex_like_extension(name), "{name} is a TeX source");
        }
        for name in ["body", "image.png", "main.pdf", "main.texx", "tex"] {
            assert!(!has_tex_like_extension(name), "{name} is not a TeX source");
        }
    }

    #[test]
    fn injection_shift_moves_lines_below_the_anchor() {
        let mut map = LineMap::default();
        map.file_mut("main.tex").extend([1, 2, 3, 6, 7]);
        map.shift_for_injection("main.tex", 3, 2, 5);
        assert_eq!(map.get("main.tex", 1), Some(("main.tex", 1)));
        assert_eq!(map.get("main.tex", 2), Some(("main.tex", 2)));
        // The two injected lines point at the anchor's own source line.
        assert_eq!(map.get("main.tex", 3), Some(("main.tex", 3)));
        assert_eq!(map.get("main.tex", 4), Some(("main.tex", 3)));
        // Everything at or below the anchor moved down by two.
        assert_eq!(map.get("main.tex", 5), Some(("main.tex", 3)));
        assert_eq!(map.get("main.tex", 6), Some(("main.tex", 6)));
    }

    /// The configured entry may read `./main.tex` while the origins were
    /// recorded under the collected path `main.tex`; the shift must find
    /// the same entry or every warning below the anchor keeps its
    /// build-copy line.
    #[test]
    fn injection_shift_accepts_a_dot_slash_entry_spelling() {
        let mut map = LineMap::default();
        map.file_mut("main.tex").extend([1, 2, 3, 6, 7]);
        map.shift_for_injection("./main.tex", 3, 2, 5);
        assert_eq!(map.get("main.tex", 1), Some(("main.tex", 1)));
        assert_eq!(map.get("main.tex", 4), Some(("main.tex", 3)));
        assert_eq!(map.get("main.tex", 6), Some(("main.tex", 6)));
        assert_eq!(map.get("main.tex", 7), Some(("main.tex", 7)));
        // One entry only: the normalised spelling must not fork a second.
        assert!(!map.files.contains_key("./main.tex"));
    }

    /// The shifted map has exactly one entry per final line: the file's own
    /// lines plus the injected preamble block — never more.
    #[test]
    fn injection_shift_sizes_the_map_to_the_file_plus_the_injected_lines() {
        let mut map = LineMap::default();
        map.file_mut("main.tex").extend([1, 2, 3, 6, 7]);
        map.shift_for_injection("main.tex", 3, 2, 5);
        assert_eq!(
            map.files["main.tex"].len(),
            5 + 2,
            "one entry per final line"
        );
        assert_eq!(map.get("main.tex", 7), Some(("main.tex", 7)));
        assert_eq!(
            map.get("main.tex", 8),
            None,
            "nothing beyond the final line"
        );
    }

    /// The entry file need not contain a code block: when the only block
    /// lives in an `\input`, the pass recorded no origins for the entry, yet
    /// the preamble still injected there shifts its lines. The shift must
    /// still size the map from the file's own length, or every real line
    /// below the anchor stays unmapped and its warning keeps the build-copy
    /// line.
    #[test]
    fn injection_shift_covers_a_file_with_no_recorded_origins() {
        let mut map = LineMap::default();
        // 5-line file, anchor at line 2 (`\begin{document}`), 10 lines injected.
        map.shift_for_injection("main.tex", 2, 10, 5);
        assert_eq!(map.get("main.tex", 1), Some(("main.tex", 1)));
        // Injected lines and the anchor itself point at the anchor's source line.
        assert_eq!(map.get("main.tex", 5), Some(("main.tex", 2)));
        assert_eq!(map.get("main.tex", 11), Some(("main.tex", 2)));
        // Real lines below the anchor moved down by ten, not left unmapped.
        assert_eq!(map.get("main.tex", 12), Some(("main.tex", 2)));
        assert_eq!(map.get("main.tex", 15), Some(("main.tex", 5)));
    }
}
