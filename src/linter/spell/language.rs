//! The document's own language (babel/polyglossia) resolved against the
//! configured default, and the one-line messages reported around it.

use std::path::{Path, PathBuf};

use super::dictionary::{
    dictionary_dic_path_for, dictionary_path_for, remote_for_language, DictionaryLocation,
    RemoteSource,
};
use super::text::line_of;
use crate::texparse::{tokenize_with_spans, Token};

/// Map a babel/polyglossia language option (e.g. `spanish`, `es`, `main=spanish`)
/// to the canonical language name used for dictionary filenames.
fn normalize_babel_option(opt: &str) -> Option<&'static str> {
    // `main=spanish` / `variant=es-MX` style keyed options: use the value.
    let value = opt.rsplit('=').next().unwrap_or(opt).trim();
    match value {
        "spanish" | "es" | "spanish-mexico" | "es-MX" => Some("spanish"),
        "english" | "en" | "USenglish" | "UKenglish" => Some("english"),
        "french" | "fr" | "francais" => Some("french"),
        _ => None,
    }
}

/// If `args` is a `\usepackage` invocation loading `babel` or `polyglossia`,
/// return the language it declares (e.g. `\usepackage[spanish]{babel}` -> `Some("spanish")`).
///
/// The tokenizer appends bracket and brace groups to `args` in the order they
/// appear, so for `[spanish]{babel}` the package name (`babel`) is `args.last()`
/// and the language option (`spanish`) is an *earlier* element — not the last one.
fn babel_language_from_usepackage(args: &[String]) -> Option<&'static str> {
    let package = args.last()?;
    if package != "babel" && package != "polyglossia" {
        return None;
    }
    for opt_group in &args[..args.len() - 1] {
        for opt in opt_group.split(',') {
            if let Some(lang) = normalize_babel_option(opt.trim()) {
                return Some(lang);
            }
        }
    }
    None
}

/// Outcome of `resolve_language`: the language spell-check will actually use,
/// plus the document's own `babel`/`polyglossia` declaration (if any) and
/// where it was found. Kept separate from finding-construction so
/// `resolve_language` stays free of `LintFinding` concerns; callers decide
/// whether the declaration and the configured default disagree.
pub(super) struct LanguageResolution {
    /// The language spell-check will use.
    pub(super) language: String,
    /// `(language, file, line)` of the `\usepackage[...]{babel}` (or
    /// `polyglossia`) declaration found in the preamble, if any.
    pub(super) declared: Option<(String, String, usize)>,
}

/// Scan a single file's preamble for a `babel`/`polyglossia` language
/// declaration, stopping at `\begin{document}` so the body is never
/// tokenized for this. Returns the language and the 1-based line of the
/// `\usepackage` that declared it.
fn find_babel_declaration(source: &str) -> Option<(&'static str, usize)> {
    let tokenized = tokenize_with_spans(source);
    for sp in &tokenized.tokens {
        match &sp.token {
            Token::Command { name, args } if name == "usepackage" => {
                if let Some(lang) = babel_language_from_usepackage(args) {
                    return Some((lang, line_of(source, sp.start)));
                }
            }
            Token::BeginDocument => break,
            _ => {}
        }
    }
    None
}

/// Resolve the language to spell-check against. Highest priority first:
/// (1) a `babel`/`polyglossia` language declared in the document's own
/// preamble — a declaration inside the file is evidence about *this*
/// document, while a global default is only a guess; (2) the user-configured
/// default; (3) `english`. The declaration (if any) is reported alongside the
/// resolved language so the caller can warn when it disagrees with the
/// configured default rather than silently overriding it.
pub(super) fn resolve_language(
    files: &[(String, String)],
    default_lang: Option<&str>,
) -> LanguageResolution {
    let declared = files.iter().find_map(|(rel, source)| {
        find_babel_declaration(source).map(|(lang, line)| (lang.to_string(), rel.clone(), line))
    });

    let language = match &declared {
        Some((lang, _, _)) => lang.clone(),
        None => default_lang
            .map(str::to_string)
            .unwrap_or_else(|| "english".to_string()),
    };

    LanguageResolution { language, declared }
}

/// Message for the `Severity::Warning` finding emitted when the document's
/// own declaration overrides a configured default that names a different
/// language. Names both languages explicitly and states which one won, so
/// the override is never silent.
pub(super) fn language_disagreement_message(configured: &str, declared: &str) -> String {
    format!(
        "Configured default language is '{}', but this document declares '{}' via babel/polyglossia; using '{}'",
        configured, declared, declared
    )
}

