//! Shared helpers for scanning LaTeX text.
//!
//! These were extracted from [`crate::linter`] so that the linter and the
//! tokenizer ([`crate::texparse`]) agree on comment stripping and `\input`
//! traversal instead of maintaining two independent copies.
//!
//! The environment-option parser ([`parse_opts`] / [`find_end_tag`]) is shared
//! by [`crate::diagrams`] and [`crate::highlight`], so an author writing
//! `[key=value, …]` options gets identical brace handling, identical warnings
//! and identical unterminated-brace errors in diagram and code blocks.
//!
//! [`compose_accent`] is the accent table shared by the spell checker and the
//! diagram pre-processor.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// Result of a recursive `\input` traversal.
#[derive(Debug, Default)]
pub struct TexFileCollection {
    /// All `.tex` files reachable from the entry point, in traversal order.
    pub files: Vec<PathBuf>,
    /// `\input` targets that referenced an already-visited file. Each entry is
    /// the `(entry, resolved_path)` pair that triggered the cycle.
    pub circular: Vec<(String, PathBuf)>,
}

/// Resolve a tex input path, adding `.tex` extension if missing.
pub fn resolve_tex_path(root: &Path, input: &str) -> PathBuf {
    let p = root.join(input);
    if p.extension().is_some() {
        p
    } else {
        p.with_extension("tex")
    }
}

/// Remove empty LaTeX groups (`{}`) from a source token. They produce no
/// glyph — `workf{}lows` is the recommended fix for the ligature `workflows`,
/// so it must be searched for as `workflows`, not penalized for following
/// the tool's own suggestion. Shared by the PDF fidelity check and the spell
/// checker so both treat the idiom the same way.
pub fn strip_empty_groups(word: &str) -> String {
    word.replace("{}", "")
}

/// Strip a LaTeX comment from a line: everything after an unescaped `%`.
pub fn strip_comment(line: &str) -> String {
    let mut result = String::with_capacity(line.len());
    let mut prev_backslash = false;

    for c in line.chars() {
        if c == '%' && !prev_backslash {
            break;
        }
        prev_backslash = c == '\\';
        result.push(c);
    }

    result
}

/// Extract arguments from `\command{arg}` and `\command[opts]{arg}` occurrences in a line.
pub fn extract_commands<'a>(line: &'a str, cmd: &str) -> Vec<&'a str> {
    let mut results = Vec::new();
    let pattern = format!("\\{}", cmd);
    let mut search = line;

    while let Some(pos) = search.find(&pattern) {
        let after = &search[pos + pattern.len()..];
        // Skip optional args [...]
        let after = if after.starts_with('[') {
            match after.find(']') {
                Some(end) => &after[end + 1..],
                None => break,
            }
        } else {
            after
        };
        if after.starts_with('{') {
            if let Some(end) = after.find('}') {
                let arg = after[1..end].trim();
                if !arg.is_empty() {
                    results.push(arg);
                }
                search = &after[end + 1..];
                continue;
            }
        }
        search = after;
    }

    results
}

/// Recursively collect `.tex` files referenced by `\input{}` from `entry`.
pub fn collect_tex_files(root: &Path, entry: &str) -> TexFileCollection {
    let mut collection = TexFileCollection::default();
    collect_tex_files_inner(root, entry, &mut collection);
    collection
}

fn collect_tex_files_inner(root: &Path, entry: &str, collection: &mut TexFileCollection) {
    let path = resolve_tex_path(root, entry);
    if !path.exists() {
        return;
    }
    if collection.files.contains(&path) {
        collection.circular.push((entry.to_string(), path));
        return;
    }
    collection.files.push(path.clone());

    if let Ok(content) = std::fs::read_to_string(&path) {
        for line in content.lines() {
            let line = strip_comment(line);
            for input in extract_commands(&line, "input") {
                collect_tex_files_inner(root, input, collection);
            }
        }
    }
}

/// Find the end tag position and validate it exists.
///
/// Moved from `crate::diagrams` unchanged: the code-listing pass runs the
/// same begin/end search over its own environments, so a `\begin{code}`
/// without `\end{code}` fails exactly like a missing diagram `\end`.
pub fn find_end_tag(after_opts: &str, end_tag: &str, env: &str) -> Result<usize> {
    after_opts
        .find(end_tag)
        .with_context(|| format!("\\begin{{{}}} without matching \\end{{{}}}", env, env))
}

