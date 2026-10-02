//! Spell checking: language and dictionary resolution, accent-aware text
//! extraction, whitelist handling, and the `lint_files` orchestrator the
//! linter calls.

mod accents;
mod dictionary;
mod language;
mod text;

#[cfg(test)]
pub(crate) mod test_support;

use std::collections::HashMap;
use std::path::Path;

use anyhow::Result;

use super::{LintFinding, Severity};
use crate::texparse::tokenize_with_spans;

use dictionary::{ensure_dictionary, load_dictionary, load_project_whitelist, DictionaryLocation};
pub use dictionary::{
    global_whitelist_path, installed_dictionaries, parse_whitelist_words, InstalledDictionary,
    PROJECT_WHITELIST_FILES,
};
pub(crate) use language::document_language;
use language::{
    expected_dictionary_hint, language_disagreement_message, resolve_language, skip_message,
    using_message,
};
use text::{build_spell_text, line_for_offset};

/// Lint files for spelling mistakes. Returns warnings (never errors).
/// If a dictionary cannot be obtained, returns Ok(vec![]) after printing a
/// clear message (per spec: don't fail the build for missing dictionaries).
pub fn lint_files(
    files: &[(String, String)],
    root: &Path,
    default_lang: Option<&str>,
) -> Result<Vec<LintFinding>> {
    // Determine language: the document's own babel/polyglossia declaration
    // wins over the configured default, which wins over the `english`
    // fallback. See `resolve_language` for the rationale.
    let resolution = resolve_language(files, default_lang);
    let lang = resolution.language;

    // The document's declaration silently overriding the user's global
    // default would just replace one confusing behaviour with another: warn,
    // naming both languages, whenever they disagree. Fires at most once per
    // run and never when either is absent or they agree.
    let mut findings = Vec::new();
    if let (Some(configured), Some((declared, file, line))) =
        (default_lang, resolution.declared.as_ref())
    {
        if declared != configured {
            findings.push(LintFinding {
                file: file.clone(),
                line: *line,
                severity: Severity::Warning,
                message: language_disagreement_message(configured, declared),
                suggestion: None,
            });
        }
    }

    // Ensure a dictionary exists and load it. Never fall back to a dictionary
    // for a different language: a missing dictionary means spell-check is
    // skipped for this run, not silently degraded.
    let dict_loc = match ensure_dictionary(&lang) {
        Ok(loc) => loc,
        Err(e) => {
            eprintln!(
                "{}",
                skip_message(
                    &lang,
                    expected_dictionary_hint(&lang).as_deref(),
                    &e.to_string()
                )
            );
            return Ok(findings);
        }
    };

    let dict_hint = match &dict_loc {
        DictionaryLocation::Wordlist(path) => path.clone(),
        DictionaryLocation::Hunspell { dic, .. } => dic.clone(),
    };
    let dict = match load_dictionary(&dict_loc) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("{}", skip_message(&lang, Some(&dict_hint), &e.to_string()));
            return Ok(findings);
        }
    };

    eprintln!("{}", using_message(&lang, &dict_loc));

    let whitelist = load_project_whitelist(root);

    // Map unknown word -> first (file, line) occurrence
    let mut unknowns: HashMap<String, (String, usize)> = HashMap::new();

    for (rel, source) in files {
        let tokenized = tokenize_with_spans(source);
        let (spell_text, line_chunks) = build_spell_text(&tokenized.tokens, source);
        let spell_base = spell_text.as_ptr() as usize;
        for word in spell_text.split(|c: char| !c.is_alphabetic()) {
            let w = word.trim();
            if w.is_empty() {
                continue;
            }
            let wl = w.to_lowercase();
            if wl.len() <= 1 {
                continue;
            }
            if !dict.contains(&wl) && !whitelist.contains(&wl) {
                let word_offset = word.as_ptr() as usize - spell_base;
                let line = line_for_offset(&line_chunks, word_offset);
                unknowns.entry(wl).or_insert_with(|| (rel.clone(), line));
            }
        }
    }

    // Spell findings are emitted in a total order — (file, line, word) — so
    // two runs over the same project print byte-identical output regardless
    // of the HashMap's iteration order. Only spell findings are ordered; the
    // language-disagreement warning pushed above keeps its position.
    let mut spell_findings: Vec<(String, usize, String)> = unknowns
        .into_iter()
        .map(|(word, (file, line))| (file, line, word))
        .collect();
    spell_findings.sort();

    for (file, line, word) in spell_findings {
        findings.push(LintFinding {
            file,
            line,
            severity: Severity::Warning,
            message: format!("Unknown word: '{}'", word),
            suggestion: Some(
                "Add to your personal dictionary with `texforge spell add <word>` \
                 (add --local instead to accept it only in this project) to accept this word"
                    .into(),
            ),
        });
    }

    Ok(findings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    use super::test_support::{hunspell_fixture_paths, run_with_home};
    use crate::test_sync::ENV_LOCK;

    #[test]
    fn tokenizer_integration_does_not_flag_commands_or_labels() {
        let _lock = ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = TempDir::new().unwrap();
        let dicts_dir = home.path().join(".texforge").join("dicts");
        fs::create_dir_all(&dicts_dir).unwrap();
        fs::write(
            dicts_dir.join("english.txt"),
            "hello\nworld\nthis\nis\nsome\ntext\nmore\n",
        )
        .unwrap();

        let orig_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", home.path());

        let src = r#"\documentclass{article}
\begin{document}
Hello world. This is some text. \label{sec:intro} More text.
\end{document}"#;

        // Run lint_files against a single file
        let files = vec![("main.tex".to_string(), src.to_string())];
        let project_root = TempDir::new().unwrap();
        let findings = lint_files(&files, project_root.path(), Some("english")).unwrap();

        match orig_home {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }

        // Should be empty: the words used are in the tiny dictionary and commands/labels not emitted
        assert!(
            findings.is_empty(),
            "Expected no findings, got: {:?}",
            findings
        );
    }

    /// Reproduces the reported defect end-to-end: a Spanish document, no
    /// configured default language, and only an English dictionary present.
    /// Must emit ZERO `Unknown word` findings — not 214 false positives from
    /// checking Spanish prose against English words.
    #[test]
    fn spanish_document_with_only_english_dictionary_emits_no_unknown_word_warnings() {
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
        // Force the offline bail deterministically (see
        // ensure_dictionary_bails_in_test_harness_environment) instead of
        // depending on real network access being unavailable.
        std::env::set_var("NEXTEST_RUN_ID", "te6-spanish-only-english-dict");

        let src = "\\usepackage[spanish]{babel}\n\\begin{document}\n\
                   mentoría ahí universidad liderazgo soluciones\n\\end{document}";
        let files = vec![("main.tex".to_string(), src.to_string())];
        let project_root = TempDir::new().unwrap();
        let findings = lint_files(&files, project_root.path(), None);

        std::env::remove_var("NEXTEST_RUN_ID");
        match orig_home {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }

        let findings = findings.unwrap();
        assert!(
            findings.is_empty(),
            "expected zero findings when the resolved language's dictionary is missing, got: {:?}",
            findings
        );
    }

    /// When a dictionary for the resolved language IS present, spell-check
    /// must run against THAT dictionary, never a different language's —
    /// proven by a Spanish word passing and an English-only word failing.
    #[test]
    fn spanish_document_checks_against_spanish_dictionary_not_english() {
        let _lock = ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = TempDir::new().unwrap();
        let dicts_dir = home.path().join(".texforge").join("dicts");
        fs::create_dir_all(&dicts_dir).unwrap();
        fs::write(dicts_dir.join("spanish.txt"), "mentoría\nahí\n").unwrap();
        fs::write(dicts_dir.join("english.txt"), "hello\nworld\n").unwrap();

        let orig_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", home.path());

        let src = "\\usepackage[spanish]{babel}\n\\begin{document}\n\
                   mentoría hello\n\\end{document}";
        let files = vec![("main.tex".to_string(), src.to_string())];
        let project_root = TempDir::new().unwrap();
        let findings = lint_files(&files, project_root.path(), None);

        match orig_home {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }

        let findings = findings.unwrap();
        assert!(
            !findings.iter().any(|f| f.message.contains("mentoría")),
            "'mentoría' is in the spanish dictionary and must not be flagged: {:?}",
            findings
        );
        assert!(
            findings.iter().any(|f| f.message.contains("hello")),
            "'hello' is English-only and must be flagged when checking against spanish, \
             proving no fallback to the english dictionary occurred: {:?}",
            findings
        );
    }

    /// End-to-end: `lint_files` on a Spanish document with a Hunspell pair
    /// installed checks against it (not a fallback wordlist), accepting both
    /// a bare stem and an affix-generated form while still flagging a
    /// genuine misspelling (requirement 10).
    #[test]
    fn spanish_document_checks_against_installed_hunspell_pair() {
        let _lock = ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = TempDir::new().unwrap();
        let dicts_dir = home.path().join(".texforge").join("dicts");
        fs::create_dir_all(&dicts_dir).unwrap();
        let (fixture_dic, fixture_aff) = hunspell_fixture_paths();
        fs::copy(&fixture_dic, dicts_dir.join("spanish.dic")).unwrap();
        fs::copy(&fixture_aff, dicts_dir.join("spanish.aff")).unwrap();

        let orig_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", home.path());

        let src = "\\usepackage[spanish]{babel}\n\\begin{document}\n\
                   sol soles perro xilofonoinventado\n\\end{document}";
        let files = vec![("main.tex".to_string(), src.to_string())];
        let project_root = TempDir::new().unwrap();
        let findings = lint_files(&files, project_root.path(), None);

        match orig_home {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }

        let findings = findings.unwrap();
        assert!(
            !findings.iter().any(|f| f.message.contains("'sol'")),
            "stem 'sol' must be accepted: {:?}",
            findings
        );
        assert!(
            !findings.iter().any(|f| f.message.contains("'soles'")),
            "affix-generated form 'soles' must be accepted: {:?}",
            findings
        );
        assert!(
            findings
                .iter()
                .any(|f| f.message.contains("xilofonoinventado")),
            "a genuine misspelling must still be flagged: {:?}",
            findings
        );
    }

    /// The reported line is the source line of the first occurrence: the
    /// byte offset of the word inside the extracted spell text must be
    /// measured from the start of that text, or findings drift to the
    /// wrong line of the document.
    #[test]
    fn unknown_word_is_reported_on_its_own_source_line() {
        let _lock = ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = TempDir::new().unwrap();
        let dicts_dir = home.path().join(".texforge").join("dicts");
        fs::create_dir_all(&dicts_dir).unwrap();
        fs::write(
            dicts_dir.join("english.txt"),
            "hello\nworld\nmore\nhere\nand\nwords\n",
        )
        .unwrap();

        let orig_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", home.path());

        // Four prose runs on three different lines: the unknown word sits
        // in the middle run, so neither the first nor the last chunk's line
        // can masquerade as its own.
        let src = "\\begin{document}\nhello world\n\\emph{zzzznotaword}\n\\textbf{more here}\nand more words\n\\end{document}";
        let files = vec![("main.tex".to_string(), src.to_string())];
        let project_root = TempDir::new().unwrap();
        let findings = lint_files(&files, project_root.path(), Some("english")).unwrap();

        match orig_home {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }

        assert_eq!(findings.len(), 1, "{findings:?}");
        assert_eq!(findings[0].message, "Unknown word: 'zzzznotaword'");
        assert_eq!(
            findings[0].line, 3,
            "the unknown word sits on source line 3"
        );
    }

    /// The `Unknown word` finding's suggestion must point at the new command,
    /// not at hand-editing files, and must now name `--local` rather than
    /// `--global` since global became the default (requirement 9).
    #[test]
    fn unknown_word_suggestion_names_spell_add_and_local_flag() {
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

        let src = "\\begin{document}\nHello zzzznotaword world\n\\end{document}";
        let files = vec![("main.tex".to_string(), src.to_string())];
        let project_root = TempDir::new().unwrap();
        let findings = lint_files(&files, project_root.path(), Some("english"));

        match orig_home {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }

        let findings = findings.unwrap();
        assert_eq!(findings.len(), 1);
        let suggestion = findings[0].suggestion.as_deref().unwrap_or("");
        assert!(
            suggestion.contains("texforge spell add"),
            "suggestion must name the new command: {}",
            suggestion
        );
        assert!(
            suggestion.contains("--local"),
            "suggestion must mention --local, since global is now the default: {}",
            suggestion
        );
        assert!(
            !suggestion.contains("--global"),
            "suggestion should not point at --global now that it is the default: {}",
            suggestion
        );
    }

    /// Five unknown words across two files: two runs must give identical,
    /// `(file, line, word)`-sorted vectors — the fix for nondeterministic
    /// `HashMap` iteration order.
    #[test]
    fn spell_findings_are_identical_and_sorted_across_runs() {
        run_with_home("", "known\n", || {
            let files = vec![
                ("a.tex".to_string(), "alpha\nbravo charlie".to_string()),
                ("b.tex".to_string(), "delta\necho".to_string()),
            ];
            let root = TempDir::new().unwrap();

            // LintFinding only derives Debug, so compare the projection that
            // matches the printed output. The constant "Unknown word: '"
            // prefix makes message order equal to word order.
            let project = |findings: &[LintFinding]| -> Vec<(String, usize, String)> {
                findings
                    .iter()
                    .map(|f| (f.file.clone(), f.line, f.message.clone()))
                    .collect()
            };

            let first = lint_files(&files, root.path(), Some("english")).unwrap();
            let second = lint_files(&files, root.path(), Some("english")).unwrap();
            assert_eq!(first.len(), 5, "{first:?}");

            let first_projection = project(&first);
            assert_eq!(
                first_projection,
                project(&second),
                "two runs over the same project must be byte-identical"
            );

            let mut sorted = first_projection.clone();
            sorted.sort();
            assert_eq!(
                first_projection, sorted,
                "spell findings must be sorted by (file, line, word)"
            );
        });
    }
}
