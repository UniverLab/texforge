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

/// Base character written by a dotless command: `\i` stands for `i` and
/// `\j` for `j`, so an accent needs no base argument at all. Every other
/// command (and a bare `\i` that carries arguments) has no dotless base.
fn dotless_base(ij_name: &str) -> Option<char> {
    match ij_name {
        "i" => Some('i'),
        "j" => Some('j'),
        _ => None,
    }
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
    if !ij_args.is_empty() {
        return None;
    }
    let base = dotless_base(ij_name)?;
    let closing_token = tokens.get(inner_idx + 1)?;
    let Token::Text(closing) = &closing_token.token else {
        return None;
    };
    if !closing.starts_with('}') {
        return None;
    }
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
        assert_eq!(compose_accent('?', 'e'), None);
        assert_eq!(compose_accent('\'', 'x'), None);
    }

    #[test]
    fn transparent_commands_skip_only_hyphen_and_slash() {
        assert!(is_transparent_command("-"));
        assert!(is_transparent_command("/"));
        assert!(!is_transparent_command("x"));
        assert!(!is_transparent_command("'"));
    }

    #[test]
    fn letter_form_accents_are_the_six_letter_commands() {
        for name in ["c", "v", "u", "H", "r", "k"] {
            assert!(is_letter_form_accent(name), "{name} is letter-form");
        }
        for name in ["'", "`", "^", "\"", "~", "=", ".", "-", "x"] {
            assert!(!is_letter_form_accent(name), "{name} is not letter-form");
        }
    }

    #[test]
    fn accent_to_char_maps_every_accent_command() {
        for (name, expected) in [
            ("'", '\''),
            ("`", '`'),
            ("^", '^'),
            ("\"", '"'),
            ("~", '~'),
            ("=", '='),
            (".", '.'),
            ("c", 'c'),
            ("v", 'v'),
            ("u", 'u'),
            ("H", 'H'),
            ("r", 'r'),
            ("k", 'k'),
        ] {
            assert_eq!(accent_to_char(name), Some(expected), "{name}");
        }
        assert_eq!(accent_to_char("x"), None);
    }

    #[test]
    fn acute_composed_covers_every_arm() {
        for (base, expected) in [
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
        ] {
            assert_eq!(acute_composed(base), Some(expected), "{base}");
        }
        assert_eq!(acute_composed('x'), None);
    }

    #[test]
    fn grave_composed_covers_every_arm() {
        for (base, expected) in [
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
        ] {
            assert_eq!(grave_composed(base), Some(expected), "{base}");
        }
        assert_eq!(grave_composed('x'), None);
    }

    #[test]
    fn circumflex_composed_covers_every_arm() {
        for (base, expected) in [
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
        ] {
            assert_eq!(circumflex_composed(base), Some(expected), "{base}");
        }
        assert_eq!(circumflex_composed('x'), None);
    }

    #[test]
    fn umlaut_composed_covers_every_arm() {
        for (base, expected) in [
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
        ] {
            assert_eq!(umlaut_composed(base), Some(expected), "{base}");
        }
        assert_eq!(umlaut_composed('x'), None);
    }

    #[test]
    fn tilde_composed_covers_every_arm() {
        for (base, expected) in [
            ('a', 'ã'),
            ('n', 'ñ'),
            ('o', 'õ'),
            ('A', 'Ã'),
            ('N', 'Ñ'),
            ('O', 'Õ'),
        ] {
            assert_eq!(tilde_composed(base), Some(expected), "{base}");
        }
        assert_eq!(tilde_composed('x'), None);
    }

    #[test]
    fn macron_composed_covers_every_arm() {
        for (base, expected) in [
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
        ] {
            assert_eq!(macron_composed(base), Some(expected), "{base}");
        }
        assert_eq!(macron_composed('x'), None);
    }

    #[test]
    fn dot_composed_covers_every_arm() {
        for (base, expected) in [
            ('a', 'ȧ'),
            ('e', 'ė'),
            ('o', 'ȯ'),
            ('A', 'Ȧ'),
            ('E', 'Ė'),
            ('O', 'Ȯ'),
        ] {
            assert_eq!(dot_composed(base), Some(expected), "{base}");
        }
        assert_eq!(dot_composed('x'), None);
    }

    #[test]
    fn cedilla_composed_covers_every_arm() {
        for (base, expected) in [
            ('c', 'ç'),
            ('C', 'Ç'),
            ('s', 'ş'),
            ('S', 'Ş'),
            ('t', 'ţ'),
            ('T', 'Ţ'),
        ] {
            assert_eq!(cedilla_composed(base), Some(expected), "{base}");
        }
        assert_eq!(cedilla_composed('x'), None);
    }

    #[test]
    fn caron_composed_covers_every_arm() {
        for (base, expected) in [
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
        ] {
            assert_eq!(caron_composed(base), Some(expected), "{base}");
        }
        assert_eq!(caron_composed('x'), None);
    }

    #[test]
    fn breve_composed_covers_every_arm() {
        for (base, expected) in [
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
        ] {
            assert_eq!(breve_composed(base), Some(expected), "{base}");
        }
        assert_eq!(breve_composed('x'), None);
    }

    #[test]
    fn double_acute_composed_covers_every_arm() {
        for (base, expected) in [('o', 'ő'), ('O', 'Ő'), ('u', 'ű'), ('U', 'Ű')] {
            assert_eq!(double_acute_composed(base), Some(expected), "{base}");
        }
        assert_eq!(double_acute_composed('x'), None);
    }

    #[test]
    fn ring_composed_covers_every_arm() {
        for (base, expected) in [('a', 'å'), ('A', 'Å'), ('u', 'ů'), ('U', 'Ů')] {
            assert_eq!(ring_composed(base), Some(expected), "{base}");
        }
        assert_eq!(ring_composed('x'), None);
    }

    #[test]
    fn ogonek_composed_covers_every_arm() {
        for (base, expected) in [('a', 'ą'), ('A', 'Ą'), ('e', 'ę'), ('E', 'Ę')] {
            assert_eq!(ogonek_composed(base), Some(expected), "{base}");
        }
        assert_eq!(ogonek_composed('x'), None);
    }

    #[test]
    fn extract_base_reads_a_braced_letter() {
        assert_eq!(extract_base_from_text_start("{a}"), Some(('a', 3)));
        assert_eq!(extract_base_from_text_start("  {b} rest"), Some(('b', 5)));
    }

    #[test]
    fn extract_base_reads_a_direct_letter() {
        assert_eq!(extract_base_from_text_start("e"), Some(('e', 1)));
        assert_eq!(extract_base_from_text_start("  x rest"), Some(('x', 3)));
    }

    #[test]
    fn extract_base_rejects_non_letters_and_bad_braces() {
        assert_eq!(extract_base_from_text_start("123"), None);
        assert_eq!(extract_base_from_text_start("{ab}"), None);
        assert_eq!(extract_base_from_text_start(""), None);
        assert_eq!(extract_base_from_text_start("   "), None);
    }

    /// A non-braced tail must not be misread as a braced base: `xa}` starts
    /// with `x`, not `{`, so the base is `x` even though the tail is long
    /// enough to look braced.
    #[test]
    fn extract_base_prefers_the_direct_letter_over_a_later_brace() {
        assert_eq!(extract_base_from_text_start("xa}"), Some(('x', 1)));
    }

    fn dotless_tokens(inner: Token, closing: &str) -> Vec<SpannedToken> {
        vec![
            SpannedToken {
                token: Token::Text("{".to_string()),
                start: 0,
                end: 1,
            },
            SpannedToken {
                token: inner,
                start: 1,
                end: 2,
            },
            SpannedToken {
                token: Token::Text(closing.to_string()),
                start: 2,
                end: 3,
            },
        ]
    }

    fn dotless_i() -> Token {
        Token::Command {
            name: "i".to_string(),
            args: Vec::new(),
        }
    }

    #[test]
    fn dotless_i_with_bare_closing_bracket_skips_nothing_further() {
        let tokens = dotless_tokens(dotless_i(), "}");
        let (composed, source) = resolve_dotless_ij('\'', &tokens, 1).expect("dotless i");
        assert_eq!(composed, 'í');
        match source {
            AccentBaseSource::FromDotlessIJ {
                extra_tokens_to_skip,
                chars_to_skip_in_last,
            } => {
                assert_eq!(extra_tokens_to_skip, 3);
                assert_eq!(chars_to_skip_in_last, 0);
            }
            _ => panic!("expected FromDotlessIJ"),
        }
    }

    #[test]
    fn dotless_i_with_trailing_text_skips_one_char() {
        let tokens = dotless_tokens(dotless_i(), "}ndice");
        let (composed, source) = resolve_dotless_ij('\'', &tokens, 1).expect("dotless i");
        assert_eq!(composed, 'í');
        match source {
            AccentBaseSource::FromDotlessIJ {
                chars_to_skip_in_last,
                ..
            } => assert_eq!(chars_to_skip_in_last, 1),
            _ => panic!("expected FromDotlessIJ"),
        }
    }

    /// `\i` and `\j` each stand for their own letter; no other command is
    /// dotless (including a look-alike that merely starts the same way).
    #[test]
    fn dotless_base_covers_i_and_j_only() {
        assert_eq!(dotless_base("i"), Some('i'));
        assert_eq!(dotless_base("j"), Some('j'));
        for name in ["x", "I", "J", "ii", "", "dotless_i"] {
            assert_eq!(dotless_base(name), None, "{name} is not dotless");
        }
    }

    #[test]
    fn dotless_j_has_no_composition_and_resolves_to_none() {
        let tokens = dotless_tokens(
            Token::Command {
                name: "j".to_string(),
                args: Vec::new(),
            },
            "}",
        );
        assert!(
            resolve_dotless_ij('~', &tokens, 1).is_none(),
            "no tilde-j composition exists, so j must not resolve"
        );
    }

    #[test]
    fn dotless_rejects_non_ij_commands_args_and_open_tails() {
        let other = dotless_tokens(
            Token::Command {
                name: "x".to_string(),
                args: Vec::new(),
            },
            "}",
        );
        assert_eq!(resolve_dotless_ij('\'', &other, 1).map(|r| r.0), None);
        let with_args = dotless_tokens(
            Token::Command {
                name: "i".to_string(),
                args: vec!["oops".to_string()],
            },
            "}",
        );
        assert_eq!(resolve_dotless_ij('\'', &with_args, 1).map(|r| r.0), None);
        let open_tail = dotless_tokens(dotless_i(), "ndice");
        assert_eq!(resolve_dotless_ij('\'', &open_tail, 1).map(|r| r.0), None);
        let not_a_command = dotless_tokens(Token::Text("i".to_string()), "}");
        assert_eq!(
            resolve_dotless_ij('\'', &not_a_command, 1).map(|r| r.0),
            None
        );
    }
}
