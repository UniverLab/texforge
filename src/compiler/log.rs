//! Engine-log helpers: overfull-box reporting and log persistence.
//!
//! After every compile — successful or not — the engine's complete log text
//! is written to disk (see [`write_log`]), and overfull horizontal boxes
//! wider than 1pt are reported by source file and line (see
//! [`format_overfull`]).

use std::path::Path;

use anyhow::{Context, Result};

use super::CompileWarning;

/// Parse the width out of `Overfull \hbox (Npt too wide)`; `None` when the
/// message carries no width (other warning kinds, malformed text).
pub(crate) fn parse_width(message: &str) -> Option<f32> {
    const MARKER: &str = "Overfull \\hbox (";
    const SUFFIX: &str = "pt too wide)";
    let start = message.find(MARKER)? + MARKER.len();
    let rest = &message[start..];
    let end = rest.find(SUFFIX)?;
    rest[..end].trim().parse::<f32>().ok()
}

/// True when the log context around a warning shows the box arose while
/// `\output` was active — there is no source line then, so the box is
/// reported under the `output routine` label instead of a `file:line`.
pub(crate) fn is_output_routine(message: &str) -> bool {
    message.contains("has occurred while \\output is active")
}

/// Render a point width without float noise: whole widths print bare
/// (`12pt`), fractional ones print as-is (`12.5pt`).
fn fmt_pt(width: f32) -> String {
    if width == width.trunc() {
        format!("{}", width as i64)
    } else {
        format!("{width}")
    }
}

/// Report overfull horizontal boxes wider than 1pt: one summary line plus one
/// line per box, widest first. Boxes with no source line (see
/// [`is_output_routine`]) are labelled `output routine`. At most 10 boxes are
/// listed unless `verbose` lists them all. Empty when no box exceeds 1pt, so
/// callers print exactly what they print today for clean builds.
pub(crate) fn format_overfull(warnings: &[CompileWarning], verbose: bool) -> String {
    let mut boxes: Vec<(&CompileWarning, f32)> = warnings
        .iter()
        .filter(|w| w.kind == "overfull hbox")
        .filter_map(|w| {
            w.width_pt
                .filter(|width| *width > 1.0)
                .map(|width| (w, width))
        })
        .collect();
    if boxes.is_empty() {
        return String::new();
    }
    boxes.sort_by(|a, b| b.1.total_cmp(&a.1));
    let worst = boxes[0].1;
    let mut out = format!(
        "  ◇ {} overfull box(es), worst {}pt\n",
        boxes.len(),
        fmt_pt(worst)
    );
    let shown = if verbose {
        boxes.len()
    } else {
        boxes.len().min(10)
    };
    for (warning, width) in boxes.iter().take(shown) {
        // No source line means the output routine: either nothing
        // attributed the warning, or the box arose while `\output` was
        // active (which also overrides a file-stack attribution, since the
        // open file is not the box's source). Any zero line reports as
        // `output routine` — a `file:0` detail would point nowhere.
        let label = if warning.line == 0 || is_output_routine(&warning.message) {
            "output routine".to_string()
        } else {
            format!("{}:{}", warning.file, warning.line)
        };
        out.push_str(&format!("    {label}  {}pt too wide\n", fmt_pt(*width)));
    }
    out
}

/// Write the engine's complete log text to `path`, creating parent
/// directories as needed.
pub(crate) fn write_log(path: &Path, raw: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
    }
    std::fs::write(path, raw).with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
}

/// The `--max-overfull` failure message for the widest reported box.
pub(crate) fn overfull_gate_message(worst_pt: f32, max_pt: f32) -> String {
    format!(
        "overfull box of {}pt exceeds --max-overfull {}pt",
        fmt_pt(worst_pt),
        fmt_pt(max_pt)
    )
}

