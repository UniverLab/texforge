//! Named diagram style presets: `default`, `editorial`, `monochrome`, `technical`.
//!
//! Presets are declared here as data — one [`Palette`] per preset — and
//! mapped onto each renderer's own theming vocabulary by the functions
//! below. `default` carries no palette: renderers keep producing exactly
//! what they do today, so omitting `style=` never changes an existing
//! document's appearance.
//!
//! Fidelity differs by renderer. Mermaid exposes a rich `Theme`; D2 exposes
//! a named colour palette via `theme-overrides`; Graphviz (`layout-rs`) has
//! no theme concept at all, only per-node/per-edge attributes, so a
//! document-wide background is not expressible there — that gap is
//! documented in `docs/diagrams.md` rather than hidden.

use anyhow::{bail, Result};

/// The four supported style presets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagramStyle {
    /// Each renderer's own untouched default. Never changes.
    Default,
    /// Restrained palette, one accent colour, no shadows, thin strokes,
    /// generous whitespace.
    Editorial,
    /// Greyscale only — for black-and-white printing.
    Monochrome,
    /// Drafting register: uniform stroke weight, no fills, monospaced
    /// labels where the renderer allows it.
    Technical,
}

/// The valid style names, in the order errors should list them.
pub const VALID_STYLE_NAMES: [&str; 4] = ["default", "editorial", "monochrome", "technical"];

impl DiagramStyle {
    /// Parse a style name from a `style=` attribute or `project.toml`.
    ///
    /// An unknown name is an error naming the offending value and the valid
    /// alternatives — silently falling back to `default` would hide a typo
    /// until the document is printed.
    pub fn parse(name: &str) -> Result<Self> {
        match name {
            "default" => Ok(Self::Default),
            "editorial" => Ok(Self::Editorial),
            "monochrome" => Ok(Self::Monochrome),
            "technical" => Ok(Self::Technical),
            other => bail!(
                "Unknown diagram style '{other}' — valid styles are: {}",
                VALID_STYLE_NAMES.join(", ")
            ),
        }
    }

    /// The palette this preset expresses, or `None` for `Default`.
    fn palette(self) -> Option<Palette> {
        match self {
            Self::Default => None,
            // texforge's own editorial palette, the one its page on
            // univerlab.org is typeset in (`data-surface='paper'`). The accent
            // is the proofreader's vermilion change-bar, and the surface
            // comment there states the rule this preset follows: it is used
            // once. `background` stays pure white rather than taking the
            // page's warm tint, because a diagram sits *on* the printed page —
            // tinting its canvas would paint a visible grey panel on white
            // paper. The tint becomes the node fill instead, so boxes read as
            // paper stock raised off the sheet.
            Self::Editorial => Some(Palette {
                background: "#FFFFFF",
                foreground: "#1B1B1A",
                neutral: "#54514C",
                neutral_light: "#F5F5F3",
                accent: Some("#B2362C"),
                monospace: false,
            }),
            Self::Monochrome => Some(Palette {
                background: "#FFFFFF",
                foreground: "#1A1A1A",
                neutral: "#808080",
                neutral_light: "#E8E8E8",
                accent: None,
                monospace: false,
            }),
            Self::Technical => Some(Palette {
                background: "#FFFFFF",
                foreground: "#1A1A1A",
                neutral: "#555555",
                // Equal to background: shapes read as unfilled outlines.
                neutral_light: "#FFFFFF",
                accent: None,
                monospace: true,
            }),
        }
    }
}

/// A restrained palette one style preset expresses, in vocabulary generic
/// enough to map onto Mermaid's `Theme`, D2's `theme-overrides`, and
/// Graphviz's per-node/per-edge `StyleAttr`.
struct Palette {
    background: &'static str,
    foreground: &'static str,
    neutral: &'static str,
    neutral_light: &'static str,
    /// The one accent colour a preset uses; `None` for the greyscale-only
    /// presets (`monochrome`, `technical`), so every field below falls back
    /// to a neutral and the output stays saturation-free.
    accent: Option<&'static str>,
    /// Whether labels should render in a monospaced face.
    monospace: bool,
}

impl Palette {
    fn accent_or_neutral(&self) -> &'static str {
        self.accent.unwrap_or(self.neutral)
    }
}

// ---------------------------------------------------------------------------
// Mermaid — the richest surface: a full `Theme` plus layout spacing.
// ---------------------------------------------------------------------------

