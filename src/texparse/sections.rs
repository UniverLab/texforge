/// Nesting level for a sectioning command name (a trailing `*` is ignored).
pub(super) fn section_level(name: &str) -> Option<u8> {
    match name.trim_end_matches('*') {
        "part" => Some(0),
        "chapter" => Some(1),
        "section" => Some(2),
        "subsection" => Some(3),
        "subsubsection" => Some(4),
        "paragraph" => Some(5),
        "subparagraph" => Some(6),
        _ => None,
    }
}

/// English text for the title macros LaTeX defines for standard headings
/// (`\abstractname`, `\contentsname`, ...): a title that is exactly one of
/// these control sequences resolves to its text.
pub(crate) const TITLE_NAME_MACROS: &[(&str, &str)] = &[
    ("abstractname", "Abstract"),
    ("contentsname", "Contents"),
    ("refname", "References"),
    ("bibname", "Bibliography"),
    ("listfigurename", "List of Figures"),
    ("listtablename", "List of Tables"),
    ("indexname", "Index"),
    ("appendixname", "Appendix"),
    ("figurename", "Figure"),
    ("tablename", "Table"),
    ("partname", "Part"),
    ("chaptername", "Chapter"),
];

/// Resolve a raw section title to display text.
///
/// A title that is exactly one alphabetic control sequence looks it up in
/// [`TITLE_NAME_MACROS`], falling back to the title as written for unknown
/// names; anything else (plain text, mixed text and macros, escaped
/// specials like `\&`) goes through [`super::title::resolve_section_title`].
pub(super) fn resolve_title(raw: &str) -> String {
    match sole_macro_name(raw) {
        Some(name) => TITLE_NAME_MACROS
            .iter()
            .find(|(macro_name, _)| *macro_name == name)
            .map(|(_, text)| (*text).to_string())
            .unwrap_or_else(|| raw.trim().to_string()),
        None => super::title::resolve_section_title(raw),
    }
}

/// The macro name when `raw` is exactly one control sequence such as
/// `\abstractname`: trimmed, a leading `\`, then one or more ASCII
/// alphabetic characters and nothing else.
fn sole_macro_name(raw: &str) -> Option<&str> {
    let trimmed = raw.trim();
    let name = trimmed.strip_prefix('\\')?;
    if !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphabetic()) {
        Some(name)
    } else {
        None
    }
}

/// Produces dotted section numbers such as `1`, `1.1` and `2.1.1`.
///
/// One counter per level, mirroring the tokenizer's section hierarchy
/// ([`section_level`]: `part` = 0, `chapter` = 1, `section` = 2, ...). Entering
/// a level bumps its counter and resets every deeper one. Leading zero counters
/// are dropped from the printed number, so a document that only uses
/// `\section` numbers its sections `1`, `2`, ... rather than `0.0.1`.
pub struct SectionTracker {
    counters: Vec<usize>,
}

impl SectionTracker {
    /// Create a tracker covering levels `0..=max_level`.
    pub fn new(max_level: u8) -> Self {
        Self {
            counters: vec![0; max_level as usize + 1],
        }
    }

    /// Enter a section at `level`, returning its dotted number.
    pub fn enter(&mut self, level: u8) -> String {
        let level = level as usize;
        self.counters[level] += 1;
        for counter in &mut self.counters[level + 1..] {
            *counter = 0;
        }
        let parts: Vec<String> = self.counters[..=level]
            .iter()
            .map(|counter| counter.to_string())
            .collect();
        let start = first_significant(&parts);
        parts[start..].join(".")
    }
}

/// Index of the first part that is not a zero counter — the point where the
/// dotted number starts. A number whose counters are all zero keeps its last
/// part rather than losing the whole thing.
fn first_significant(parts: &[String]) -> usize {
    parts
        .iter()
        .position(|part| part != "0")
        .unwrap_or(parts.len() - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn section_only_document_numbers_without_leading_zeros() {
        let mut tracker = SectionTracker::new(6);
        assert_eq!(tracker.enter(2), "1");
        assert_eq!(tracker.enter(2), "2");
    }

    #[test]
    fn nested_levels_produce_dotted_numbers() {
        let mut tracker = SectionTracker::new(6);
        assert_eq!(tracker.enter(2), "1");
        assert_eq!(tracker.enter(3), "1.1");
        assert_eq!(tracker.enter(3), "1.2");
        assert_eq!(tracker.enter(2), "2");
    }

    #[test]
    fn entering_a_level_resets_deeper_counters() {
        let mut tracker = SectionTracker::new(6);
        assert_eq!(tracker.enter(2), "1");
        assert_eq!(tracker.enter(3), "1.1");
        assert_eq!(tracker.enter(2), "2");
        assert_eq!(tracker.enter(3), "2.1");
    }

    #[test]
    fn subsection_only_document_drops_leading_zeros() {
        let mut tracker = SectionTracker::new(6);
        assert_eq!(tracker.enter(3), "1");
        assert_eq!(tracker.enter(3), "2");
    }

    /// Every counter at zero keeps the last part: the dotted number must not
    /// vanish entirely.
    #[test]
    fn all_zero_parts_fall_back_to_the_last_one() {
        assert_eq!(first_significant(&["0".into(), "0".into()]), 1);
        assert_eq!(first_significant(&["0".into()]), 0);
    }

    #[test]
    fn the_first_nonzero_part_starts_the_number() {
        assert_eq!(first_significant(&["0".into(), "3".into()]), 1);
        assert_eq!(first_significant(&["2".into(), "0".into()]), 0);
    }

    #[test]
    fn every_name_macro_resolves_to_its_english_text() {
        for (name, text) in TITLE_NAME_MACROS {
            let raw = format!("\\{name}");
            assert_eq!(resolve_title(&raw), (*text).to_string(), "macro: {raw}");
        }
    }

    #[test]
    fn unknown_name_macro_shows_as_written() {
        assert_eq!(resolve_title(r"\unknown"), r"\unknown");
        assert_eq!(resolve_title(r"\mytitle"), r"\mytitle");
    }

    #[test]
    fn mixed_text_and_macro_is_not_a_name_lookup() {
        let raw = r"Intro \abstractname";
        assert_eq!(
            resolve_title(raw),
            crate::texparse::title::resolve_section_title(raw)
        );
        assert_ne!(resolve_title(raw), "Intro Abstract");
    }

    #[test]
    fn escaped_specials_keep_their_established_resolution() {
        assert_eq!(resolve_title(r"\&"), "&");
    }
}