/// Best-effort path to name in the skip message before it's known which
/// backend (if any) would have served `lang` — the `.dic` half of a
/// Hunspell pair, or the wordlist path otherwise.
pub(super) fn expected_dictionary_hint(lang: &str) -> Option<PathBuf> {
    match remote_for_language(lang) {
        Some(RemoteSource::Hunspell { .. }) => dictionary_dic_path_for(lang),
        _ => dictionary_path_for(lang),
    }
}

/// Single-line message printed when spell-check must be skipped because the
/// dictionary for the resolved language is unavailable. Names the language,
/// the dictionary path that was expected, and why it could not be obtained —
/// so a wrong-language (or no-language) run is never silent.
pub(super) fn skip_message(lang: &str, expected_path: Option<&Path>, reason: &str) -> String {
    let expected = expected_path
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "<unknown: could not determine home directory>".to_string());
    format!(
        "Spell-check skipped: resolved language '{}', but its dictionary ({}) is unavailable: {}",
        lang, expected, reason
    )
}

/// Single-line message printed when spell-check runs, naming the language
/// and what it is being checked against, for either backend.
pub(super) fn using_message(lang: &str, loc: &DictionaryLocation) -> String {
    match loc {
        DictionaryLocation::Wordlist(path) => format!(
            "Spell-check: checking '{}' prose against {}",
            lang,
            path.display()
        ),
        DictionaryLocation::Hunspell { dic, aff } => format!(
            "Spell-check: checking '{}' prose against Hunspell dictionary {} + {}",
            lang,
            dic.display(),
            aff.display()
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    use super::super::{lint_files, Severity};
    use crate::test_sync::ENV_LOCK;

    // --- TE6: language resolution must not silently default to English ---

    /// `\usepackage[spanish]{babel}` tokenizes with the package name
    /// (`babel`) last and the language option (`spanish`) *before* it — the
    /// bug was checking `args.last()` for the language, which is always the
    /// package name, so this never matched.
    #[test]
    fn babel_language_reads_the_option_not_the_package_name() {
        let args = vec!["spanish".to_string(), "babel".to_string()];
        assert_eq!(babel_language_from_usepackage(&args), Some("spanish"));
    }

    #[test]
    fn babel_language_ignores_unrelated_packages() {
        let args = vec!["amsmath".to_string()];
        assert_eq!(babel_language_from_usepackage(&args), None);

        let args = vec!["utf8".to_string(), "inputenc".to_string()];
        assert_eq!(babel_language_from_usepackage(&args), None);
    }

    #[test]
    fn babel_language_handles_babel_with_no_option() {
        let args = vec!["babel".to_string()];
        assert_eq!(babel_language_from_usepackage(&args), None);
    }

    #[test]
    fn babel_language_handles_keyed_options_and_polyglossia() {
        let args = vec!["main=spanish".to_string(), "polyglossia".to_string()];
        assert_eq!(babel_language_from_usepackage(&args), Some("spanish"));
    }

    #[test]
    fn normalize_babel_option_maps_every_english_spelling() {
        assert_eq!(normalize_babel_option("english"), Some("english"));
        assert_eq!(normalize_babel_option("en"), Some("english"));
        assert_eq!(normalize_babel_option("USenglish"), Some("english"));
        assert_eq!(normalize_babel_option("UKenglish"), Some("english"));
    }

    #[test]
    fn resolve_language_infers_spanish_from_babel_preamble() {
        let src = "\\usepackage[spanish]{babel}\n\\begin{document}\nHola\n\\end{document}";
        let files = vec![("main.tex".to_string(), src.to_string())];
        assert_eq!(resolve_language(&files, None).language, "spanish");
    }

    #[test]
    fn resolve_language_defaults_to_english_without_babel() {
        let src = "\\documentclass{article}\n\\begin{document}\nHello\n\\end{document}";
        let files = vec![("main.tex".to_string(), src.to_string())];
        assert_eq!(resolve_language(&files, None).language, "english");
    }

    /// TE10 regression: the document's own declaration must win over a
    /// configured default that names a different language.
    #[test]
    fn resolve_language_prefers_document_declaration_over_configured_default() {
        let src = "\\usepackage[spanish]{babel}\n\\begin{document}\nHola\n\\end{document}";
        let files = vec![("main.tex".to_string(), src.to_string())];
        assert_eq!(
            resolve_language(&files, Some("english")).language,
            "spanish"
        );
    }

    /// The precedence rule is not spanish-specific: any declared language
    /// overrides the configured default.
    #[test]
    fn resolve_language_prefers_document_declaration_for_other_languages_too() {
        let src = "\\usepackage[french]{babel}\n\\begin{document}\nSalut\n\\end{document}";
        let files = vec![("main.tex".to_string(), src.to_string())];
        assert_eq!(resolve_language(&files, Some("english")).language, "french");
    }

    #[test]
    fn resolve_language_uses_configured_default_without_babel() {
        let src = "\\documentclass{article}\n\\begin{document}\nHello\n\\end{document}";
        let files = vec![("main.tex".to_string(), src.to_string())];
        assert_eq!(
            resolve_language(&files, Some("english")).language,
            "english"
        );
    }

    #[test]
    fn skip_message_is_one_line_and_names_language_path_and_reason() {
        let msg = skip_message(
            "spanish",
            Some(Path::new("/home/user/.texforge/dicts/spanish.txt")),
            "network disabled during tests",
        );
        assert_eq!(msg.lines().count(), 1, "must be exactly one line: {}", msg);
        assert!(msg.contains("spanish"), "must name the language: {}", msg);
        assert!(
            msg.contains("/home/user/.texforge/dicts/spanish.txt"),
            "must name the expected dictionary path: {}",
            msg
        );
        assert!(
            msg.contains("network disabled during tests"),
            "must carry the reason: {}",
            msg
        );
    }

    #[test]
    fn using_message_is_one_line_and_names_language_and_path_for_wordlist() {
        let msg = using_message(
            "spanish",
            &DictionaryLocation::Wordlist(PathBuf::from("/home/user/.texforge/dicts/spanish.txt")),
        );
        assert_eq!(msg.lines().count(), 1, "must be exactly one line: {}", msg);
        assert!(msg.contains("spanish"));
        assert!(msg.contains("/home/user/.texforge/dicts/spanish.txt"));
    }

    /// Requirement 7: `using_message` names the language and what it is
    /// checking against for the Hunspell backend too — both files, not just
    /// one, since the pair together is what "checking against" means here.
    #[test]
    fn using_message_names_both_files_for_hunspell() {
        let msg = using_message(
            "spanish",
            &DictionaryLocation::Hunspell {
                dic: PathBuf::from("/home/user/.texforge/dicts/spanish.dic"),
                aff: PathBuf::from("/home/user/.texforge/dicts/spanish.aff"),
            },
        );
        assert_eq!(msg.lines().count(), 1, "must be exactly one line: {}", msg);
        assert!(msg.contains("spanish"));
        assert!(msg.contains("/home/user/.texforge/dicts/spanish.dic"));
        assert!(msg.contains("/home/user/.texforge/dicts/spanish.aff"));
    }

    // --- TE10: a global default must not silently override the document's
    // own language declaration ---

    /// The disagreement warning must be a `Warning`, name both languages, and
    /// point at the line of the `\usepackage` that declared the language
    /// which won. Also proves the skip path still runs (zero `Unknown word`
    /// findings) so the disagreement warning is the only finding produced.
    #[test]
    fn disagreement_warning_names_both_languages_and_points_at_declaration() {
        let _lock = ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = TempDir::new().unwrap();
        fs::create_dir_all(home.path().join(".texforge").join("dicts")).unwrap();
        fs::write(
            home.path()
                .join(".texforge")
                .join("dicts")
                .join("english.txt"),
            "hello\nworld\n",
        )
        .unwrap();

        let orig_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", home.path());
        std::env::set_var("NEXTEST_RUN_ID", "te10-disagreement-warning");

        let src = "\\usepackage[spanish]{babel}\n\\begin{document}\nHola\n\\end{document}";
        let files = vec![("main.tex".to_string(), src.to_string())];
        let project_root = TempDir::new().unwrap();
        let findings = lint_files(&files, project_root.path(), Some("english"));

        std::env::remove_var("NEXTEST_RUN_ID");
        match orig_home {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }

        let findings = findings.unwrap();
        assert_eq!(
            findings.len(),
            1,
            "expected exactly one finding (the disagreement warning; spell-check itself is \
             skipped since no spanish dictionary is obtainable): {:?}",
            findings
        );
        let warning = &findings[0];
        assert!(matches!(warning.severity, Severity::Warning));
        assert!(
            warning.message.contains("spanish") && warning.message.contains("english"),
            "message must name both languages: {}",
            warning.message
        );
        assert_eq!(warning.file, "main.tex");
        assert_eq!(warning.line, 1, "must point at the \\usepackage line");
    }

    #[test]
    fn no_disagreement_warning_when_declared_matches_configured_default() {
        let _lock = ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = TempDir::new().unwrap();
        fs::create_dir_all(home.path().join(".texforge").join("dicts")).unwrap();

        let orig_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", home.path());
        std::env::set_var("NEXTEST_RUN_ID", "te10-no-disagreement-same-lang");

        let src = "\\usepackage[spanish]{babel}\n\\begin{document}\nHola\n\\end{document}";
        let files = vec![("main.tex".to_string(), src.to_string())];
        let project_root = TempDir::new().unwrap();
        let findings = lint_files(&files, project_root.path(), Some("spanish"));

        std::env::remove_var("NEXTEST_RUN_ID");
        match orig_home {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }

        let findings = findings.unwrap();
        assert!(
            findings.is_empty(),
            "declared and configured language agree; expected no findings: {:?}",
            findings
        );
    }

    #[test]
    fn no_disagreement_warning_without_babel_declaration() {
        let _lock = ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = TempDir::new().unwrap();
        fs::create_dir_all(home.path().join(".texforge").join("dicts")).unwrap();
        fs::write(
            home.path()
                .join(".texforge")
                .join("dicts")
                .join("english.txt"),
            "hello\nworld\n",
        )
        .unwrap();

        let orig_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", home.path());

        let src = "\\documentclass{article}\n\\begin{document}\nhello world\n\\end{document}";
        let files = vec![("main.tex".to_string(), src.to_string())];
        let project_root = TempDir::new().unwrap();
        let findings = lint_files(&files, project_root.path(), Some("english"));

        match orig_home {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }

        let findings = findings.unwrap();
        assert!(
            findings.is_empty(),
            "no babel declaration means no disagreement is possible: {:?}",
            findings
        );
    }

    /// A multi-file project where two files declare the same language must
    /// produce exactly one warning, not one per file.
    #[test]
    fn multi_file_project_with_matching_declarations_produces_one_warning() {
        let _lock = ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = TempDir::new().unwrap();
        fs::create_dir_all(home.path().join(".texforge").join("dicts")).unwrap();
        fs::write(
            home.path()
                .join(".texforge")
                .join("dicts")
                .join("english.txt"),
            "hello\nworld\n",
        )
        .unwrap();

        let orig_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", home.path());
        std::env::set_var("NEXTEST_RUN_ID", "te10-multi-file-one-warning");

        let src_a = "\\usepackage[spanish]{babel}\n\\begin{document}\nHola\n\\end{document}";
        let src_b = "\\usepackage[spanish]{babel}\n\\begin{document}\nAdios\n\\end{document}";
        let files = vec![
            ("a.tex".to_string(), src_a.to_string()),
            ("b.tex".to_string(), src_b.to_string()),
        ];
        let project_root = TempDir::new().unwrap();
        let findings = lint_files(&files, project_root.path(), Some("english"));

        std::env::remove_var("NEXTEST_RUN_ID");
        match orig_home {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }

        let findings = findings.unwrap();
        let warnings: Vec<_> = findings
            .iter()
            .filter(|f| f.message.contains("Configured default language"))
            .collect();
        assert_eq!(
            warnings.len(),
            1,
            "two files declaring the same language must produce one warning, not two: {:?}",
            findings
        );
    }

    // --- the skip message's "expected path" must name the right backend ---

    /// Spanish is backed by a Hunspell pair, so the hint names the `.dic`;
    /// English keeps the plain wordlist, so the hint names the `.txt`.
    #[test]
    fn expected_dictionary_hint_names_the_dic_for_hunspell_and_the_txt_for_wordlists() {
        let _lock = ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = TempDir::new().unwrap();
        let orig_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", home.path());

        let file_name_of = |lang: &str| {
            expected_dictionary_hint(lang).map(|p| p.file_name().unwrap().to_os_string())
        };
        let spanish = file_name_of("spanish");
        let english = file_name_of("english");

        match orig_home {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }

        assert_eq!(
            spanish,
            Some(std::ffi::OsString::from("spanish.dic")),
            "Hunspell-backed languages must hint at the .dic half of the pair"
        );
        assert_eq!(
            english,
            Some(std::ffi::OsString::from("english.txt")),
            "wordlist languages must hint at the .txt wordlist"
        );
    }

    /// A group that merely looks like `\usepackage[...]{babel}` — any other
    /// command name — is not a declaration: the guard on the command name is
    /// what keeps arbitrary `[spanish]{babel}` groups from steering the
    /// document language.
    #[test]
    fn only_the_usepackage_command_declares_a_babel_language() {
        let src = "\\mycmd[spanish]{babel}\n\\begin{document}\nHola\n\\end{document}";
        let files = vec![("main.tex".to_string(), src.to_string())];
        assert_eq!(resolve_language(&files, None).language, "english");
    }

    /// The scan stops at `\begin{document}`: a `\usepackage` written in the
    /// body is past the preamble and must not steer language resolution.
    #[test]
    fn babel_declaration_after_begin_document_is_ignored() {
        let src = "\\begin{document}\nHola\n\\usepackage[spanish]{babel}\n\\end{document}";
        let files = vec![("main.tex".to_string(), src.to_string())];
        assert_eq!(resolve_language(&files, None).language, "english");
    }
}