/// Build Mermaid `RenderOptions` for `style`. `Default` returns exactly
/// `RenderOptions::default()`, so an unstyled diagram renders byte-identical
/// to before this feature existed.
pub fn mermaid_options(style: DiagramStyle) -> mermaid_rs_renderer::RenderOptions {
    let Some(p) = style.palette() else {
        return mermaid_rs_renderer::RenderOptions::default();
    };

    let accent = p.accent_or_neutral();
    let font_family = if p.monospace {
        "'Courier New', 'DejaVu Sans Mono', ui-monospace, monospace".to_string()
    } else {
        "'Helvetica Neue', Helvetica, Arial, sans-serif".to_string()
    };
    let pie_colors: [String; 12] = {
        let cycle = [
            p.background,
            accent,
            p.neutral,
            p.neutral_light,
            p.foreground,
        ];
        std::array::from_fn(|i| cycle[i % cycle.len()].to_string())
    };

    let theme = mermaid_rs_renderer::Theme {
        font_family,
        font_size: 14.0,
        primary_color: p.neutral_light.to_string(),
        primary_text_color: p.foreground.to_string(),
        // Shapes are drawn in ink; the accent is reserved for what connects
        // them. Spending it on every node border as well turns "one accent"
        // into "coloured everywhere", which is what the restrained presets
        // exist to avoid — and a saturated accent on every outline reads as a
        // warning rather than as a finish.
        primary_border_color: p.neutral.to_string(),
        line_color: accent.to_string(),
        secondary_color: p.neutral_light.to_string(),
        tertiary_color: p.background.to_string(),
        edge_label_background: p.background.to_string(),
        cluster_background: p.neutral_light.to_string(),
        cluster_border: p.neutral.to_string(),
        background: p.background.to_string(),
        sequence_actor_fill: p.neutral_light.to_string(),
        sequence_actor_border: p.neutral.to_string(),
        sequence_actor_line: p.neutral.to_string(),
        sequence_note_fill: p.neutral_light.to_string(),
        sequence_note_border: p.neutral.to_string(),
        sequence_activation_fill: p.neutral_light.to_string(),
        sequence_activation_border: p.neutral.to_string(),
        text_color: p.foreground.to_string(),
        git_colors: [
            p.foreground,
            accent,
            p.neutral,
            p.neutral_light,
            p.background,
            p.foreground,
            accent,
            p.neutral,
        ]
        .map(|s| s.to_string()),
        git_inv_colors: [p.background; 8].map(|s| s.to_string()),
        git_branch_label_colors: [p.foreground; 8].map(|s| s.to_string()),
        git_commit_label_color: p.foreground.to_string(),
        git_commit_label_background: p.background.to_string(),
        git_tag_label_color: p.foreground.to_string(),
        git_tag_label_background: p.neutral_light.to_string(),
        git_tag_label_border: p.neutral.to_string(),
        pie_colors,
        pie_title_text_size: 20.0,
        pie_title_text_color: p.foreground.to_string(),
        pie_section_text_size: 14.0,
        pie_section_text_color: p.foreground.to_string(),
        pie_legend_text_size: 14.0,
        pie_legend_text_color: p.foreground.to_string(),
        pie_stroke_color: p.foreground.to_string(),
        pie_stroke_width: 1.0,
        pie_outer_stroke_width: 1.0,
        pie_outer_stroke_color: p.neutral.to_string(),
        pie_opacity: 1.0,
    };

    let mut layout = mermaid_rs_renderer::LayoutConfig::default();
    if matches!(style, DiagramStyle::Editorial) {
        // "Generous whitespace" is one of the editorial rules.
        layout.node_spacing *= 1.4;
        layout.rank_spacing *= 1.4;
    }

    mermaid_rs_renderer::RenderOptions { theme, layout }
}

// ---------------------------------------------------------------------------
// D2 — native theming via a `d2-config` block prepended to the source.
// ---------------------------------------------------------------------------

/// A `vars: { d2-config: ... }` block to prepend to D2 source, or an empty
/// string for `Default` (no override — d2-little's own default theme).
///
/// `theme-overrides` in D2's own config vocabulary only takes effect when
/// parsed from source text; the compile-options struct has no field for it.
/// Prepending valid D2 syntax is the supported route, not a workaround.
///
/// `technical` bases on theme 301 (Terminal Grayscale), whose `mono`
/// special rule switches labels to a monospaced face — the one lever D2
/// exposes for that requirement. Its other special rules (double borders,
/// container dots) only affect nested/grouped diagrams and are a documented
/// side effect, not a colour or fidelity gap.
pub fn d2_prefix(style: DiagramStyle) -> String {
    let Some(p) = style.palette() else {
        return String::new();
    };
    let accent = p.accent_or_neutral();
    let theme_id = if p.monospace { 301 } else { 0 };
    format!(
        "vars: {{\n  d2-config: {{\n    theme-id: {theme_id}\n    theme-overrides: {{\n      N1: \"{fg}\"\n      N2: \"{neu}\"\n      N3: \"{neu}\"\n      N4: \"{neul}\"\n      N5: \"{neul}\"\n      N6: \"{neul}\"\n      N7: \"{bg}\"\n      B1: \"{fg}\"\n      B2: \"{acc}\"\n      B3: \"{neu}\"\n      B4: \"{neul}\"\n      B5: \"{neul}\"\n      B6: \"{bg}\"\n      AA2: \"{acc}\"\n      AA4: \"{neul}\"\n      AA5: \"{bg}\"\n      AB4: \"{neul}\"\n      AB5: \"{bg}\"\n    }}\n  }}\n}}\n",
        fg = p.foreground,
        neu = p.neutral,
        neul = p.neutral_light,
        bg = p.background,
        acc = accent,
    )
}

