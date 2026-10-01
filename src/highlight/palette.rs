//! Highlight palettes: themes, styles and the scope→colour tables they share.
//!
//! Two orthogonal knobs pick a palette:
//!
//! * the **theme** (`github`, `one-light`) names the *palette family* —
//!   which syntax colours the scopes get;
//! * the **style** (`light`, `light-mono`, `dark`, `dark-mono`) picks the
//!   *register*: light or dark, and whether hue is allowed at all.
//!
//! Every value is a Rust constant, so nothing is loaded at runtime. The
//! syntect [`Theme`] each palette needs is built once per process and cached
//! in [`THEMES`], keyed on both knobs.
//!
//! Byte-identity matters for `light`: it is the rendering every existing
//! document gets today, down to the fixed `tfxtint`/`tfxframe`/`tfxgutter`
//! colour names. The other styles emit ordinary `tfxcol<hex>` names instead,
//! because a document may mix styles and the frame names cannot be shared
//! then (see [`Palette::block_style`]).

use std::collections::BTreeSet;
use std::sync::OnceLock;

use anyhow::{bail, Result};
use syntect::highlighting::{
    FontStyle as SyntectFont, StyleModifier, Theme, ThemeItem, ThemeSettings,
};

use super::engine::Rgb;

/// The palette families. Both are light: a dark treatment is a *style*
/// (`dark` / `dark-mono`), never a theme.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HighlightTheme {
    Github,
    OneLight,
}

impl Default for HighlightTheme {
    /// `github` — the documented default.
    fn default() -> Self {
        Self::Github
    }
}

/// The valid theme names, in the order errors should list them.
pub const VALID_THEME_NAMES: [&str; 2] = ["github", "one-light"];

impl HighlightTheme {
    /// Parse a theme name from `project.toml`'s `[highlight] theme`. An
    /// unknown name fails the build, naming the offending value and every
    /// valid alternative — mirroring `DiagramStyle::parse`.
    pub fn parse(name: &str) -> Result<Self> {
        match name {
            "github" => Ok(Self::Github),
            "one-light" => Ok(Self::OneLight),
            other => bail!(
                "Unknown code theme '{other}' — valid themes are: {}",
                VALID_THEME_NAMES.join(", ")
            ),
        }
    }

    fn index(self) -> usize {
        match self {
            Self::Github => 0,
            Self::OneLight => 1,
        }
    }
}

/// The four rendering styles. `Light` is today's rendering, unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HighlightStyle {
    /// Today's rendering: the theme's own palette on a light frame.
    Light,
    /// The theme family is ignored: greys only, for black-and-white
    /// printing. Keywords bold, comments italic.
    LightMono,
    /// The dark twin of the theme family, on a dark frame.
    Dark,
    /// Greys only, on a dark frame — a terminal that survives a mono printer.
    DarkMono,
}

impl Default for HighlightStyle {
    /// `light` — the documented default, and byte-identical to a build
    /// before styles existed.
    fn default() -> Self {
        Self::Light
    }
}

/// The valid style names, in the order errors should list them.
pub const VALID_STYLE_NAMES: [&str; 4] = ["light", "light-mono", "dark", "dark-mono"];

impl HighlightStyle {
    /// Parse a style name from `style=`, `[highlight] style` or a
    /// `[highlight.by_lang]` value. An unknown name fails the build, naming
    /// the offending value and every valid alternative.
    pub fn parse(name: &str) -> Result<Self> {
        match name {
            "light" => Ok(Self::Light),
            "light-mono" => Ok(Self::LightMono),
            "dark" => Ok(Self::Dark),
            "dark-mono" => Ok(Self::DarkMono),
            other => bail!(
                "Unknown code style '{other}' — valid styles are: {}",
                VALID_STYLE_NAMES.join(", ")
            ),
        }
    }

    /// The cache slot: styles are ordered as documented, and each theme gets
    /// its own four slots.
    fn index(self) -> usize {
        match self {
            Self::Light => 0,
            Self::LightMono => 1,
            Self::Dark => 2,
            Self::DarkMono => 3,
        }
    }

