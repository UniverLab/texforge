//! Caption/label/placement/font-size vocabulary for code listings.

/// The two names a document uses, resolved from language + project overrides.
pub(crate) struct Names {
    pub listing: String,
    pub list: String,
}

/// Language → names. english → Listing / List of Listings;
/// spanish → Listado / Índice de listados; anything else → English.
/// `caption_override` / `list_override` win over the language choice.
pub(crate) fn resolve_names(
    language: &str,
    caption_override: Option<&str>,
    list_override: Option<&str>,
) -> Names {
    let (listing, list) = match language.trim().to_ascii_lowercase().as_str() {
        "spanish" => ("Listado".to_string(), "Índice de listados".to_string()),
        _ => ("Listing".to_string(), "List of Listings".to_string()),
    };
    Names {
        listing: caption_override.unwrap_or(&listing).to_string(),
        list: list_override.unwrap_or(&list).to_string(),
    }
}

/// `size=` values. `Small` is today's size and emits no command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Size {
    Scriptsize,
    Footnotesize,
    Small,
    Normalsize,
}

impl Size {
    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "scriptsize" => Some(Self::Scriptsize),
            "footnotesize" => Some(Self::Footnotesize),
            "small" => Some(Self::Small),
            "normalsize" => Some(Self::Normalsize),
            _ => None,
        }
    }

    /// `Some("\\footnotesize")` … or `None` for the default `Small`.
    pub(crate) fn command(self) -> Option<&'static str> {
        match self {
            Self::Scriptsize => Some("\\scriptsize"),
            Self::Footnotesize => Some("\\footnotesize"),
            Self::Small => None,
            Self::Normalsize => Some("\\normalsize"),
        }
    }

    /// Approximate LaTeX baselineskip at this size, in points.
    pub(crate) fn baseline_pt(self) -> f64 {
        match self {
            Self::Scriptsize => 9.5,
            Self::Footnotesize => 11.0,
            Self::Small => 12.0,
            Self::Normalsize => 14.5,
        }
    }
}

/// `pos=` / `float=` / `placement=`.
pub(crate) enum Placement {
    Inline,
    Float(String),
}

impl Placement {
    /// `Ok(Inline)` for omitted/`H`; `Err(value)` names the offending value.
    pub(crate) fn parse(raw: Option<&str>) -> Result<Self, String> {
        match raw {
            None => Ok(Self::Inline),
            Some(value) => {
                let trimmed = value.trim();
                match trimmed {
                    "H" => Ok(Self::Inline),
                    "h" | "t" | "b" | "p" => Ok(Self::Float(trimmed.to_string())),
                    _ => Err(value.to_string()),
                }
            }
        }
    }
}

/// Conservative LaTeX article text height (pt); documented heuristic.
pub(crate) const TEXT_HEIGHT_PT: f64 = 500.0;
/// Caption line + float separation allowance (pt).
pub(crate) const CAPTION_ALLOWANCE_PT: f64 = 20.0;

/// Does an `n`-line block at `size` plausibly fit a float on one page?
pub(crate) fn fits_on_page(lines: usize, size: Size) -> bool {
    lines as f64 * size.baseline_pt() + CAPTION_ALLOWANCE_PT <= TEXT_HEIGHT_PT
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_names_english_spanish_and_other() {
        let english = resolve_names("english", None, None);
        assert_eq!(english.listing, "Listing");
        assert_eq!(english.list, "List of Listings");
        let spanish = resolve_names("spanish", None, None);
        assert_eq!(spanish.listing, "Listado");
        assert_eq!(spanish.list, "Índice de listados");
        let other = resolve_names("french", None, None);
        assert_eq!(other.listing, "Listing");
        assert_eq!(other.list, "List of Listings");
        let empty = resolve_names("", None, None);
        assert_eq!(empty.listing, "Listing");
    }

    #[test]
    fn resolve_names_overrides_win() {
        let both = resolve_names("spanish", Some("Code"), Some("Codes"));
        assert_eq!(both.listing, "Code");
        assert_eq!(both.list, "Codes");
        let one = resolve_names("english", Some("Snippet"), None);
        assert_eq!(one.listing, "Snippet");
        assert_eq!(one.list, "List of Listings");
    }

    #[test]
    fn size_parse_accepts_the_four_and_rejects_others() {
        assert_eq!(Size::parse("scriptsize"), Some(Size::Scriptsize));
        assert_eq!(Size::parse("footnotesize"), Some(Size::Footnotesize));
        assert_eq!(Size::parse("small"), Some(Size::Small));
        assert_eq!(Size::parse("normalsize"), Some(Size::Normalsize));
        assert_eq!(Size::parse("large"), None);
        assert_eq!(Size::parse(""), None);
        assert_eq!(Size::Small.command(), None);
        assert_eq!(Size::Footnotesize.command(), Some("\\footnotesize"));
    }

    #[test]
    fn placement_parse_maps_h_to_inline_and_letters_to_float() {
        assert!(matches!(Placement::parse(None), Ok(Placement::Inline)));
        assert!(matches!(Placement::parse(Some("H")), Ok(Placement::Inline)));
        for pos in ["h", "t", "b", "p"] {
            let parsed = Placement::parse(Some(pos)).unwrap();
            assert!(matches!(parsed, Placement::Float(_)));
        }
        assert!(Placement::parse(Some("Z")).is_err());
    }

    #[test]
    fn fits_on_page_rejects_a_very_tall_block_and_accepts_a_short_one() {
        assert!(fits_on_page(3, Size::Small));
        assert!(!fits_on_page(10_000, Size::Small));
        // `normalsize` fits fewer lines than `scriptsize`.
        let mut script_lines = 0;
        while fits_on_page(script_lines + 1, Size::Scriptsize) {
            script_lines += 1;
        }
        let mut normal_lines = 0;
        while fits_on_page(normal_lines + 1, Size::Normalsize) {
            normal_lines += 1;
        }
        assert!(normal_lines < script_lines);
    }
}
