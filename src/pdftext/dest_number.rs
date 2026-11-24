//! Printed section numbers carried by hyperref destination names.
//!
//! The bookmarks `hyperref` writes point at named destinations whose names
//! encode the number LaTeX printed in the PDF: `chapter.1` is the first
//! numbered chapter, `section.1.1` its first section, `chapter*.1` an
//! unnumbered chapter, and `appendix.Alph1` the first appendix. Reading the
//! number back out of the name keeps `texforge pdf pages` in agreement with
//! the printed page instead of re-counting the bookmark tree.

/// How a destination name numbers its outline entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DestNumber {
    /// The number printed in the PDF (`appendix.Alph1` -> `A`).
    Numbered(String),
    /// A starred (unnumbered) heading: it still opens its page.
    Unnumbered,
    /// Not a hyperref section destination (`Doc-Start`, `page.3`, ...).
    Unknown,
}

/// Sectioning kinds `hyperref` writes destinations for, including the
/// chapter/section-level heading opened after `\appendix`.
const KNOWN_KINDS: &[&str] = &[
    "part",
    "chapter",
    "section",
    "subsection",
    "subsubsection",
    "paragraph",
    "subparagraph",
    "appendix",
];

/// Recover the printed section number from a hyperref destination name.
///
/// `chapter.1` -> `Numbered("1")`, `chapter*.1` -> `Unnumbered`,
/// `appendix.Alph1` -> `Numbered("A")`; anything else -> `Unknown`.
pub fn number_from_dest(name: &str) -> DestNumber {
    let (head, rest) = match name.split_once('.') {
        Some((head, rest)) if !rest.is_empty() => (head, rest),
        _ => return DestNumber::Unknown,
    };
    let (kind, starred) = match head.strip_suffix('*') {
        Some(kind) => (kind, true),
        None => (head, false),
    };
    if !KNOWN_KINDS.contains(&kind) {
        return DestNumber::Unknown;
    }
    if starred {
        return DestNumber::Unnumbered;
    }
    DestNumber::Numbered(
        rest.split('.')
            .map(map_segment)
            .collect::<Vec<_>>()
            .join("."),
    )
}

/// Map one dotted segment to its printed form: `Alph<k>`/`alph<k>` count in
/// letters (`27` -> `AA`), `Roman<k>`/`roman<k>` in numerals, anything else
/// (arabic figures, `II`, ...) is kept as written.
fn map_segment(segment: &str) -> String {
    if let Some(number) = counter_value(segment, "Alph") {
        return column_letters(number);
    }
    if let Some(number) = counter_value(segment, "alph") {
        return column_letters(number).to_lowercase();
    }
    if let Some(number) = counter_value(segment, "Roman") {
        return roman_numeral(number);
    }
    if let Some(number) = counter_value(segment, "roman") {
        return roman_numeral(number).to_lowercase();
    }
    segment.to_string()
}

/// The counter value behind a `prefix<k>` segment, when `k` parses and the
/// segment carries nothing else (`Alph1` yes, `Alph` or `Alph0` no).
fn counter_value(segment: &str, prefix: &str) -> Option<usize> {
    let digits = segment.strip_prefix(prefix)?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let value: usize = digits.parse().ok()?;
    (value >= 1).then_some(value)
}

/// Spreadsheet-style column letters: `1` -> `A`, `26` -> `Z`, `27` -> `AA`.
pub(crate) fn column_letters(mut value: usize) -> String {
    let mut letters = Vec::new();
    while value > 0 {
        value -= 1;
        letters.push((b'A' + (value % 26) as u8) as char);
        value /= 26;
    }
    letters.iter().rev().collect()
}

/// Uppercase roman numeral for a positive counter (`4` -> `IV`).
fn roman_numeral(mut value: usize) -> String {
    const TABLE: &[(usize, &str)] = &[
        (1000, "M"),
        (900, "CM"),
        (500, "D"),
        (400, "CD"),
        (100, "C"),
        (90, "XC"),
        (50, "L"),
        (40, "XL"),
        (10, "X"),
        (9, "IX"),
        (5, "V"),
        (4, "IV"),
        (1, "I"),
    ];
    let mut out = String::new();
    for (arabic, roman) in TABLE {
        while value >= *arabic {
            out.push_str(roman);
            value -= arabic;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starred_chapters_are_unnumbered() {
        assert_eq!(number_from_dest("chapter*.1"), DestNumber::Unnumbered);
        assert_eq!(number_from_dest("chapter*.36"), DestNumber::Unnumbered);
    }

    #[test]
    fn arabic_segments_pass_through() {
        assert_eq!(
            number_from_dest("chapter.3"),
            DestNumber::Numbered("3".into())
        );
        assert_eq!(
            number_from_dest("section.1.1"),
            DestNumber::Numbered("1.1".into())
        );
        assert_eq!(
            number_from_dest("part.II"),
            DestNumber::Numbered("II".into())
        );
    }

    #[test]
    fn alph_counters_become_letters() {
        assert_eq!(
            number_from_dest("appendix.Alph1"),
            DestNumber::Numbered("A".into())
        );
        assert_eq!(
            number_from_dest("appendix.Alph27"),
            DestNumber::Numbered("AA".into())
        );
        assert_eq!(
            number_from_dest("section.Alph1.2"),
            DestNumber::Numbered("A.2".into())
        );
    }

    #[test]
    fn roman_counters_become_numerals() {
        assert_eq!(
            number_from_dest("chapter.Roman2"),
            DestNumber::Numbered("II".into())
        );
        assert_eq!(
            number_from_dest("chapter.roman3"),
            DestNumber::Numbered("iii".into())
        );
        assert_eq!(
            number_from_dest("section.alph2"),
            DestNumber::Numbered("b".into())
        );
    }

    #[test]
    fn non_section_destinations_are_unknown() {
        assert_eq!(number_from_dest("Doc-Start"), DestNumber::Unknown);
        assert_eq!(number_from_dest("page.3"), DestNumber::Unknown);
        assert_eq!(number_from_dest("cite.x"), DestNumber::Unknown);
        assert_eq!(number_from_dest("chapter"), DestNumber::Unknown);
        assert_eq!(number_from_dest("chapter."), DestNumber::Unknown);
        assert_eq!(number_from_dest(""), DestNumber::Unknown);
    }

    #[test]
    fn columns_roll_past_z() {
        assert_eq!(column_letters(1), "A");
        assert_eq!(column_letters(26), "Z");
        assert_eq!(column_letters(27), "AA");
        assert_eq!(column_letters(28), "AB");
    }
}