    /// Whether the block needs an explicit base-colour statement.
    ///
    /// `light` inherits the document's black text, which is exactly what
    /// today's output does; a dark frame needs its light foreground spelled
    /// out, or every uncoloured token would print black on the dark tint.
    pub fn paints_base(self) -> bool {
        matches!(self, Self::Dark | Self::DarkMono)
    }
}

/// The font emphasis a token carries. `light` never sets one, so the emitted
/// LaTeX keeps the plain text it has always emitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FontStyle {
    Normal,
    Bold,
    Italic,
}

impl From<SyntectFont> for FontStyle {
    /// syntect allows any combination of the three flags; the listings only
    /// ever set one, and bold wins over italic when both arrive.
    fn from(style: SyntectFont) -> Self {
        if style.contains(SyntectFont::BOLD) {
            Self::Bold
        } else if style.contains(SyntectFont::ITALIC) {
            Self::Italic
        } else {
            Self::Normal
        }
    }
}

/// The token categories the scope table distinguishes. The 22 scope
/// selectors in [`SCOPE_GROUPS`] collapse onto these 13, which is what lets
/// every palette be a flat `[Rgb; 13]` array.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Category {
    Comment,
    String,
    Constant,
    Function,
    Class,
    Keyword,
    Tag,
    Attribute,
    Invalid,
    Inserted,
    Deleted,
    Changed,
    Raw,
}

/// How many palettes' `colors` arrays are indexed by [`Category`].
const CATEGORY_COUNT: usize = 13;

/// Every scope selector the curated syntaxes can match, with the category it
/// colours. This is the scope map every palette shares — the syntaxes are
/// untouched by styles, only the colours behind these groups change.
const SCOPE_GROUPS: &[(&str, Category)] = &[
    ("comment", Category::Comment),
    ("string", Category::String),
    ("constant.numeric", Category::Constant),
    ("constant.language", Category::Constant),
    ("constant.character.escape", Category::Constant),
    ("constant.other", Category::Constant),
    ("support.constant", Category::Constant),
    ("entity.name.function", Category::Function),
    ("support.function", Category::Function),
    ("entity.name.class", Category::Class),
    ("entity.name.type", Category::Class),
    ("support.class", Category::Class),
    ("support.type", Category::Class),
    ("keyword", Category::Keyword),
    ("storage", Category::Keyword),
    ("entity.name.tag", Category::Tag),
    ("entity.other.attribute-name", Category::Attribute),
    ("invalid", Category::Invalid),
    ("markup.inserted", Category::Inserted),
    ("markup.deleted", Category::Deleted),
    ("markup.changed", Category::Changed),
    ("markup.raw", Category::Raw),
];

/// A palette as data: the base foreground every unscoped token inherits, the
/// frame colours, and the scope-group → colour table.
pub(crate) struct Palette {
    name: &'static str,
    /// The colour an unscoped token renders in.
    pub(crate) base: Rgb,
    /// The block background the frame paints.
    pub(crate) tint: Rgb,
    /// The frame hairline.
    pub(crate) frame: Rgb,
    /// The colour of the line numbers (and of the gutter separator rule).
    pub(crate) gutter: Rgb,
    colors: [Rgb; CATEGORY_COUNT],
    /// Greyscale: keywords bold, comments italic.
    mono: bool,
    /// `light` only: the frame uses the fixed `tfxtint`/`tfxframe`/
    /// `tfxgutter` names, so an unchanged document keeps its exact output.
    legacy_light: bool,
}

