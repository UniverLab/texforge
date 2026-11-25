//! Detect whether project sources visibly load fontspec.
//!
//! Text scan over the project sources only (class files live in the Tectonic
//! bundle and are not scanned). A source loads fontspec when, on a
//! comment-stripped line outside verbatim body lines, it contains `\usepackage`
//! or `\RequirePackage` whose braced package list includes `fontspec`, or it
//! contains any of `\setmainfont`, `\setsansfont`, `\setmonofont`,
//! `\newfontfamily`, `\setmathfont`.

use crate::{texparse, texutil};

/// Does any of these project sources visibly load fontspec?
pub(crate) fn fontspec_visible_load(files: &[(String, String)]) -> bool {
    files.iter().any(|(_, content)| {
        let verbatim_lines = texparse::verbatim_body_lines(content);
        content.lines().enumerate().any(|(index, line)| {
            if verbatim_lines.contains(&(index + 1)) {
                return false;
            }
            line_loads_fontspec(line)
        })
    })
}

fn line_loads_fontspec(line: &str) -> bool {
    let line = texutil::strip_comment(line);
    const FONT_CMDS: [&str; 5] = [
        r"\setmainfont",
        r"\setsansfont",
        r"\setmonofont",
        r"\newfontfamily",
        r"\setmathfont",
    ];
    if FONT_CMDS.iter().any(|cmd| line.contains(cmd)) {
        return true;
    }
    [r"\usepackage", r"\RequirePackage"]
        .iter()
        .any(|cmd| scan_cmd_for_fontspec(&line, cmd))
}

/// Scan one package-loading command for a visible `fontspec` load.
fn scan_cmd_for_fontspec(line: &str, cmd: &str) -> bool {
    let mut rest = line;
    for _ in 0..=line.len() {
        let Some(pos) = rest.find(cmd) else {
            break;
        };
        let after_cmd = &rest[pos + cmd.len()..];
        let Some(without_opts) = strip_optional_package_options(after_cmd) else {
            break;
        };
        let after_opts = without_opts.trim_start();
        let Some((loads, next)) = parse_braced_package_list(after_opts) else {
            rest = after_opts;
            continue;
        };
        if loads {
            return true;
        }
        rest = next;
    }
    false
}

/// Strip a `[...]` option list after `\usepackage`/`\RequirePackage`.
///
/// Returns the text after the options, or the input unchanged when there is
/// no option list. Returns `None` when `[` is never closed.
fn strip_optional_package_options(after: &str) -> Option<&str> {
    let Some(stripped) = after.strip_prefix('[') else {
        return Some(after);
    };
    let end = stripped.find(']')?;
    Some(&stripped[end + 1..])
}

/// Parse a `{pkg,...}` list (caller must have trimmed leading whitespace).
/// Returns whether it loads `fontspec` plus the text after `}`.
/// Returns `None` when there is no braced list to parse.
fn parse_braced_package_list(after: &str) -> Option<(bool, &str)> {
    let args = after.strip_prefix('{')?;
    let (list, next) = args.split_once('}')?;
    Some((package_list_loads_fontspec(list), next))
}

/// Does a comma-separated package list load `fontspec`?
fn package_list_loads_fontspec(list: &str) -> bool {
    list.split(',')
        .map(str::trim)
        .any(|name| name == "fontspec")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files(content: &str) -> Vec<(String, String)> {
        vec![("main.tex".to_string(), content.to_string())]
    }

    #[test]
    fn plain_false() {
        assert!(!fontspec_visible_load(&files(
            "\\documentclass{article}\n\\begin{document}\nhi\n\\end{document}\n"
        )));
    }

    #[test]
    fn commented_false() {
        assert!(!fontspec_visible_load(&files(
            "% \\usepackage{fontspec}\n\\begin{document}\\end{document}\n"
        )));
    }

    #[test]
    fn verbatim_false() {
        let src = "\\documentclass{article}\n\\begin{document}\n\
                   \\begin{verbatim}\n\\usepackage{fontspec}\n\\end{verbatim}\n\
                   \\end{document}\n";
        assert!(!fontspec_visible_load(&files(src)));
    }

    #[test]
    fn usepackage_fontspec_true() {
        assert!(fontspec_visible_load(&files(
            "\\usepackage{fontspec}\n\\begin{document}\\end{document}\n"
        )));
    }

    #[test]
    fn usepackage_with_options_and_list_true() {
        assert!(fontspec_visible_load(&files(
            "\\usepackage[x]{fontspec,graphicx}\n\\begin{document}\\end{document}\n"
        )));
    }

    #[test]
    fn requirepackage_true() {
        assert!(fontspec_visible_load(&files(
            "\\RequirePackage{fontspec}\n\\begin{document}\\end{document}\n"
        )));
    }

    #[test]
    fn setmonofont_true() {
        assert!(fontspec_visible_load(&files(
            "\\setmonofont{Foo}\n\\begin{document}\\end{document}\n"
        )));
    }

    #[test]
    fn setmainfont_true() {
        assert!(fontspec_visible_load(&files(
            "\\setmainfont{Foo}\n\\begin{document}\\end{document}\n"
        )));
    }

    #[test]
    fn setsansfont_true() {
        assert!(fontspec_visible_load(&files(
            "\\setsansfont{Foo}\n\\begin{document}\\end{document}\n"
        )));
    }

    #[test]
    fn newfontfamily_true() {
        assert!(fontspec_visible_load(&files(
            "\\newfontfamily\\myfont{Foo}\n\\begin{document}\\end{document}\n"
        )));
    }

    #[test]
    fn setmathfont_true() {
        assert!(fontspec_visible_load(&files(
            "\\setmathfont{Foo}\n\\begin{document}\\end{document}\n"
        )));
    }
}