/// Parse `[key=val, key2=val2]` into a map. Returns `(map, rest_of_str)`.
///
/// `env` is the label warnings and errors are reported under (diagrams pass
/// `"mermaid diagram"`, the highlight pass passes `"code"` /
/// `"lstlisting"`), `known` is the option keys the caller actually consumes.
///
/// A value may be wrapped in `{...}` — the LaTeX convention for "this may
/// contain a comma" — in which case the comma inside no longer separates
/// options; the outer braces are stripped from the stored value but nested
/// braces are preserved. Brace depth is tracked in a single pass over the
/// string, so an option is only split on a comma seen at depth zero.
///
/// An unrecognised key emits a warning (naming `env`) and is dropped rather
/// than aborting the build; an unterminated `{` is a hard error, since there
/// is no reasonable place to guess the value ended.
pub fn parse_opts<'a>(
    s: &'a str,
    env: &str,
    known: &[&str],
) -> Result<(HashMap<String, String>, &'a str)> {
    let s = s.trim_start_matches('\n').trim_start_matches('\r');
    if !s.starts_with('[') {
        return Ok((HashMap::new(), s));
    }
    let after = &s[1..];

    let mut depth = 0i32;
    let mut part_start = 0usize;
    let mut parts: Vec<&str> = Vec::new();
    let mut end_idx = None;
    for (i, c) in after.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => depth -= 1,
            ',' if depth == 0 => {
                parts.push(&after[part_start..i]);
                part_start = i + 1;
            }
            ']' if depth == 0 => {
                parts.push(&after[part_start..i]);
                end_idx = Some(i);
                break;
            }
            _ => {}
        }
    }

    let Some(end_idx) = end_idx else {
        if depth > 0 {
            let unterminated = &after[part_start..];
            let option = unterminated
                .split_once('=')
                .map_or(unterminated, |(k, _)| k)
                .trim();
            anyhow::bail!(
                "{env}: unterminated '{{' in option '{option}' — every {{ needs a matching }}"
            );
        }
        return Ok((HashMap::new(), s));
    };
    let rest = &after[end_idx + 1..];

    let mut map = HashMap::new();
    for part in parts {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let Some((k, v)) = part.split_once('=') else {
            eprintln!("warning: {env}: unknown option '{part}' ignored");
            continue;
        };
        let k = k.trim();
        let v = v.trim();
        if !known.contains(&k) {
            eprintln!("warning: {env}: unknown option '{k}' ignored");
            continue;
        }
        let value = v
            .strip_prefix('{')
            .and_then(|v| v.strip_suffix('}'))
            .unwrap_or(v);
        map.insert(k.to_string(), value.to_string());
    }
    Ok((map, rest))
}

/// Compose a LaTeX accent command with its base letter.
///
/// `accent` is the command name without the backslash (`'`, `` ` ``, `^`,
/// `"`, `~`, `=`, `.`, or a letter form `c`/`v`/`u`/`H`/`r`/`k`); `base` is
/// the ASCII letter the accent applies to. Returns `None` for a combination
/// Unicode has no precomposed character for.
///
/// Shared by the spell checker, which resolves accents while tokenizing, and
/// by [`crate::diagrams`], which rewrites accents in diagram sources to the
/// UTF-8 character because Mermaid, D2 and Graphviz do not read LaTeX.
pub(crate) fn compose_accent(accent: char, base: char) -> Option<char> {
    match accent {
        '\'' => acute_composed(base),
        '`' => grave_composed(base),
        '^' => circumflex_composed(base),
        '"' => umlaut_composed(base),
        '~' => tilde_composed(base),
        '=' => macron_composed(base),
        '.' => dot_composed(base),
        'c' => cedilla_composed(base),
        'v' => caron_composed(base),
        'u' => breve_composed(base),
        'H' => double_acute_composed(base),
        'r' => ring_composed(base),
        'k' => ogonek_composed(base),
        _ => None,
    }
}