/// GitHub light syntax colours (`github` tmTheme) — today's rendering.
const GITHUB_LIGHT: Palette = Palette {
    name: "github",
    base: Rgb::new(0x24, 0x29, 0x2e),
    tint: Rgb::new(0xf6, 0xf8, 0xfa),
    frame: Rgb::new(0xc3, 0xc7, 0xcb),
    gutter: Rgb::new(0x6a, 0x73, 0x7d),
    colors: [
        Rgb::new(0x6a, 0x73, 0x7d), // comment
        Rgb::new(0x03, 0x2f, 0x62), // string
        Rgb::new(0x00, 0x5c, 0xc5), // constant
        Rgb::new(0x00, 0x5c, 0xc5), // function
        Rgb::new(0x6f, 0x42, 0xc1), // class
        Rgb::new(0xd7, 0x3a, 0x49), // keyword
        Rgb::new(0x22, 0x86, 0x3a), // tag
        Rgb::new(0x6f, 0x42, 0xc1), // attribute
        Rgb::new(0xb3, 0x1d, 0x28), // invalid
        Rgb::new(0x22, 0x86, 0x3a), // inserted
        Rgb::new(0xb3, 0x1d, 0x28), // deleted
        Rgb::new(0xe3, 0x62, 0x09), // changed
        Rgb::new(0x03, 0x2f, 0x62), // raw
    ],
    mono: false,
    legacy_light: true,
};

/// Atom's One Light syntax colours — today's second theme.
const ONE_LIGHT: Palette = Palette {
    name: "one-light",
    base: Rgb::new(0x38, 0x3a, 0x42),
    tint: Rgb::new(0xfa, 0xfa, 0xfa),
    frame: Rgb::new(0xd9, 0xd9, 0xdc),
    gutter: Rgb::new(0xa0, 0xa1, 0xa7),
    colors: [
        Rgb::new(0xa0, 0xa1, 0xa7), // comment
        Rgb::new(0x50, 0xa1, 0x4f), // string
        Rgb::new(0x98, 0x68, 0x01), // constant
        Rgb::new(0x40, 0x78, 0xf2), // function
        Rgb::new(0xc1, 0x84, 0x01), // class
        Rgb::new(0xa6, 0x26, 0xa4), // keyword
        Rgb::new(0xe4, 0x56, 0x49), // tag
        Rgb::new(0x98, 0x68, 0x01), // attribute
        Rgb::new(0xe4, 0x56, 0x49), // invalid
        Rgb::new(0x50, 0xa1, 0x4f), // inserted
        Rgb::new(0xe4, 0x56, 0x49), // deleted
        Rgb::new(0x98, 0x68, 0x01), // changed
        Rgb::new(0x50, 0xa1, 0x4f), // raw
    ],
    mono: false,
    legacy_light: true,
};

/// GitHub Dark Dimmed. The published comment grey (`#768390`) and danger red
/// (`#f85149`) fall below WCAG AA on this background, so both are lightened
/// here — the same reason a designer would touch a palette before shipping
/// it.
const GITHUB_DARK: Palette = Palette {
    name: "github-dark",
    base: Rgb::new(0xad, 0xba, 0xc7),
    tint: Rgb::new(0x22, 0x27, 0x2e),
    frame: Rgb::new(0x8b, 0x94, 0x9e),
    gutter: Rgb::new(0x8b, 0x94, 0x9e),
    colors: [
        Rgb::new(0x8b, 0x94, 0x9e), // comment
        Rgb::new(0x96, 0xd0, 0xff), // string
        Rgb::new(0x6c, 0xb6, 0xff), // constant
        Rgb::new(0xdc, 0xbd, 0xfb), // function
        Rgb::new(0xdc, 0xbd, 0xfb), // class
        Rgb::new(0xf4, 0x70, 0x67), // keyword
        Rgb::new(0x8d, 0xdb, 0x8c), // tag
        Rgb::new(0xf6, 0x9d, 0x50), // attribute
        Rgb::new(0xff, 0x7b, 0x72), // invalid
        Rgb::new(0x57, 0xab, 0x5a), // inserted
        Rgb::new(0xff, 0x7b, 0x72), // deleted
        Rgb::new(0xda, 0xaa, 0x3f), // changed
        Rgb::new(0x96, 0xd0, 0xff), // raw
    ],
    mono: false,
    legacy_light: false,
};

