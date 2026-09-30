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
        let start = parts
            .iter()
            .position(|part| part != "0")
            .unwrap_or(parts.len() - 1);
        parts[start..].join(".")
    }
}