/// True when any reported overfull box (wider than 1pt, the set
/// [`format_overfull`] lists) exceeds `max_pt`.
pub(crate) fn exceeds_max(warnings: &[CompileWarning], max_pt: f32) -> Option<f32> {
    warnings
        .iter()
        .filter(|w| w.kind == "overfull hbox")
        .filter_map(|w| w.width_pt.filter(|width| *width > 1.0))
        .fold(None, |worst: Option<f32>, width| {
            Some(worst.map_or(width, |m| m.max(width)))
        })
        .filter(|worst| *worst > max_pt)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::linter::Severity;

    fn warning(file: &str, line: usize, message: &str) -> CompileWarning {
        let width_pt = if message.contains("Overfull \\hbox") {
            parse_width(message)
        } else {
            None
        };
        CompileWarning {
            file: file.to_string(),
            line,
            severity: Severity::Warning,
            kind: "overfull hbox",
            message: message.to_string(),
            width_pt,
        }
    }

    #[test]
    fn parse_width_reads_the_too_wide_number() {
        assert_eq!(
            parse_width("Overfull \\hbox (12.0pt too wide) in paragraph at lines 41--42"),
            Some(12.0)
        );
        assert_eq!(parse_width("Overfull \\hbox (3.25pt too wide)"), Some(3.25));
    }

    #[test]
    fn parse_width_rejects_other_kinds_and_garbage() {
        assert_eq!(parse_width("Underfull \\hbox (badness 1856)"), None);
        assert_eq!(parse_width("Overfull \\vbox (12.0pt too wide)"), None);
        assert_eq!(parse_width("Overfull \\hbox (wide)"), None);
        assert_eq!(parse_width(""), None);
    }

    #[test]
    fn output_routine_detects_the_active_output_marker() {
        assert!(is_output_routine(
            "Overfull \\hbox (12.0pt too wide) has occurred while \\output is active"
        ));
        assert!(!is_output_routine(
            "Overfull \\hbox (12.0pt too wide) in paragraph at lines 41--42"
        ));
    }

    #[test]
    fn range_line_reports_the_first_line() {
        let warnings = vec![warning(
            "main.tex",
            41,
            "Overfull \\hbox (12.0pt too wide) in paragraph at lines 41--42",
        )];
        let out = format_overfull(&warnings, false);
        assert!(out.contains("1 overfull box(es), worst 12pt"), "{out}");
        assert!(out.contains("    main.tex:41  12pt too wide"), "{out}");
    }

    #[test]
    fn on_input_line_reports_that_line() {
        let warnings = vec![warning(
            "ch.tex",
            7,
            "Overfull \\hbox (5.5pt too wide) on input line 7.",
        )];
        let out = format_overfull(&warnings, false);
        assert!(out.contains("ch.tex:7  5.5pt too wide"), "{out}");
    }

    #[test]
    fn output_routine_box_has_no_file_line() {
        let warnings = vec![warning(
            "",
            0,
            "Overfull \\hbox (20.0pt too wide) has occurred while \\output is active",
        )];
        let out = format_overfull(&warnings, false);
        assert!(out.contains("    output routine  20pt too wide"), "{out}");
        assert!(!out.contains(':'));
    }

    #[test]
    fn output_routine_marker_overrides_file_stack_attribution() {
        // The engine prints the box while a file is open, so the parser
        // credits the file — but with no line and the output-routine
        // marker, the report still says `output routine`.
        let warnings = vec![warning(
            "main.tex",
            0,
            "Overfull \\hbox (20.0pt too wide) has occurred while \\output is active",
        )];
        let out = format_overfull(&warnings, false);
        assert!(out.contains("    output routine  20pt too wide"), "{out}");
        assert!(!out.contains("main.tex"), "{out}");
    }

    #[test]
    fn attributed_file_without_a_line_is_still_the_output_routine() {
        // The file stack can credit a box to the open file while the log
        // line carries no `at lines` / `on input line` marker: a `file:0`
        // detail would point nowhere, so it reports as `output routine`.
        let warnings = vec![warning(
            "main.tex",
            0,
            "Overfull \\hbox (7.0pt too wide) in paragraph",
        )];
        let out = format_overfull(&warnings, false);
        assert!(out.contains("    output routine  7pt too wide"), "{out}");
        assert!(!out.contains("main.tex"), "{out}");
    }

    #[test]
    fn clean_log_formats_to_nothing() {
        assert_eq!(format_overfull(&[], false), "");
        assert_eq!(format_overfull(&[], true), "");
        let warnings = vec![CompileWarning {
            file: "main.tex".to_string(),
            line: 58,
            severity: Severity::Warning,
            kind: "underfull hbox",
            message: "Underfull \\hbox (badness 1856)".to_string(),
            width_pt: None,
        }];
        assert_eq!(format_overfull(&warnings, false), "");
    }

    #[test]
    fn sub_point_box_is_ignored_everywhere() {
        let warnings = vec![
            warning(
                "main.tex",
                3,
                "Overfull \\hbox (0.4pt too wide) in paragraph",
            ),
            warning(
                "main.tex",
                9,
                "Overfull \\hbox (2.0pt too wide) in paragraph",
            ),
        ];
        let out = format_overfull(&warnings, false);
        assert!(out.contains("1 overfull box(es), worst 2pt"), "{out}");
        assert!(!out.contains("0.4pt"), "{out}");
        assert_eq!(exceeds_max(&warnings, 1.5), Some(2.0));
        assert_eq!(exceeds_max(&warnings, 2.0), None);
    }

    #[test]
    fn gate_message_names_both_widths() {
        assert_eq!(
            overfull_gate_message(12.0, 5.0),
            "overfull box of 12pt exceeds --max-overfull 5pt"
        );
    }

    #[test]
    fn boxes_sort_widest_first_with_worst_in_the_header() {
        let warnings = vec![
            warning("a.tex", 1, "Overfull \\hbox (2.0pt too wide) x"),
            warning("b.tex", 2, "Overfull \\hbox (9.0pt too wide) y"),
            warning("c.tex", 3, "Overfull \\hbox (5.0pt too wide) z"),
        ];
        let out = format_overfull(&warnings, false);
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 4, "{out}");
        assert!(lines[0].contains("3 overfull box(es), worst 9pt"), "{out}");
        assert!(lines[1].contains("b.tex:2"), "{out}");
        assert!(lines[2].contains("c.tex:3"), "{out}");
        assert!(lines[3].contains("a.tex:1"), "{out}");
    }

    #[test]
    fn long_lists_cap_at_ten_unless_verbose() {
        let warnings: Vec<CompileWarning> = (2..=13)
            .map(|i| {
                warning(
                    "main.tex",
                    i,
                    &format!("Overfull \\hbox ({i}.0pt too wide) in paragraph"),
                )
            })
            .collect();
        let short = format_overfull(&warnings, false);
        assert!(short.contains("12 overfull box(es), worst 13pt"), "{short}");
        assert_eq!(short.lines().count(), 11, "{short}");
        assert!(short.contains("main.tex:4"), "{short}");
        assert!(!short.contains("main.tex:2"), "{short}");
        assert!(!short.contains("main.tex:3"), "{short}");
        let all = format_overfull(&warnings, true);
        assert_eq!(all.lines().count(), 13, "{all}");
        assert!(all.contains("main.tex:2"), "{all}");
    }

    #[test]
    fn write_log_creates_missing_parents() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("build").join("main.log");
        write_log(&path, "engine output\n").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "engine output\n");
    }
}