/// Atom One Dark. Comment grey and red are lightened for AA on `#282c34`.
const ONE_DARK: Palette = Palette {
    name: "one-dark",
    base: Rgb::new(0xab, 0xb2, 0xbf),
    tint: Rgb::new(0x28, 0x2c, 0x34),
    frame: Rgb::new(0x9a, 0xa0, 0xaa),
    gutter: Rgb::new(0x9a, 0xa0, 0xaa),
    colors: [
        Rgb::new(0x9a, 0xa0, 0xaa), // comment
        Rgb::new(0x98, 0xc3, 0x79), // string
        Rgb::new(0xd1, 0x9a, 0x66), // constant
        Rgb::new(0x61, 0xaf, 0xef), // function
        Rgb::new(0xe5, 0xc0, 0x7b), // class
        Rgb::new(0xc6, 0x78, 0xdd), // keyword
        Rgb::new(0xef, 0x80, 0x89), // tag
        Rgb::new(0xd1, 0x9a, 0x66), // attribute
        Rgb::new(0xef, 0x80, 0x89), // invalid
        Rgb::new(0x98, 0xc3, 0x79), // inserted
        Rgb::new(0xef, 0x80, 0x89), // deleted
        Rgb::new(0xd1, 0x9a, 0x66), // changed
        Rgb::new(0x98, 0xc3, 0x79), // raw
    ],
    mono: false,
    legacy_light: false,
};

/// Greys only, on paper white: hue carries no meaning, so weight and slant
/// do the work instead. For black-and-white printing.
const LIGHT_MONO: Palette = Palette {
    name: "light-mono",
    base: Rgb::new(0x00, 0x00, 0x00),
    tint: Rgb::new(0xf6, 0xf6, 0xf6),
    frame: Rgb::new(0xc5, 0xc5, 0xc5),
    gutter: Rgb::new(0x6e, 0x6e, 0x6e),
    colors: [
        Rgb::new(0x6e, 0x6e, 0x6e), // comment
        Rgb::new(0x30, 0x30, 0x30), // string
        Rgb::new(0x00, 0x00, 0x00), // constant
        Rgb::new(0x00, 0x00, 0x00), // function
        Rgb::new(0x33, 0x33, 0x33), // class
        Rgb::new(0x00, 0x00, 0x00), // keyword
        Rgb::new(0x33, 0x33, 0x33), // tag
        Rgb::new(0x44, 0x44, 0x44), // attribute
        Rgb::new(0x00, 0x00, 0x00), // invalid
        Rgb::new(0x33, 0x33, 0x33), // inserted
        Rgb::new(0x33, 0x33, 0x33), // deleted
        Rgb::new(0x33, 0x33, 0x33), // changed
        Rgb::new(0x30, 0x30, 0x30), // raw
    ],
    mono: true,
    legacy_light: false,
};

/// Greys only, on a dark frame: a terminal that stays readable when the
/// printer throws the hue away.
const DARK_MONO: Palette = Palette {
    name: "dark-mono",
    base: Rgb::new(0xe6, 0xe6, 0xe6),
    tint: Rgb::new(0x2b, 0x2b, 0x2b),
    frame: Rgb::new(0x9a, 0x9a, 0x9a),
    gutter: Rgb::new(0x9a, 0x9a, 0x9a),
    colors: [
        Rgb::new(0x9a, 0x9a, 0x9a), // comment
        Rgb::new(0xcf, 0xcf, 0xcf), // string
        Rgb::new(0xd4, 0xd4, 0xd4), // constant
        Rgb::new(0xe6, 0xe6, 0xe6), // function
        Rgb::new(0xe6, 0xe6, 0xe6), // class
        Rgb::new(0xff, 0xff, 0xff), // keyword
        Rgb::new(0xcf, 0xcf, 0xcf), // tag
        Rgb::new(0xc0, 0xc0, 0xc0), // attribute
        Rgb::new(0xff, 0xff, 0xff), // invalid
        Rgb::new(0xcf, 0xcf, 0xcf), // inserted
        Rgb::new(0xcf, 0xcf, 0xcf), // deleted
        Rgb::new(0xcf, 0xcf, 0xcf), // changed
        Rgb::new(0xcf, 0xcf, 0xcf), // raw
    ],
    mono: true,
    legacy_light: false,
};

