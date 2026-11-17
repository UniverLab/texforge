//! `label=` options on diagram blocks.
//!
//! The build turns `label={…}` on a `mermaid`, `d2` or `graphviz` block into
//! a real `\label`, with or without a `caption`. This collects those labels
//! so the `\ref` check accepts them.

/// Diagram environments whose `label=` option defines a `\ref` target.
const DIAGRAM_ENVIRONMENTS: &[&str] = &["mermaid", "d2", "graphviz"];

/// Every diagram `label=` option defined in `source`.
pub(super) fn labels(source: &str) -> Vec<String> {
    let mut found = Vec::new();
    for spanned in crate::texparse::tokenize_with_spans(source).tokens {
        let crate::texparse::Token::Environment { name } = spanned.token else {
            continue;
        };
        if !DIAGRAM_ENVIRONMENTS.contains(&name.as_str()) {
            continue;
        }
        let tag = format!("\\begin{{{name}}}");
        // `\end{env}` yields an `Environment` token too; only a `\begin` tag
        // carries an option list. The text handed to `parse_opts` starts right
        // after the tag, exactly what the build passes, so the leading newline
        // `parse_opts` itself strips behaves the same in both.
        if !source[spanned.start..spanned.end].starts_with(&tag) {
            continue;
        }
        if let Some(label) = label_in_options(&source[spanned.start + tag.len()..], &name) {
            found.push(label);
        }
    }
    found
}

/// The `label=` value of one `\begin{env}[…]` remainder, or `None` when the
/// option list does not parse or carries no usable label.
fn label_in_options(rest: &str, env: &str) -> Option<String> {
    let (opts, _) = crate::diagrams::parse_opts(rest, env).ok()?;
    let label = opts.get("label")?;
    (!label.is_empty()).then(|| label.clone())
}

#[cfg(test)]
mod tests {
    use super::labels;

    #[test]
    fn braced_label_is_collected() {
        let src = "\\begin{mermaid}[label={fig:flow}]\nA --> B\n\\end{mermaid}\n";
        assert_eq!(labels(src), vec!["fig:flow".to_string()]);
    }

    #[test]
    fn plain_label_is_collected() {
        let src = "\\begin{mermaid}[label=fig:flow]\nA --> B\n\\end{mermaid}\n";
        assert_eq!(labels(src), vec!["fig:flow".to_string()]);
    }

    #[test]
    fn options_split_over_several_lines_are_collected() {
        let src = "\\begin{mermaid}[\nlabel={fig:flow}\n]\nA --> B\n\\end{mermaid}\n";
        assert_eq!(labels(src), vec!["fig:flow".to_string()]);
    }

    /// The build strips a leading newline before its `[`, so a `label=` on the
    /// line after `\begin` counts there; collecting it must agree.
    #[test]
    fn a_newline_before_the_option_list_is_collected() {
        let src = "\\begin{mermaid}\n[label={fig:flow}]\nA --> B\n\\end{mermaid}\n";
        assert_eq!(labels(src), vec!["fig:flow".to_string()]);
    }

    /// The `\end{env}` tag is an `Environment` token too and must not be read
    /// as a second block: the body's text has no options.
    #[test]
    fn the_end_tag_adds_nothing() {
        let src = "\\begin{mermaid}\nA --> B\n\\end{mermaid}\n";
        assert!(labels(src).is_empty());
    }

    #[test]
    fn comma_inside_braces_stays_one_label() {
        let src = "\\begin{mermaid}[caption={A, B}, label={fig:flow}]\nA\n\\end{mermaid}\n";
        assert_eq!(labels(src), vec!["fig:flow".to_string()]);
    }

    #[test]
    fn unterminated_brace_adds_nothing() {
        let src = "\\begin{mermaid}[label={x]\nA\n\\end{mermaid}\n";
        assert!(labels(src).is_empty());
    }

    #[test]
    fn empty_label_adds_nothing() {
        let src = "\\begin{mermaid}[label={}]\nA\n\\end{mermaid}\n";
        assert!(labels(src).is_empty());
    }

    #[test]
    fn commented_begin_adds_nothing() {
        let src = "% \\begin{mermaid}[label={fig:flow}]\n";
        assert!(labels(src).is_empty());
    }
}
