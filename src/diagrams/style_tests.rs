//! Tests for the `diagram-design` preset and pins for the four existing
//! presets, kept here so `style.rs` stays under the file-size budget.
//!
//! Only the public surface of [`super::style`] is used: parsing, the style
//! name list, fingerprints, and the three renderer mappings. Field-by-field
//! palette assertions live in `style.rs`'s own test module, which alone can
//! see the private `Palette`.

use super::style::{self, DiagramStyle};

const DIAGRAM_DESIGN_FONT: &str = "'Helvetica Neue', Arial, 'DejaVu Sans', sans-serif";

#[test]
fn diagram_design_parses() {
    assert_eq!(
        DiagramStyle::parse("diagram-design").unwrap(),
        DiagramStyle::DiagramDesign
    );
}

#[test]
fn unknown_name_error_lists_all_five_names_in_order() {
    let err = DiagramStyle::parse("diagram-desgin").unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("diagram-desgin"), "message: {msg}");
    let mut prev = 0;
    for name in [
        "default",
        "editorial",
        "monochrome",
        "technical",
        "diagram-design",
    ] {
        let pos = msg
            .find(name)
            .unwrap_or_else(|| panic!("message missing '{name}': {msg}"));
        assert!(pos >= prev, "names out of order in: {msg}");
        prev = pos;
    }
    assert_eq!(style::VALID_STYLE_NAMES.len(), 5);
}

#[test]
fn diagram_design_fingerprint_differs_from_every_other_style() {
    let own = DiagramStyle::DiagramDesign.fingerprint();
    for other in [
        DiagramStyle::Default,
        DiagramStyle::Editorial,
        DiagramStyle::Monochrome,
        DiagramStyle::Technical,
    ] {
        assert_ne!(
            own,
            other.fingerprint(),
            "fingerprint collides with {other:?}"
        );
    }
}

#[test]
fn diagram_design_d2_prefix_has_tint_and_muted_lines() {
    let prefix = style::d2_prefix(DiagramStyle::DiagramDesign);
    assert!(prefix.contains("#F1ECE4"), "prefix: {prefix}");
    assert!(prefix.contains("#6B5A43"), "prefix: {prefix}");
    assert!(prefix.contains("theme-id: 0"), "prefix: {prefix}");
}

#[test]
fn diagram_design_graphviz_injection_has_tint_and_muted_lines() {
    let injected = style::graphviz_inject("digraph G { A }", DiagramStyle::DiagramDesign);
    assert!(
        injected.contains("fillcolor=\"#F1ECE4\""),
        "injected: {injected}"
    );
    assert!(
        injected.contains("color=\"#6B5A43\""),
        "injected: {injected}"
    );
}

#[test]
fn diagram_design_mermaid_options_use_palette_literals() {
    let opts = style::mermaid_options(DiagramStyle::DiagramDesign);
    assert_eq!(opts.theme.background, "#FFFFFF");
    assert_eq!(opts.theme.text_color, "#1D1510");
    assert_eq!(opts.theme.primary_color, "#F1ECE4");
    assert_eq!(opts.theme.primary_border_color, "#6B5A43");
    // No accent: edges fall back to the muted neutral, not a colour.
    assert_eq!(opts.theme.line_color, "#6B5A43");
    assert_eq!(opts.theme.font_family, DIAGRAM_DESIGN_FONT);
}

#[test]
fn diagram_design_mermaid_layout_shares_editorial_spacing() {
    let styled = style::mermaid_options(DiagramStyle::DiagramDesign);
    let plain = mermaid_rs_renderer::RenderOptions::default();
    assert_eq!(styled.layout.node_spacing, plain.layout.node_spacing * 1.4);
    assert_eq!(styled.layout.rank_spacing, plain.layout.rank_spacing * 1.4);
}

// ---- Requirement 4: the four existing presets are pinned -----------------

#[test]
fn editorial_preset_pinned() {
    let opts = style::mermaid_options(DiagramStyle::Editorial);
    assert_eq!(opts.theme.background, "#FFFFFF");
    assert_eq!(opts.theme.text_color, "#1D1510");
    assert_eq!(opts.theme.primary_border_color, "#6B5A43");
    assert_eq!(opts.theme.primary_color, "#F6F1E8");
    assert_eq!(opts.theme.line_color, "#87222C");
    assert_eq!(
        opts.theme.font_family,
        "'EB Garamond', Garamond, 'Times New Roman', serif"
    );
    let prefix = style::d2_prefix(DiagramStyle::Editorial);
    for colour in ["#FFFFFF", "#1D1510", "#6B5A43", "#F6F1E8", "#87222C"] {
        assert!(
            prefix.contains(colour),
            "d2 prefix missing {colour}: {prefix}"
        );
    }
    let injected = style::graphviz_inject("digraph G { A -> B }", DiagramStyle::Editorial);
    assert!(
        injected.contains("fillcolor=\"#F6F1E8\""),
        "injected: {injected}"
    );
    // The accent role carries the garnet for both nodes and edges.
    assert!(
        injected.contains(
            "node [style=filled, fillcolor=\"#F6F1E8\", color=\"#87222C\", fontsize=12];"
        ),
        "injected: {injected}"
    );
    assert!(
        injected.contains("edge [color=\"#87222C\"];"),
        "injected: {injected}"
    );
}

#[test]
fn monochrome_preset_pinned() {
    let opts = style::mermaid_options(DiagramStyle::Monochrome);
    assert_eq!(opts.theme.background, "#FFFFFF");
    assert_eq!(opts.theme.text_color, "#1A1A1A");
    assert_eq!(opts.theme.primary_border_color, "#808080");
    assert_eq!(opts.theme.primary_color, "#E8E8E8");
    // No accent: border and line colours fall back to the neutral.
    assert_eq!(opts.theme.line_color, "#808080");
    assert_eq!(
        opts.theme.font_family,
        "'Latin Modern Roman', 'Times New Roman', serif"
    );
    let injected = style::graphviz_inject("digraph G { A -> B }", DiagramStyle::Monochrome);
    assert!(
        injected.contains("fillcolor=\"#E8E8E8\""),
        "injected: {injected}"
    );
    assert!(
        injected.contains("color=\"#808080\""),
        "injected: {injected}"
    );
}

#[test]
fn technical_preset_pinned() {
    let opts = style::mermaid_options(DiagramStyle::Technical);
    assert_eq!(opts.theme.background, "#FFFFFF");
    assert_eq!(opts.theme.text_color, "#1A1A1A");
    assert_eq!(opts.theme.primary_border_color, "#555555");
    assert_eq!(opts.theme.primary_color, "#FFFFFF");
    assert_eq!(opts.theme.line_color, "#555555");
    assert_eq!(
        opts.theme.font_family,
        "'Courier New', 'DejaVu Sans Mono', ui-monospace, monospace"
    );
    let prefix = style::d2_prefix(DiagramStyle::Technical);
    assert!(prefix.contains("theme-id: 301"), "prefix: {prefix}");
    let injected = style::graphviz_inject("digraph G { A -> B }", DiagramStyle::Technical);
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
fn default_mermaid_options_equal_untouched_defaults() {
    let opts = style::mermaid_options(DiagramStyle::Default);
    let base = mermaid_rs_renderer::RenderOptions::default();
    assert_eq!(format!("{:?}", opts.theme), format!("{:?}", base.theme));
    assert_eq!(format!("{:?}", opts.layout), format!("{:?}", base.layout));
}
