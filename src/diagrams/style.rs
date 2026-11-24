//! Named diagram style presets: `default`, `editorial`, `monochrome`,
//! `technical`, `diagram-design`.
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

/// The five supported style presets.
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
    /// Warm paper-tint nodes, muted brown lines, ink text, sans-serif
    /// face, generous whitespace.
    DiagramDesign,
}

/// The valid style names, in the order errors should list them.
pub const VALID_STYLE_NAMES: [&str; 5] = [
    "default",
    "editorial",
    "monochrome",
    "technical",
    "diagram-design",
];

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
            "diagram-design" => Ok(Self::DiagramDesign),
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
            // The palette of the `article` and `workshop` templates: ink
            // `#1D1510`, garnet accent `#87222C`, muted brown `#6B5A43`.
            // The one-accent rule holds: the accent is used once, for what
            // connects the shapes. `background` stays pure white rather than
            // taking the page's warm tint, because a diagram sits *on* the
            // printed page — tinting its canvas would paint a visible panel
            // on white paper. The tint becomes the node fill instead, so
            // boxes read as paper stock raised off the sheet.
            Self::Editorial => Some(Palette {
                background: "#FFFFFF",
                foreground: "#1D1510",
                neutral: "#6B5A43",
                neutral_light: "#F6F1E8",
                accent: Some("#87222C"),
                monospace: false,
                font_family: "'EB Garamond', Garamond, 'Times New Roman', serif",
            }),
            Self::Monochrome => Some(Palette {
                background: "#FFFFFF",
                foreground: "#1A1A1A",
                neutral: "#808080",
                neutral_light: "#E8E8E8",
                accent: None,
                monospace: false,
                font_family: "'Latin Modern Roman', 'Times New Roman', serif",
            }),
            Self::Technical => Some(Palette {
                background: "#FFFFFF",
                foreground: "#1A1A1A",
                neutral: "#555555",
                // Equal to background: shapes read as unfilled outlines.
                neutral_light: "#FFFFFF",
                accent: None,
                monospace: true,
                font_family: "'Courier New', 'DejaVu Sans Mono', ui-monospace, monospace",
            }),
            // Muted lines on tinted nodes, one ink for all text: the
            // intent is a warm hand-drawn register — brown strokes,
            // paper-tint fills, ink labels in a sans-serif face — with the
            // same generous spacing as `Editorial`. `accent` stays `None`
            // so edges and the D2/Graphviz accent role fall back to the
            // muted `neutral` through `accent_or_neutral`, giving muted
            // arrows instead of coloured ones. The garnet `#87222C` of
            // the report's figures is deliberately not a palette colour:
            // no engine hook exists for a per-node failure accent (stroke
            // width, corner radius and arrowheads are likewise beyond
            // what the engines express), so authors who want a failure
            // highlight set it per node in the diagram source.
            Self::DiagramDesign => Some(Palette {
                background: "#FFFFFF",
                foreground: "#1D1510",
                neutral: "#6B5A43",
                neutral_light: "#F1ECE4",
                accent: None,
                monospace: false,
                font_family: "'Helvetica Neue', Arial, 'DejaVu Sans', sans-serif",
            }),
        }
    }

    /// Hash of every palette field for this preset, so the render cache
    /// notices a palette change. `Default` carries no palette and hashes
    /// the constant `0`.
    pub(crate) fn fingerprint(self) -> u64 {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        match self.palette() {
            None => {
                0u8.hash(&mut hasher);
            }
            Some(p) => {
                p.background.hash(&mut hasher);
                p.foreground.hash(&mut hasher);
                p.neutral.hash(&mut hasher);
                p.neutral_light.hash(&mut hasher);
                p.accent.hash(&mut hasher);
                p.monospace.hash(&mut hasher);
                p.font_family.hash(&mut hasher);
            }
        }
        hasher.finish()
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
    /// Whether labels should render in a monospaced face. Only the D2
    /// branch reads this (as its `theme-id`); Mermaid and Graphviz fonts
    /// come from `font_family` below.
    monospace: bool,
    /// The Mermaid font stack for this preset. D2 fixes its own faces and
    /// `layout-rs` never reads `fontname`, so for those two engines only
    /// colours change; see `docs/diagrams.md`.
    font_family: &'static str,
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
    let font_family = p.font_family.to_string();
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
    if matches!(style, DiagramStyle::Editorial | DiagramStyle::DiagramDesign) {
        // "Generous whitespace" is one of the editorial rules, shared by
        // the hand-drawn register of `DiagramDesign`.
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
    fn parse_all_five_names() {
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
        assert_eq!(
            DiagramStyle::parse("diagram-design").unwrap(),
            DiagramStyle::DiagramDesign
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

    #[test]
    fn editorial_mermaid_uses_template_palette() {
        let opts = mermaid_options(DiagramStyle::Editorial);
        assert_eq!(opts.theme.line_color, "#87222C");
        assert_eq!(opts.theme.text_color, "#1D1510");
        assert!(
            opts.theme.font_family.starts_with("'EB Garamond'"),
            "font family: {}",
            opts.theme.font_family
        );
    }

    #[test]
    fn editorial_d2_prefix_has_accent_and_ink() {
        let prefix = d2_prefix(DiagramStyle::Editorial);
        assert!(prefix.contains("#87222C"), "prefix: {prefix}");
        assert!(prefix.contains("#1D1510"), "prefix: {prefix}");
    }

    #[test]
    fn editorial_graphviz_injection_has_accent() {
        let injected = graphviz_inject("digraph G { A -> B }", DiagramStyle::Editorial);
        assert!(injected.contains("#87222C"), "injected: {injected}");
    }

    #[test]
    fn monochrome_mermaid_uses_latin_modern_stack() {
        let opts = mermaid_options(DiagramStyle::Monochrome);
        assert!(
            opts.theme.font_family.starts_with("'Latin Modern Roman'"),
            "font family: {}",
            opts.theme.font_family
        );
    }

    #[test]
    fn fingerprints_differ_across_all_five_presets() {
        let prints = [
            DiagramStyle::Default.fingerprint(),
            DiagramStyle::Editorial.fingerprint(),
            DiagramStyle::Monochrome.fingerprint(),
            DiagramStyle::Technical.fingerprint(),
            DiagramStyle::DiagramDesign.fingerprint(),
        ];
        for i in 0..prints.len() {
            for j in (i + 1)..prints.len() {
                assert_ne!(prints[i], prints[j], "presets {i} and {j} collide");
            }
        }
    }

    #[test]
    fn fingerprint_stable_across_calls() {
        for style in [
            DiagramStyle::Default,
            DiagramStyle::Editorial,
            DiagramStyle::Monochrome,
            DiagramStyle::Technical,
            DiagramStyle::DiagramDesign,
        ] {
            assert_eq!(style.fingerprint(), style.fingerprint());
        }
    }

    #[test]
    fn technical_outputs_pinned() {
        let opts = mermaid_options(DiagramStyle::Technical);
        assert_eq!(
            opts.theme.font_family,
            "'Courier New', 'DejaVu Sans Mono', ui-monospace, monospace"
        );
        let prefix = d2_prefix(DiagramStyle::Technical);
        assert!(prefix.contains("theme-id: 301"), "prefix: {prefix}");
        let injected = graphviz_inject("digraph G { A -> B }", DiagramStyle::Technical);
        assert!(
            injected.contains(
                "node [style=filled, fillcolor=\"#FFFFFF\", color=\"#555555\", fontsize=12];"
            ),
            "injected: {injected}"
        );
        assert!(
            injected.contains("edge [color=\"#555555\"];"),
            "injected: {injected}"
        );
    }

    #[test]
    fn existing_palettes_pinned_field_by_field() {
        // Every palette field of the four pre-existing presets, against
        // literals: nothing about them may drift silently.
        assert!(DiagramStyle::Default.palette().is_none());
        let p = DiagramStyle::Editorial.palette().unwrap();
        assert_eq!(p.background, "#FFFFFF");
        assert_eq!(p.foreground, "#1D1510");
        assert_eq!(p.neutral, "#6B5A43");
        assert_eq!(p.neutral_light, "#F6F1E8");
        assert_eq!(p.accent, Some("#87222C"));
        assert!(!p.monospace);
        assert_eq!(
            p.font_family,
            "'EB Garamond', Garamond, 'Times New Roman', serif"
        );
        let p = DiagramStyle::Monochrome.palette().unwrap();
        assert_eq!(p.background, "#FFFFFF");
        assert_eq!(p.foreground, "#1A1A1A");
        assert_eq!(p.neutral, "#808080");
        assert_eq!(p.neutral_light, "#E8E8E8");
        assert_eq!(p.accent, None);
        assert!(!p.monospace);
        assert_eq!(
            p.font_family,
            "'Latin Modern Roman', 'Times New Roman', serif"
        );
        let p = DiagramStyle::Technical.palette().unwrap();
        assert_eq!(p.background, "#FFFFFF");
        assert_eq!(p.foreground, "#1A1A1A");
        assert_eq!(p.neutral, "#555555");
        assert_eq!(p.neutral_light, "#FFFFFF");
        assert_eq!(p.accent, None);
        assert!(p.monospace);
        assert_eq!(
            p.font_family,
            "'Courier New', 'DejaVu Sans Mono', ui-monospace, monospace"
        );
    }

    #[test]
    fn diagram_design_palette_pinned_field_by_field() {
        let p = DiagramStyle::DiagramDesign.palette().unwrap();
        assert_eq!(p.background, "#FFFFFF");
        assert_eq!(p.foreground, "#1D1510");
        assert_eq!(p.neutral, "#6B5A43");
        assert_eq!(p.neutral_light, "#F1ECE4");
        assert_eq!(p.accent, None);
        assert!(!p.monospace);
        assert_eq!(
            p.font_family,
            "'Helvetica Neue', Arial, 'DejaVu Sans', sans-serif"
        );
    }

    #[test]
    fn default_outputs_pinned() {
        let opts = mermaid_options(DiagramStyle::Default);
        let base = mermaid_rs_renderer::RenderOptions::default();
        assert_eq!(opts.theme.font_family, base.theme.font_family);
        assert_eq!(opts.theme.line_color, base.theme.line_color);
        assert_eq!(opts.theme.text_color, base.theme.text_color);
        assert_eq!(opts.layout.node_spacing, base.layout.node_spacing);
        assert_eq!(d2_prefix(DiagramStyle::Default), "");
        let src = "digraph G { A -> B }";
        assert_eq!(graphviz_inject(src, DiagramStyle::Default), src);
    }
}