/// The palette one (theme, style) pair renders with.
///
/// The mono palettes are family-independent on purpose: with hue gone, the
/// two light themes are the same picture, and a document that switches
/// `theme` must not silently switch its printing palette.
pub(crate) fn palette(theme: HighlightTheme, style: HighlightStyle) -> &'static Palette {
    match (theme, style) {
        (_, HighlightStyle::LightMono) => &LIGHT_MONO,
        (_, HighlightStyle::DarkMono) => &DARK_MONO,
        (HighlightTheme::Github, HighlightStyle::Light) => &GITHUB_LIGHT,
        (HighlightTheme::OneLight, HighlightStyle::Light) => &ONE_LIGHT,
        (HighlightTheme::Github, HighlightStyle::Dark) => &GITHUB_DARK,
        (HighlightTheme::OneLight, HighlightStyle::Dark) => &ONE_DARK,
    }
}

/// The LaTeX colour names one block paints its frame with.
///
/// A document may mix styles, so these are per block: `light` keeps the
/// fixed names (byte-identity), every other style names its own colours.
#[derive(Debug, Clone)]
pub(crate) struct BlockColors {
    pub(crate) tint: String,
    pub(crate) frame: String,
    pub(crate) gutter: String,
}

impl BlockColors {
    /// The legacy light frame: the fixed `tfxtint`/`tfxframe`/`tfxgutter`
    /// names every existing document renders with.
    fn legacy() -> Self {
        Self {
            tint: "tfxtint".to_string(),
            frame: "tfxframe".to_string(),
            gutter: "tfxgutter".to_string(),
        }
    }
}

/// What one block needs from its palette: the frame colours to reference
/// and, for a dark style, the base foreground to state inside its group.
#[derive(Debug, Clone)]
pub(crate) struct BlockStyle {
    pub(crate) colors: BlockColors,
    pub(crate) base: Option<Rgb>,
}

impl Palette {
    /// The colour of `category`'s scope group.
    fn color(&self, category: Category) -> Rgb {
        self.colors[category as usize]
    }

    /// Every scope colour, for the palette-wide invariant tests.
    #[cfg(test)]
    fn colors(&self) -> &[Rgb; CATEGORY_COUNT] {
        &self.colors
    }

    /// The frame colours and base foreground for one block, registering
    /// every name it needs in the preamble's `used` set. `numbers` decides
    /// whether the gutter colour is actually referenced (an unused
    /// `\definecolor` is harmless, a used undefined one is not).
    pub(crate) fn block_style(
        &self,
        numbers: bool,
        paints_base: bool,
        used: &mut BTreeSet<Rgb>,
    ) -> BlockStyle {
        if self.legacy_light {
            return BlockStyle {
                colors: BlockColors::legacy(),
                base: None,
            };
        }
        used.insert(self.tint);
        used.insert(self.frame);
        if numbers {
            used.insert(self.gutter);
        }
        BlockStyle {
            colors: BlockColors {
                tint: self.tint.name(),
                frame: self.frame.name(),
                gutter: self.gutter.name(),
            },
            // `light-mono` needs no base statement: its base is the
            // document's own black, exactly as a `light` block's is.
            base: paints_base.then_some(self.base),
        }
    }

    /// The syntect font flag for `category`: only the mono palettes lean on
    /// weight and slant, so the light palettes emit plain text as always.
    fn font_style(&self, category: Category) -> Option<SyntectFont> {
        if !self.mono {
            return None;
        }
        match category {
            Category::Keyword => Some(SyntectFont::BOLD),
            Category::Comment => Some(SyntectFont::ITALIC),
            _ => None,
        }
    }
}

/// Built themes, one slot per (theme, style) pair — `theme.index() * 4 +
/// style.index()`. Both knobs are in the key: indexing one on the other would
/// cross-wire the families.
static THEMES: [OnceLock<Theme>; 8] = [const { OnceLock::new() }; 8];

/// The cached syntect theme for one palette.
pub(crate) fn theme_of(theme: HighlightTheme, style: HighlightStyle) -> &'static Theme {
    let slot = theme.index() * 4 + style.index();
    THEMES[slot].get_or_init(|| build_theme(palette(theme, style)))
}