fn acute_composed(base: char) -> Option<char> {
    Some(match base {
        'a' => 'á',
        'e' => 'é',
        'i' => 'í',
        'o' => 'ó',
        'u' => 'ú',
        'y' => 'ý',
        'A' => 'Á',
        'E' => 'É',
        'I' => 'Í',
        'O' => 'Ó',
        'U' => 'Ú',
        'Y' => 'Ý',
        _ => return None,
    })
}

fn grave_composed(base: char) -> Option<char> {
    Some(match base {
        'a' => 'à',
        'e' => 'è',
        'i' => 'ì',
        'o' => 'ò',
        'u' => 'ù',
        'A' => 'À',
        'E' => 'È',
        'I' => 'Ì',
        'O' => 'Ò',
        'U' => 'Ù',
        _ => return None,
    })
}

fn circumflex_composed(base: char) -> Option<char> {
    Some(match base {
        'a' => 'â',
        'e' => 'ê',
        'i' => 'î',
        'o' => 'ô',
        'u' => 'û',
        'A' => 'Â',
        'E' => 'Ê',
        'I' => 'Î',
        'O' => 'Ô',
        'U' => 'Û',
        _ => return None,
    })
}

fn umlaut_composed(base: char) -> Option<char> {
    Some(match base {
        'a' => 'ä',
        'e' => 'ë',
        'i' => 'ï',
        'o' => 'ö',
        'u' => 'ü',
        'A' => 'Ä',
        'E' => 'Ë',
        'I' => 'Ï',
        'O' => 'Ö',
        'U' => 'Ü',
        _ => return None,
    })
}

fn tilde_composed(base: char) -> Option<char> {
    Some(match base {
        'a' => 'ã',
        'n' => 'ñ',
        'o' => 'õ',
        'A' => 'Ã',
        'N' => 'Ñ',
        'O' => 'Õ',
        _ => return None,
    })
}

fn macron_composed(base: char) -> Option<char> {
    Some(match base {
        'a' => 'ā',
        'e' => 'ē',
        'i' => 'ī',
        'o' => 'ō',
        'u' => 'ū',
        'A' => 'Ā',
        'E' => 'Ē',
        'I' => 'Ī',
        'O' => 'Ō',
        'U' => 'Ū',
        _ => return None,
    })
}

fn dot_composed(base: char) -> Option<char> {
    Some(match base {
        'a' => 'ȧ',
        'e' => 'ė',
        'o' => 'ȯ',
        'A' => 'Ȧ',
        'E' => 'Ė',
        'O' => 'Ȯ',
        _ => return None,
    })
}

fn cedilla_composed(base: char) -> Option<char> {
    Some(match base {
        'c' => 'ç',
        'C' => 'Ç',
        's' => 'ş',
        'S' => 'Ş',
        't' => 'ţ',
        'T' => 'Ţ',
        _ => return None,
    })
}

fn caron_composed(base: char) -> Option<char> {
    Some(match base {
        'c' => 'č',
        'C' => 'Č',
        's' => 'š',
        'S' => 'Š',
        'z' => 'ž',
        'Z' => 'Ž',
        'e' => 'ě',
        'E' => 'Ě',
        'r' => 'ř',
        'R' => 'Ř',
        'n' => 'ň',
        'N' => 'Ň',
        _ => return None,
    })
}

fn breve_composed(base: char) -> Option<char> {
    Some(match base {
        'a' => 'ă',
        'A' => 'Ă',
        'e' => 'ĕ',
        'E' => 'Ĕ',
        'i' => 'ĭ',
        'I' => 'Ĭ',
        'o' => 'ŏ',
        'O' => 'Ŏ',
        'u' => 'ŭ',
        'U' => 'Ŭ',
        _ => return None,
    })
}

fn double_acute_composed(base: char) -> Option<char> {
    Some(match base {
        'o' => 'ő',
        'O' => 'Ő',
        'u' => 'ű',
        'U' => 'Ű',
        _ => return None,
    })
}

fn ring_composed(base: char) -> Option<char> {
    Some(match base {
        'a' => 'å',
        'A' => 'Å',
        'u' => 'ů',
        'U' => 'Ů',
        _ => return None,
    })
}