// ---------------------------------------------------------------------------
// Graphviz — no theme concept; per-node/per-edge default attributes only.
// ---------------------------------------------------------------------------

/// `node [...]; edge [...];` default-attribute statements to inject right
/// after the graph's opening `{`, or an empty string for `Default`.
///
/// `layout-rs` has no document-wide theme or background: `StyleAttr` is
/// applied per node and per edge as the graph is built, and the parser
/// never reads a graph-level `bgcolor`. A page background is therefore not
/// expressible here — an accepted, documented gap, not a build failure.
/// Font family (`fontname`) is likewise never read by the DOT builder, so
/// `technical`'s monospaced-label rule cannot be expressed for Graphviz
/// either; only the colour and fill rules carry over.
fn graphviz_attr_statements(style: DiagramStyle) -> String {
    let Some(p) = style.palette() else {
        return String::new();
    };
    let accent = p.accent_or_neutral();
    format!(
        "node [style=filled, fillcolor=\"{}\", color=\"{}\", fontsize=12];\nedge [color=\"{}\"];\n",
        p.neutral_light, accent, accent
    )
}

/// Inject `style`'s default-attribute statements into DOT source, right
/// after the first `{` (the body of `digraph G { ... }` / `graph G { ... }`).
pub fn graphviz_inject(src: &str, style: DiagramStyle) -> String {
    let stmt = graphviz_attr_statements(style);
    if stmt.is_empty() {
        return src.to_string();
    }
    match src.find('{') {
        Some(idx) => {
            let mut out = String::with_capacity(src.len() + stmt.len() + 1);
            out.push_str(&src[..=idx]);
            out.push('\n');
            out.push_str(&stmt);
            out.push_str(&src[idx + 1..]);
            out
        }
        None => src.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_all_four_names() {
        assert_eq!(
            DiagramStyle::parse("default").unwrap(),
            DiagramStyle::Default
        );
        assert_eq!(
            DiagramStyle::parse("editorial").unwrap(),
            DiagramStyle::Editorial
        );
        assert_eq!(
            DiagramStyle::parse("monochrome").unwrap(),
            DiagramStyle::Monochrome
        );
        assert_eq!(
            DiagramStyle::parse("technical").unwrap(),
            DiagramStyle::Technical
        );
    }

    #[test]
    fn parse_unknown_name_lists_valid_names() {
        let err = DiagramStyle::parse("editoral").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("editoral"), "message: {msg}");
        for name in VALID_STYLE_NAMES {
            assert!(msg.contains(name), "message missing '{name}': {msg}");
        }
    }

    #[test]
    fn parse_rejects_old_mono_name() {
        // `mono` was renamed to `monochrome`; the abbreviation must not be
        // a silent alias, and the error should point at the new name.
        let err = DiagramStyle::parse("mono").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("monochrome"), "message: {msg}");
    }

    #[test]
    fn parse_rejects_blueprint_name() {
        // `technical` was named on purpose to avoid promising a blue-on-white
        // colour scheme; `blueprint` must not be a silent alias for it.
        assert!(DiagramStyle::parse("blueprint").is_err());
    }

    #[test]
    fn mermaid_default_style_is_untouched_default_options() {
        let opts = mermaid_options(DiagramStyle::Default);
        let base = mermaid_rs_renderer::RenderOptions::default();
        assert_eq!(opts.theme.background, base.theme.background);
        assert_eq!(opts.theme.primary_color, base.theme.primary_color);
        assert_eq!(opts.layout.node_spacing, base.layout.node_spacing);
    }

    #[test]
    fn mermaid_monochrome_style_has_no_accent() {
        let opts = mermaid_options(DiagramStyle::Monochrome);
        // No accent color: border and line colors fall back to the neutral.
        assert_eq!(opts.theme.primary_border_color, opts.theme.line_color);
    }

    #[test]
    fn d2_default_style_has_no_prefix() {
        assert_eq!(d2_prefix(DiagramStyle::Default), "");
    }

    #[test]
    fn d2_editorial_style_prefix_contains_theme_overrides() {
        let prefix = d2_prefix(DiagramStyle::Editorial);
        assert!(prefix.contains("theme-overrides"));
        // Derived from the palette rather than hardcoded: this asserts the
        // accent reaches the prefix, which is what can break. Retuning the
        // palette is not a regression and should not fail here.
        let accent = DiagramStyle::Editorial.palette().unwrap().accent.unwrap();
        assert!(prefix.contains(accent));
    }

    #[test]
    fn graphviz_default_style_leaves_source_untouched() {
        let src = "digraph G { A -> B }";
        assert_eq!(graphviz_inject(src, DiagramStyle::Default), src);
    }

    #[test]
    fn graphviz_styled_source_injects_after_opening_brace() {
        let src = "digraph G { A -> B }";
        let injected = graphviz_inject(src, DiagramStyle::Editorial);
        assert!(injected.contains("node [style=filled"));
        assert!(injected.contains("edge [color="));
        assert!(injected.trim_end().ends_with("}"));
    }
}