/// Turn one palette into a syntect [`Theme`]: every scope group becomes a
/// rule with the palette's colour and font flag.
fn build_theme(palette: &Palette) -> Theme {
    let scopes = SCOPE_GROUPS
        .iter()
        .map(|(selector, category)| ThemeItem {
            scope: selector
                .parse()
                .expect("theme scope selectors are compile-time constants"),
            style: StyleModifier {
                foreground: Some(palette.color(*category).into()),
                background: None,
                font_style: palette.font_style(*category),
            },
        })
        .collect();
    Theme {
        name: Some(palette.name.to_string()),
        author: Some("texforge".to_string()),
        settings: ThemeSettings {
            foreground: Some(palette.base.into()),
            // Not emitted anywhere: the frame paints the tint with `color`
            // rules. Kept truthful so a palette round-trips through syntect.
            background: Some(palette.tint.into()),
            ..ThemeSettings::default()
        },
        scopes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// WCAG relative luminance of one colour.
    fn luminance(rgb: Rgb) -> f64 {
        let channel = |value: u8| {
            let c = f64::from(value) / 255.0;
            if c <= 0.040_45 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(rgb.r) + 0.7152 * channel(rgb.g) + 0.0722 * channel(rgb.b)
    }

    /// WCAG contrast ratio of `foreground` against `background`, 1.0…21.0.
    fn contrast_ratio(foreground: Rgb, background: Rgb) -> f64 {
        let (mut high, mut low) = (luminance(foreground), luminance(background));
        if high < low {
            std::mem::swap(&mut high, &mut low);
        }
        (high + 0.05) / (low + 0.05)
    }

    fn styles() -> [(HighlightTheme, HighlightStyle); 6] {
        [
            (HighlightTheme::Github, HighlightStyle::Light),
            (HighlightTheme::OneLight, HighlightStyle::Light),
            (HighlightTheme::Github, HighlightStyle::Dark),
            (HighlightTheme::OneLight, HighlightStyle::Dark),
            (HighlightTheme::Github, HighlightStyle::LightMono),
            (HighlightTheme::OneLight, HighlightStyle::DarkMono),
        ]
    }

    #[test]
    fn style_parse_names_and_error() {
        assert_eq!(
            HighlightStyle::parse("light").unwrap(),
            HighlightStyle::Light
        );
        assert_eq!(
            HighlightStyle::parse("light-mono").unwrap(),
            HighlightStyle::LightMono
        );
        assert_eq!(HighlightStyle::parse("dark").unwrap(), HighlightStyle::Dark);
        assert_eq!(
            HighlightStyle::parse("dark-mono").unwrap(),
            HighlightStyle::DarkMono
        );
        let err = HighlightStyle::parse("neon").unwrap_err().to_string();
        assert!(err.contains("neon"), "{err}");
        for name in VALID_STYLE_NAMES {
            assert!(err.contains(name), "missing {name}: {err}");
        }
    }

    #[test]
    fn theme_parse_names_and_error() {
        assert_eq!(
            HighlightTheme::parse("github").unwrap(),
            HighlightTheme::Github
        );
        assert_eq!(
            HighlightTheme::parse("one-light").unwrap(),
            HighlightTheme::OneLight
        );
        let err = HighlightTheme::parse("dracula").unwrap_err().to_string();
        assert!(err.contains("dracula"), "{err}");
        for name in VALID_THEME_NAMES {
            assert!(err.contains(name), "missing {name}: {err}");
        }
    }

    #[test]
    fn default_style_is_light() {
        assert_eq!(HighlightStyle::default(), HighlightStyle::Light);
        assert_eq!(HighlightTheme::default(), HighlightTheme::Github);
        assert!(!HighlightStyle::default().paints_base());
        assert!(HighlightStyle::Dark.paints_base());
        assert!(HighlightStyle::DarkMono.paints_base());
        // Only the `-mono` styles reach for weight and slant instead of hue.
        assert!(!palette(HighlightTheme::Github, HighlightStyle::Light).mono);
        assert!(!palette(HighlightTheme::Github, HighlightStyle::Dark).mono);
        assert!(palette(HighlightTheme::Github, HighlightStyle::LightMono).mono);
        assert!(palette(HighlightTheme::Github, HighlightStyle::DarkMono).mono);
    }

    /// `light` must stay today's rendering: the frame hairline is still the
    /// comment grey at 40% over white, which is how the value was derived in
    /// the first place.
    #[test]
    fn light_palettes_are_unchanged() {
        let github = palette(HighlightTheme::Github, HighlightStyle::Light);
        assert_eq!(github.tint, Rgb::new(0xf6, 0xf8, 0xfa));
        assert_eq!(github.frame, Rgb::new(0xc3, 0xc7, 0xcb));
        assert_eq!(github.gutter, Rgb::new(0x6a, 0x73, 0x7d));
        assert_eq!(github.base, Rgb::new(0x24, 0x29, 0x2e));
        assert_eq!(github.frame, github.gutter.over_white(40));

        let one = palette(HighlightTheme::OneLight, HighlightStyle::Light);
        assert_eq!(one.tint, Rgb::new(0xfa, 0xfa, 0xfa));
        assert_eq!(one.frame, Rgb::new(0xd9, 0xd9, 0xdc));
        assert_eq!(one.gutter, Rgb::new(0xa0, 0xa1, 0xa7));
        assert_eq!(one.frame, one.gutter.over_white(40));
    }

    #[test]
    fn dark_palettes_use_the_dark_twin() {
        let github = palette(HighlightTheme::Github, HighlightStyle::Dark);
        assert_eq!(github.tint, Rgb::new(0x22, 0x27, 0x2e));
        assert_eq!(github.frame, Rgb::new(0x8b, 0x94, 0x9e));
        assert_eq!(github.gutter, Rgb::new(0x8b, 0x94, 0x9e));
        let one = palette(HighlightTheme::OneLight, HighlightStyle::Dark);
        assert_eq!(one.tint, Rgb::new(0x28, 0x2c, 0x34));
        assert_eq!(one.frame, Rgb::new(0x9a, 0xa0, 0xaa));
        assert_eq!(one.gutter, Rgb::new(0x9a, 0xa0, 0xaa));
    }

    #[test]
    fn mono_palette_values() {
        let light = palette(HighlightTheme::Github, HighlightStyle::LightMono);
        assert_eq!(light.tint, Rgb::new(0xf6, 0xf6, 0xf6));
        assert_eq!(light.base, Rgb::new(0x00, 0x00, 0x00));
        assert_eq!(light.color(Category::Keyword), Rgb::new(0, 0, 0));
        assert_eq!(light.color(Category::Comment), Rgb::new(0x6e, 0x6e, 0x6e));
        assert_eq!(light.color(Category::String), Rgb::new(0x30, 0x30, 0x30));

        let dark = palette(HighlightTheme::Github, HighlightStyle::DarkMono);
        assert_eq!(dark.tint, Rgb::new(0x2b, 0x2b, 0x2b));
        assert_eq!(dark.base, Rgb::new(0xe6, 0xe6, 0xe6));
        assert_eq!(dark.color(Category::Keyword), Rgb::new(255, 255, 255));
        assert_eq!(dark.color(Category::Comment), Rgb::new(0x9a, 0x9a, 0x9a));
        assert_eq!(dark.color(Category::String), Rgb::new(0xcf, 0xcf, 0xcf));
    }

    /// The `-mono` promise is *no hue anywhere*: a printer or a reader with
    /// colour vision deficiency sees the same hierarchy in every token.
    #[test]
    fn mono_palettes_have_no_hue() {
        for style in [HighlightStyle::LightMono, HighlightStyle::DarkMono] {
            let grey = palette(HighlightTheme::Github, style);
            for rgb in [grey.base, grey.tint, grey.frame, grey.gutter]
                .into_iter()
                .chain(grey.colors().iter().copied())
            {
                assert_eq!(rgb.r, rgb.g, "{style:?}: {rgb:?} is not a grey");
                assert_eq!(rgb.g, rgb.b, "{style:?}: {rgb:?} is not a grey");
            }
        }
    }

    /// Every (theme, style) pair gets its own cache slot: sharing one would
    /// answer a dark block with the light theme's colours.
    #[test]
    fn theme_cache_keys_cover_every_pair() {
        let mut slots: Vec<usize> = (0..2)
            .flat_map(|theme| (0..4).map(move |style| theme * 4 + style))
            .collect();
        assert_eq!(slots.len(), THEMES.len(), "one slot per pair");
        slots.sort_unstable();
        slots.dedup();
        assert_eq!(slots.len(), 8, "indices must stay distinct: {slots:?}");
        for (theme, style) in styles() {
            let built = theme_of(theme, style);
            assert_eq!(
                built.settings.foreground,
                Some(palette(theme, style).base.into())
            );
            assert!(
                std::ptr::eq(built, theme_of(theme, style)),
                "the cache must answer the same theme twice"
            );
        }
    }

    /// Only the mono palettes set a font flag, and only on keywords and
    /// comments — bold and italic are how they carry the hierarchy hue
    /// normally would.
    #[test]
    fn only_mono_palettes_set_font_flags() {
        let light = build_theme(palette(HighlightTheme::Github, HighlightStyle::Light));
        assert!(light
            .scopes
            .iter()
            .all(|item| item.style.font_style.is_none()));

        let mono = build_theme(palette(HighlightTheme::Github, HighlightStyle::LightMono));
        let flag = |selector: &str| {
            mono.scopes
                .iter()
                .find(|item| format!("{:?}", item.scope).contains(selector))
                .and_then(|item| item.style.font_style)
        };
        assert_eq!(flag("keyword"), Some(SyntectFont::BOLD));
        assert_eq!(flag("comment"), Some(SyntectFont::ITALIC));
    }

    /// `FontStyle::from` follows its documented precedence: bold wins when
    /// syntect reports both flags, underline alone carries no emphasis.
    #[test]
    fn font_style_from_prefers_bold_over_italic() {
        assert_eq!(
            FontStyle::from(SyntectFont::BOLD | SyntectFont::ITALIC),
            FontStyle::Bold
        );
        assert_eq!(FontStyle::from(SyntectFont::UNDERLINE), FontStyle::Normal);
    }

    /// WCAG AA (4.5:1) for every token against its own frame. The two `light`
    /// rows are today's palettes, excluded on purpose: the spec requires
    /// their output to stay byte-identical, so they cannot be re-tinted here
    /// (`one-light`'s comment grey measures 2.47:1 on paper).
    #[test]
    fn new_palettes_meet_wcag_aa() {
        for style in [
            HighlightStyle::Dark,
            HighlightStyle::LightMono,
            HighlightStyle::DarkMono,
        ] {
            for theme in [HighlightTheme::Github, HighlightTheme::OneLight] {
                let grey = palette(theme, style);
                for color in grey
                    .colors()
                    .iter()
                    .copied()
                    .chain([grey.base, grey.gutter])
                {
                    let ratio = contrast_ratio(color, grey.tint);
                    assert!(
                        ratio >= 4.5,
                        "{theme:?}/{style:?}: {color:?} on {:?} is {ratio:.2}:1",
                        grey.tint
                    );
                }
            }
        }
    }

    /// The frame hairline is a 0.4pt decoration, not text, so it is graded
    /// against the existing light frames rather than against AA: `github`
    /// measures 1.60:1 on its tint and `one-light` 1.35:1. A new style must
    /// not disappear into its own background — a hairline nobody can see is
    /// not a frame — but it does not have to shout either.
    #[test]
    fn new_frames_stay_visible_against_their_tint() {
        let legacy = palette(HighlightTheme::Github, HighlightStyle::Light);
        let floor = contrast_ratio(legacy.frame, legacy.tint);
        assert!(
            floor >= 1.3,
            "the baseline frame itself must show: {floor:.2}"
        );
        for style in [
            HighlightStyle::Dark,
            HighlightStyle::LightMono,
            HighlightStyle::DarkMono,
        ] {
            let grey = palette(HighlightTheme::Github, style);
            let ratio = contrast_ratio(grey.frame, grey.tint);
            assert!(
                ratio >= floor,
                "{style:?}: frame on tint is {ratio:.2}:1, below the light baseline {floor:.2}:1"
            );
        }
    }
}
