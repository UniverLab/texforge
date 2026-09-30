//! Accent macro recognition and composition: `\'`, `\v{c}`, `\ij`,
//! transparent `\-`/`\/`, and the command -> character tables behind them.

use crate::texparse::{SpannedToken, Token};

const ACCENT_COMMANDS: &[&str] = &[
    "'", "`", "^", "\"", "~", "=", ".", "c", "v", "u", "H", "r", "k",
];

/// Commands that render no character at all and must therefore be *skipped*
/// rather than treated as a word break. `\-` is a discretionary hyphen — a
/// hint about where a line may break — and `\/` is an italic correction;
/// both appear inside words. Measured on a real document: `impor\-tancia`
/// was being reported as the two unknown words `impor` and `tancia`.
const TRANSPARENT_COMMANDS: &[&str] = &["-", "/"];

pub(super) fn is_transparent_command(name: &str) -> bool {
    TRANSPARENT_COMMANDS.contains(&name)
}

pub(super) fn is_accent_command(name: &str) -> bool {
    ACCENT_COMMANDS.contains(&name)
}

fn is_letter_form_accent(name: &str) -> bool {
    matches!(name, "c" | "v" | "u" | "H" | "r" | "k")
}

fn accent_to_char(name: &str) -> Option<char> {
    match name {
        "'" => Some('\''),
        "`" => Some('`'),
        "^" => Some('^'),
        "\"" => Some('"'),
        "~" => Some('~'),
        "=" => Some('='),
        "." => Some('.'),
        "c" => Some('c'),
        "v" => Some('v'),
        "u" => Some('u'),
        "H" => Some('H'),
        "r" => Some('r'),
        "k" => Some('k'),
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

fn compose_accent(accent: char, base: char) -> Option<char> {
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

fn extract_base_from_text_start(text: &str) -> Option<(char, usize)> {
    let first_non_ws = text.find(|c: char| !c.is_whitespace())?;
    let remaining = &text[first_non_ws..];

    if remaining.starts_with('{') && remaining.len() >= 3 {
        let inner = &remaining[1..];
        if let Some(base) = inner.chars().next() {
            if base.is_ascii_alphabetic() {
                let after_base = &inner[base.len_utf8()..];
                if after_base.starts_with('}') {
                    let total_skip = first_non_ws + 1 + base.len_utf8() + 1;
                    return Some((base, total_skip));
                }
            }
        }
    }

    let base = remaining.chars().next()?;
    if base.is_ascii_alphabetic() {
        Some((base, first_non_ws + base.len_utf8()))
    } else {
        None
    }
}

pub(super) enum AccentBaseSource {
    FromArgs,
    FromNextText {
        chars_to_skip: usize,
    },
    FromDotlessIJ {
        extra_tokens_to_skip: usize,
        chars_to_skip_in_last: usize,
    },
}

fn resolve_accent_from_args(
    name: &str,
    args: &[String],
    accent_char: char,
) -> Option<(char, AccentBaseSource)> {
    if !is_letter_form_accent(name) || args.is_empty() {
        return None;
    }
    let arg = args[0].trim();
    if arg.len() != 1 {
        return None;
    }
    let base = arg.chars().next()?;
    if !base.is_ascii_alphabetic() {
        return None;
    }
    compose_accent(accent_char, base).map(|composed| (composed, AccentBaseSource::FromArgs))
}

fn resolve_dotless_ij(
    accent_char: char,
    tokens: &[SpannedToken],
    inner_idx: usize,
) -> Option<(char, AccentBaseSource)> {
    let inner = tokens.get(inner_idx)?;
    let Token::Command {
        name: ij_name,
        args: ij_args,
    } = &inner.token
    else {
        return None;
    };
    if !(ij_name == "i" || ij_name == "j") || !ij_args.is_empty() {
        return None;
    }
    let closing_token = tokens.get(inner_idx + 1)?;
    let Token::Text(closing) = &closing_token.token else {
        return None;
    };
    if !closing.starts_with('}') {
        return None;
    }
    let base = if ij_name == "i" { 'i' } else { 'j' };
    let composed = compose_accent(accent_char, base)?;
    let chars_to_skip = usize::from(closing.len() > 1);
    Some((
        composed,
        AccentBaseSource::FromDotlessIJ {
            extra_tokens_to_skip: 3,
            chars_to_skip_in_last: chars_to_skip,
        },
    ))
}

fn resolve_accent_from_next_text(
    accent_char: char,
    text: &str,
) -> Option<(char, AccentBaseSource)> {
    let (base, skip) = extract_base_from_text_start(text)?;
    let composed = compose_accent(accent_char, base)?;
    Some((
        composed,
        AccentBaseSource::FromNextText {
            chars_to_skip: skip,
        },
    ))
}

pub(super) fn try_resolve_accent(
    name: &str,
    args: &[String],
    tokens: &[SpannedToken],
    accent_idx: usize,
) -> Option<(char, AccentBaseSource)> {
    let accent_char = accent_to_char(name)?;

    if let Some(resolved) = resolve_accent_from_args(name, args, accent_char) {
        return Some(resolved);
    }

    let next_idx = accent_idx + 1;
    let next_token = tokens.get(next_idx)?;
    let Token::Text(text) = &next_token.token else {
        return None;
    };

    if text == "{" {
        let inner_idx = next_idx + 1;
        if let Some(resolved) = resolve_dotless_ij(accent_char, tokens, inner_idx) {
            return Some(resolved);
        }
    }

    resolve_accent_from_next_text(accent_char, text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    use super::super::lint_files;
    use super::super::test_support::run_with_home;

    #[test]
    fn symbol_form_accents_resolve_brace_and_direct() {
        run_with_home("violación\n", "hello\nworld\n", || {
            let src = "\\usepackage[spanish]{babel}\n\\begin{document}\n\
                        violaci\\'{o}n\n\\end{document}";
            let files = vec![("main.tex".to_string(), src.to_string())];
            let project_root = TempDir::new().unwrap();
            let findings = lint_files(&files, project_root.path(), None).unwrap();
            let unknown: Vec<_> = findings
                .iter()
                .filter(|f| f.message.contains("Unknown word"))
                .collect();
            assert!(
                unknown.is_empty(),
                "violación (brace form) must not be flagged: {:?}",
                unknown
            );
        });
    }

    #[test]
    fn symbol_form_accents_resolve_space_form() {
        run_with_home("café\n", "hello\nworld\n", || {
            let src = "\\usepackage[spanish]{babel}\n\\begin{document}\n\
                        caf\\' e\n\\end{document}";
            let files = vec![("main.tex".to_string(), src.to_string())];
            let project_root = TempDir::new().unwrap();
            let findings = lint_files(&files, project_root.path(), None).unwrap();
            let unknown: Vec<_> = findings
                .iter()
                .filter(|f| f.message.contains("Unknown word"))
                .collect();
            assert!(
                unknown.is_empty(),
                "café (symbol-form space variant \\' e) must not be flagged: {:?}",
                unknown
            );
        });
    }

    #[test]
    fn letter_form_accents_resolve_brace_form_through_lint_files() {
        run_with_home("français\n", "hello\nworld\n", || {
            let src = "\\usepackage[spanish]{babel}\n\\begin{document}\n\
                        fran\\c{c}ais\n\\end{document}";
            let files = vec![("main.tex".to_string(), src.to_string())];
            let project_root = TempDir::new().unwrap();
            let findings = lint_files(&files, project_root.path(), None).unwrap();
            assert!(
                !findings.iter().any(|f| f.message.contains("fran")),
                "français (letter-form \\c{{c}}) must not produce 'fran': {:?}",
                findings
            );
            assert!(
                !findings.iter().any(|f| f.message.contains("'ais'")),
                "français (letter-form \\c{{c}}) must not eat 'ais': {:?}",
                findings
            );
        });
    }

    #[test]
    fn letter_form_accents_resolve_space_form_through_lint_files() {
        run_with_home("č\n", "hello\nworld\n", || {
            let src = "\\usepackage[spanish]{babel}\n\\begin{document}\n\
                        \\v c\n\\end{document}";
            let files = vec![("main.tex".to_string(), src.to_string())];
            let project_root = TempDir::new().unwrap();
            let findings = lint_files(&files, project_root.path(), None).unwrap();
            let unknown: Vec<_> = findings
                .iter()
                .filter(|f| f.message.contains("Unknown word"))
                .collect();
            assert!(
                unknown.is_empty(),
                "\\v c (space form) must resolve: {:?}",
                unknown
            );
        });
    }

    #[test]
    fn spanish_document_with_accent_macros_produces_zero_warnings() {
        run_with_home(
            "universidad\ncoincidencia\ncomparación\nnúmero\nmás\naquí\n",
            "hello\nworld\n",
            || {
                let src = "\\usepackage[spanish]{babel}\n\\begin{document}\n\
                            universidad coincidencia comparaci\\'{o}n n\\'umero \
                            m\\'as aqu\\'{\\i}\n\\end{document}";
                let files = vec![("main.tex".to_string(), src.to_string())];
                let project_root = TempDir::new().unwrap();
                let findings = lint_files(&files, project_root.path(), None).unwrap();
                let unknown: Vec<_> = findings
                    .iter()
                    .filter(|f| f.message.contains("Unknown word"))
                    .collect();
                assert!(
                    unknown.is_empty(),
                    "Spanish document with accent macros must produce zero unknown-word warnings: {:?}",
                    unknown
                );
            },
        );
    }

    #[test]
    fn discretionary_hyphen_does_not_split_a_word() {
        // Measured on a real document: `impor\-tancia` was reported as the
        // two unknown words `impor` and `tancia`. `\-` marks where a line
        // MAY break; it renders nothing and must not break the word here.
        run_with_home("importancia\n", "hello\nworld\n", || {
            let src = "\\usepackage[spanish]{babel}\n\\begin{document}\n\
                        impor\\-tancia\n\\end{document}";
            let files = vec![("main.tex".to_string(), src.to_string())];
            let project_root = TempDir::new().unwrap();
            let findings = lint_files(&files, project_root.path(), None).unwrap();
            let unknown: Vec<_> = findings
                .iter()
                .filter(|f| f.message.contains("Unknown word"))
                .collect();
            assert!(
                unknown.is_empty(),
                "a discretionary hyphen must not split a word: {:?}",
                unknown
            );
        });
    }

    #[test]
    fn document_with_letter_form_macro_produces_zero_warnings() {
        run_with_home("čeština\n", "hello\nworld\n", || {
            let src = "\\usepackage[spanish]{babel}\n\\begin{document}\n\
                        \\v{c}e\\v{s}tina\n\\end{document}";
            let files = vec![("main.tex".to_string(), src.to_string())];
            let project_root = TempDir::new().unwrap();
            let findings = lint_files(&files, project_root.path(), None).unwrap();
            let unknown: Vec<_> = findings
                .iter()
                .filter(|f| f.message.contains("Unknown word"))
                .collect();
            assert!(
                unknown.is_empty(),
                "document with letter-form macros must produce zero warnings: {:?}",
                unknown
            );
        });
    }

    #[test]
    fn misspelling_with_accent_macro_is_reported_as_composed_word() {
        run_with_home("hola\n", "hello\nworld\n", || {
            let src = "\\usepackage[spanish]{babel}\n\\begin{document}\n\
                        xyz\\'{a}bc\n\\end{document}";
            let files = vec![("main.tex".to_string(), src.to_string())];
            let project_root = TempDir::new().unwrap();
            let findings = lint_files(&files, project_root.path(), None).unwrap();
            assert!(
                findings.iter().any(|f| f.message.contains("xyzábc")),
                "misspelling with accent must be reported as composed word 'xyzábc': {:?}",
                findings
            );
        });
    }

    #[test]
    fn fran_c_ais_does_not_eat_following_words() {
        run_with_home(
            "français\nmás\npalabras\n",
            "hello\nworld\nmore\nwords\n",
            || {
                let src = "\\usepackage[spanish]{babel}\n\\begin{document}\n\
                        fran\\c{c}ais m\\'as palabras\n\\end{document}";
                let files = vec![("main.tex".to_string(), src.to_string())];
                let project_root = TempDir::new().unwrap();
                let findings = lint_files(&files, project_root.path(), None).unwrap();
                let unknown: Vec<_> = findings
                    .iter()
                    .filter(|f| f.message.contains("Unknown word"))
                    .collect();
                assert!(
                    unknown.is_empty(),
                    "fran\\c{{c}}ais m\\'as palabras must produce zero unknown-word warnings \
                 (must not eat 'ais', 'm\\'as', or 'palabras'): {:?}",
                    unknown
                );
            },
        );
    }

    #[test]
    fn unknown_macro_breaks_word_rather_than_absorbing_letters() {
        run_with_home("hola\n", "hello\nworld\n", || {
            let src = "\\usepackage[spanish]{babel}\n\\begin{document}\n\
                        abc\\unknownmacro def\n\\end{document}";
            let files = vec![("main.tex".to_string(), src.to_string())];
            let project_root = TempDir::new().unwrap();
            let findings = lint_files(&files, project_root.path(), None).unwrap();
            let unknown_words: Vec<_> = findings
                .iter()
                .filter_map(|f| {
                    f.message
                        .strip_prefix("Unknown word: '")
                        .and_then(|s| s.strip_suffix('\''))
                        .map(String::from)
                })
                .collect();
            assert!(
                unknown_words.contains(&"abc".to_string()),
                "unknown macro must break word, leaving 'abc' to be checked: {:?}",
                unknown_words
            );
            assert!(
                unknown_words.contains(&"def".to_string()),
                "text after unknown macro must not be swallowed: {:?}",
                unknown_words
            );
        });
    }

    #[test]
    fn dotless_i_with_accent_resolves() {
        run_with_home("mercurio\níndice\n", "hello\nworld\n", || {
            let src = "\\usepackage[spanish]{babel}\n\\begin{document}\n\
                        mercurio \\'{\\i}ndice\n\\end{document}";
            let files = vec![("main.tex".to_string(), src.to_string())];
            let project_root = TempDir::new().unwrap();
            let findings = lint_files(&files, project_root.path(), None).unwrap();
            let unknown: Vec<_> = findings
                .iter()
                .filter(|f| f.message.contains("Unknown word"))
                .collect();
            assert!(
                !unknown.iter().any(|f| f.message.contains("'ndice'")),
                "\\ '{{\\i}} must resolve to 'í', not leave 'ndice': {:?}",
                unknown
            );
        });
    }

    #[test]
    fn all_accent_forms_resolve_via_helper() {
        assert_eq!(compose_accent('\'', 'e'), Some('é'));
        assert_eq!(compose_accent('`', 'a'), Some('à'));
        assert_eq!(compose_accent('^', 'o'), Some('ô'));
        assert_eq!(compose_accent('"', 'u'), Some('ü'));
        assert_eq!(compose_accent('~', 'n'), Some('ñ'));
        assert_eq!(compose_accent('=', 'a'), Some('ā'));
        assert_eq!(compose_accent('.', 'e'), Some('ė'));
        assert_eq!(compose_accent('c', 'c'), Some('ç'));
        assert_eq!(compose_accent('v', 's'), Some('š'));
        assert_eq!(compose_accent('u', 'a'), Some('ă'));
        assert_eq!(compose_accent('H', 'u'), Some('ű'));
        assert_eq!(compose_accent('r', 'a'), Some('å'));
        assert_eq!(compose_accent('k', 'e'), Some('ę'));
    }
}
