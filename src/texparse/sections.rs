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
}
