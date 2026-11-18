use syntect::highlighting::FontStyle as SyntectFont;

use super::palette::Category;

/// Font emphasis for the `ink` palette: bold for keywords/numbers, bold-italic
/// for names, italic for comments, regular otherwise.
pub(crate) fn font_style(category: Category) -> Option<SyntectFont> {
    match category {
        Category::Keyword => Some(SyntectFont::BOLD),
        Category::Constant => Some(SyntectFont::BOLD),
        Category::Function | Category::Class => Some(SyntectFont::BOLD | SyntectFont::ITALIC),
        Category::Comment => Some(SyntectFont::ITALIC),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every category maps to its ink emphasis: keywords and constants bold,
    /// functions and classes bold-italic, comments italic, all else regular.
    #[test]
    fn every_category_maps_to_its_ink_emphasis() {
        assert_eq!(font_style(Category::Keyword), Some(SyntectFont::BOLD));
        assert_eq!(font_style(Category::Constant), Some(SyntectFont::BOLD));
        assert_eq!(
            font_style(Category::Function),
            Some(SyntectFont::BOLD | SyntectFont::ITALIC)
        );
        assert_eq!(
            font_style(Category::Class),
            Some(SyntectFont::BOLD | SyntectFont::ITALIC)
        );
        assert_eq!(font_style(Category::Comment), Some(SyntectFont::ITALIC));
        for category in [
            Category::String,
            Category::Tag,
            Category::Attribute,
            Category::Invalid,
            Category::Inserted,
            Category::Deleted,
            Category::Changed,
            Category::Raw,
        ] {
            assert_eq!(font_style(category), None, "{category:?}");
        }
    }
}
