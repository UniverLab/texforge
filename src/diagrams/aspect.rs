//! Include-graphics sizing that preserves natural aspect ratio.

use std::collections::HashMap;

/// Build the `\includegraphics` option string for a diagram figure.
///
/// When only `width` is given, `keepaspectratio` is added so the rendered
/// figure keeps the artefact's natural proportions.
pub(crate) fn includegraphics_options(opts: &HashMap<String, String>) -> String {
    let width = opts.get("width").map(String::as_str);
    let height = opts.get("height").map(String::as_str);
    let scale = opts.get("scale").map(String::as_str);
    let keepaspectratio = opts.contains_key("keepaspectratio");

    let mut include_opts = Vec::new();
    if let Some(s) = scale {
        include_opts.push(format!("scale={s}"));
    } else {
        if let Some(w) = width {
            include_opts.push(format!("width={w}"));
        }
        if let Some(h) = height {
            include_opts.push(format!("height={h}"));
        }
        if width.is_some() && height.is_none() {
            include_opts.push("keepaspectratio".to_string());
        }
    }
    if keepaspectratio && !include_opts.iter().any(|o| o == "keepaspectratio") {
        include_opts.push("keepaspectratio".to_string());
    }
    if include_opts.is_empty() {
        "width=\\linewidth".to_string()
    } else {
        include_opts.join(",")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect()
    }

    #[test]
    fn width_only_preserves_aspect_ratio() {
        let s = includegraphics_options(&opts(&[("width", "0.8\\linewidth")]));
        assert!(s.contains("width=0.8\\linewidth"), "{s}");
        assert!(!s.contains("height="), "{s}");
        assert!(s.contains("keepaspectratio"), "{s}");
    }

    #[test]
    fn height_only_does_not_add_width() {
        let s = includegraphics_options(&opts(&[("height", "6cm")]));
        assert!(!s.contains("width="), "{s}");
        assert!(!s.contains("keepaspectratio"), "{s}");
    }

    #[test]
    fn width_and_height_do_not_force_keepaspectratio() {
        let s = includegraphics_options(&opts(&[("width", "0.8\\linewidth"), ("height", "6cm")]));
        assert!(s.contains("height=6cm"), "{s}");
        assert!(!s.contains("keepaspectratio"), "{s}");
    }

    #[test]
    fn scale_ignores_width_and_height() {
        let s = includegraphics_options(&opts(&[("scale", "0.5"), ("width", "0.8\\linewidth")]));
        assert_eq!(s, "scale=0.5");
    }

    #[test]
    fn explicit_keepaspectratio_is_kept_once() {
        let s = includegraphics_options(&opts(&[
            ("width", "0.8\\linewidth"),
            ("keepaspectratio", ""),
        ]));
        assert_eq!(s.matches("keepaspectratio").count(), 1, "{s}");
    }

    #[test]
    fn empty_opts_fall_back_to_linewidth() {
        let s = includegraphics_options(&HashMap::new());
        assert_eq!(s, "width=\\linewidth");
    }
}