fn ogonek_composed(base: char) -> Option<char> {
    Some(match base {
        'a' => 'ą',
        'A' => 'Ą',
        'e' => 'ę',
        'E' => 'Ę',
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_empty_groups_joins_the_ligature_workaround() {
        assert_eq!(strip_empty_groups("workf{}lows"), "workflows");
        assert_eq!(strip_empty_groups("Artif{}icial"), "Artificial");
        assert_eq!(strip_empty_groups("MLf{}low"), "MLflow");
    }

    #[test]
    fn strip_empty_groups_handles_leading_trailing_and_doubled_groups() {
        assert_eq!(strip_empty_groups("{}Word"), "Word");
        assert_eq!(strip_empty_groups("Word{}"), "Word");
        assert_eq!(strip_empty_groups("Mid{}dle{}Point"), "MiddlePoint");
    }

    #[test]
    fn strip_empty_groups_leaves_non_empty_groups_alone() {
        assert_eq!(strip_empty_groups("\\textit{foo}"), "\\textit{foo}");
        assert_eq!(strip_empty_groups("plain"), "plain");
    }

    // --- the shared environment-option parser (diagrams + highlight) ---

    #[test]
    fn parse_opts_without_brackets_returns_empty_map_and_rest() {
        let (map, rest) = parse_opts("hello", "code", &["lang"]).unwrap();
        assert!(map.is_empty());
        assert_eq!(rest, "hello");
    }

    #[test]
    fn parse_opts_braced_value_keeps_commas_and_nested_braces() {
        let (map, _) = parse_opts(
            "[caption={\\texttt{a, b}}]",
            "mermaid diagram",
            &["caption"],
        )
        .unwrap();
        assert_eq!(
            map.get("caption").map(String::as_str),
            Some("\\texttt{a, b}")
        );
    }

    #[test]
    fn parse_opts_key_outside_known_list_is_dropped() {
        let (map, rest) = parse_opts("[lang=python, frobnicate=yes]", "code", &["lang"]).unwrap();
        assert_eq!(map.get("lang").map(String::as_str), Some("python"));
        assert!(!map.contains_key("frobnicate"));
        assert!(rest.is_empty());
    }

    #[test]
    fn parse_opts_unterminated_brace_is_an_error_naming_the_label() {
        let err = parse_opts("[lang={unterminated]", "code", &["lang"]).unwrap_err();
        let message = err.to_string();
        assert!(message.contains("code"), "message: {message}");
        assert!(message.contains("lang"), "message: {message}");
    }

    /// An unclosed `[` with balanced braces is not an option list at all:
    /// the parser hands the text back untouched instead of reporting a
    /// phantom unterminated `{`.
    #[test]
    fn parse_opts_unclosed_bracket_is_not_an_option_list() {
        let (map, rest) = parse_opts("[lang=python", "code", &["lang"]).unwrap();
        assert!(map.is_empty());
        assert_eq!(rest, "[lang=python");
    }

    #[test]
    fn find_end_tag_locates_the_matching_end() {
        let end = find_end_tag("body\n\\end{code}", "\\end{code}", "code").unwrap();
        assert_eq!(end, 5);
    }

    #[test]
    fn find_end_tag_missing_end_names_both_tags() {
        let err = find_end_tag("body", "\\end{code}", "code").unwrap_err();
        let message = err.to_string();
        assert!(message.contains("\\begin{code}"), "message: {message}");
        assert!(message.contains("\\end{code}"), "message: {message}");
    }

    // --- the shared accent table (spell checker + diagrams) ---

    fn assert_table(accent: char, pairs: &[(char, char)]) {
        for &(base, expected) in pairs {
            assert_eq!(compose_accent(accent, base), Some(expected), "{base}");
        }
        assert_eq!(compose_accent(accent, 'x'), None);
    }

    #[test]
    fn acute_table_covers_every_arm() {
        assert_table(
            '\'',
            &[
                ('a', 'á'),
                ('e', 'é'),
                ('i', 'í'),
                ('o', 'ó'),
                ('u', 'ú'),
                ('y', 'ý'),
                ('A', 'Á'),
                ('E', 'É'),
                ('I', 'Í'),
                ('O', 'Ó'),
                ('U', 'Ú'),
                ('Y', 'Ý'),
            ],
        );
    }

    #[test]
    fn grave_table_covers_every_arm() {
        assert_table(
            '`',
            &[
                ('a', 'à'),
                ('e', 'è'),
                ('i', 'ì'),
                ('o', 'ò'),
                ('u', 'ù'),
                ('A', 'À'),
                ('E', 'È'),
                ('I', 'Ì'),
                ('O', 'Ò'),
                ('U', 'Ù'),
            ],
        );
    }

    #[test]
    fn circumflex_table_covers_every_arm() {
        assert_table(
            '^',
            &[
                ('a', 'â'),
                ('e', 'ê'),
                ('i', 'î'),
                ('o', 'ô'),
                ('u', 'û'),
                ('A', 'Â'),
                ('E', 'Ê'),
                ('I', 'Î'),
                ('O', 'Ô'),
                ('U', 'Û'),
            ],
        );
    }

    #[test]
    fn umlaut_table_covers_every_arm() {
        assert_table(
            '"',
            &[
                ('a', 'ä'),
                ('e', 'ë'),
                ('i', 'ï'),
                ('o', 'ö'),
                ('u', 'ü'),
                ('A', 'Ä'),
                ('E', 'Ë'),
                ('I', 'Ï'),
                ('O', 'Ö'),
                ('U', 'Ü'),
            ],
        );
    }

    #[test]
    fn tilde_table_covers_every_arm() {
        assert_table(
            '~',
            &[
                ('a', 'ã'),
                ('n', 'ñ'),
                ('o', 'õ'),
                ('A', 'Ã'),
                ('N', 'Ñ'),
                ('O', 'Õ'),
            ],
        );
    }

    #[test]
    fn macron_table_covers_every_arm() {
        assert_table(
            '=',
            &[
                ('a', 'ā'),
                ('e', 'ē'),
                ('i', 'ī'),
                ('o', 'ō'),
                ('u', 'ū'),
                ('A', 'Ā'),
                ('E', 'Ē'),
                ('I', 'Ī'),
                ('O', 'Ō'),
                ('U', 'Ū'),
            ],
        );
    }

    #[test]
    fn dot_table_covers_every_arm() {
        assert_table(
            '.',
            &[
                ('a', 'ȧ'),
                ('e', 'ė'),
                ('o', 'ȯ'),
                ('A', 'Ȧ'),
                ('E', 'Ė'),
                ('O', 'Ȯ'),
            ],
        );
    }

    #[test]
    fn cedilla_table_covers_every_arm() {
        assert_table(
            'c',
            &[
                ('c', 'ç'),
                ('C', 'Ç'),
                ('s', 'ş'),
                ('S', 'Ş'),
                ('t', 'ţ'),
                ('T', 'Ţ'),
            ],
        );
    }

    #[test]
    fn caron_table_covers_every_arm() {
        assert_table(
            'v',
            &[
                ('c', 'č'),
                ('C', 'Č'),
                ('s', 'š'),
                ('S', 'Š'),
                ('z', 'ž'),
                ('Z', 'Ž'),
                ('e', 'ě'),
                ('E', 'Ě'),
                ('r', 'ř'),
                ('R', 'Ř'),
                ('n', 'ň'),
                ('N', 'Ň'),
            ],
        );
    }

    #[test]
    fn breve_table_covers_every_arm() {
        assert_table(
            'u',
            &[
                ('a', 'ă'),
                ('A', 'Ă'),
                ('e', 'ĕ'),
                ('E', 'Ĕ'),
                ('i', 'ĭ'),
                ('I', 'Ĭ'),
                ('o', 'ŏ'),
                ('O', 'Ŏ'),
                ('u', 'ŭ'),
                ('U', 'Ŭ'),
            ],
        );
    }

    #[test]
    fn double_acute_table_covers_every_arm() {
        assert_table('H', &[('o', 'ő'), ('O', 'Ő'), ('u', 'ű'), ('U', 'Ű')]);
    }

    #[test]
    fn ring_table_covers_every_arm() {
        assert_table('r', &[('a', 'å'), ('A', 'Å'), ('u', 'ů'), ('U', 'Ů')]);
    }

    #[test]
    fn ogonek_table_covers_every_arm() {
        assert_table('k', &[('a', 'ą'), ('A', 'Ą'), ('e', 'ę'), ('E', 'Ę')]);
    }

    #[test]
    fn unknown_accent_composes_nothing() {
        assert_eq!(compose_accent('?', 'e'), None);
        assert_eq!(compose_accent('\'', 'x'), None);
    }
}
