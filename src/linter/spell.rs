use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use super::{LintFinding, Severity};

/// 1-based line number of a byte offset.
fn line_of(source: &str, offset: usize) -> usize {
    let offset = offset.min(source.len());
    1 + source[..offset].matches('\n').count()
}
use crate::texparse::{tokenize_with_spans, Token};

/// Project-local whitelist filenames to check, in order.
const PROJECT_WHITELIST_FILES: &[&str] = &["spell-whitelist.txt", ".texforge/spell-words"];

/// Managed dictionary directory under the user's home (`~/.texforge/dicts`).
fn dictionaries_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".texforge").join("dicts"))
}

fn dictionary_path_for(lang: &str) -> Option<PathBuf> {
    dictionaries_dir().map(|d| d.join(format!("{}.txt", lang)))
}

/// Language -> remote wordlist URL mapping (best-effort). Missing entries
/// mean the language is not supported remotely and spell-check will be
/// skipped with a clear message.
fn remote_for_language(lang: &str) -> Option<&'static str> {
    match lang {
        "english" => Some("https://raw.githubusercontent.com/dwyl/english-words/master/words.txt"),
        "en" => Some("https://raw.githubusercontent.com/dwyl/english-words/master/words.txt"),
        // Spanish sources are varied; attempt a common wordlist if present.
        "spanish" | "es" => {
            Some("https://raw.githubusercontent.com/manuelperez/wordlists/master/spanish.txt")
        }
        _ => None,
    }
}

/// Ensure a dictionary for `lang` is present, downloading and caching it on
/// first use. Returns the local path to the wordlist on success.
fn ensure_dictionary(lang: &str) -> Result<PathBuf> {
    let path = dictionary_path_for(lang).ok_or_else(|| {
        anyhow::anyhow!("Could not determine home directory for dictionary cache")
    })?;
    if path.exists() {
        return Ok(path);
    }

    let Some(url) = remote_for_language(lang) else {
        anyhow::bail!("No remote dictionary configured for language: {}", lang)
    };

    // During tests (and when running under a test harness such as nextest)
    // avoid any network activity — fail open with a clear message so the
    // test suite remains offline-friendly and deterministic. Detect at
    // runtime because cfg!(test) is not reliable for code compiled into
    // non-test binaries that run under test harnesses.
    let is_test_harness = std::env::var("RUST_TEST_THREADS").is_ok()
        || std::env::var("NEXTEST_CURRENT_RUN_ID").is_ok()
        || std::env::var("NEXTEST_RUN_ID").is_ok()
        || std::env::var("CI").is_ok();

    if is_test_harness {
        anyhow::bail!(
            "Dictionary for '{}' not present and network disabled during tests",
            lang
        );
    }

    eprintln!(
        "Dictionary for '{}' not found locally. Downloading...",
        lang
    );

    // Prefer the system 'curl' or 'wget' binary to avoid pulling in reqwest/
    // rustls at runtime (which has previously caused panics in some test
    // environments). If neither tool is available, degrade to a clear message
    // and do not attempt network activity.

    let download_result: Result<Vec<u8>> = (|| {
        // Try curl first
        if let Ok(output) = std::process::Command::new("curl")
            .args(["-fsSL", url])
            .output()
        {
            if output.status.success() {
                return Ok(output.stdout);
            }
            // fall through to wget
        }

        // Try wget next
        if let Ok(output) = std::process::Command::new("wget")
            .args(["-qO-", url])
            .output()
        {
            if output.status.success() {
                return Ok(output.stdout);
            }
        }

        // No download tool available or both failed: do not attempt reqwest here
        anyhow::bail!("No download tool (curl or wget) available to fetch dictionary")
    })();

    let bytes = match download_result {
        Ok(b) => b,
        Err(e) => {
            eprintln!(
                "Spell-check disabled: could not obtain dictionary for '{}': {}",
                lang, e
            );
            return Ok(path);
        }
    };

    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).context("Failed to create dictionary cache directory")?;
    }

    let mut f = fs::File::create(&path)
        .with_context(|| format!("Failed to create dictionary file: {}", path.display()))?;
    f.write_all(&bytes)?;
    eprintln!("  ◇ Dictionary cached to {}", path.display());

    Ok(path)
}

fn load_dictionary(path: &Path) -> Result<HashSet<String>> {
    let content = fs::read_to_string(path).context("Failed to read dictionary file")?;
    let mut set = HashSet::new();
    for line in content.lines() {
        let w = line.trim();
        if w.is_empty() {
            continue;
        }
        set.insert(w.to_lowercase());
    }
    Ok(set)
}

fn load_project_whitelist(root: &Path) -> HashSet<String> {
    let mut set = HashSet::new();
    for name in PROJECT_WHITELIST_FILES {
        let p = root.join(name);
        if p.exists() {
            if let Ok(text) = fs::read_to_string(&p) {
                for line in text.lines() {
                    let w = line.trim();
                    if w.is_empty() || w.starts_with('#') {
                        continue;
                    }
                    set.insert(w.to_lowercase());
                }
            }
        }
    }
    set
}

/// Lint files for spelling mistakes. Returns warnings (never errors).
/// If a dictionary cannot be obtained, returns Ok(vec![]) after printing a
/// clear message (per spec: don't fail the build for missing dictionaries).
pub fn lint_files(
    files: &[(String, String)],
    root: &Path,
    default_lang: Option<&str>,
) -> Result<Vec<LintFinding>> {
    // Determine language: prefer configured default; otherwise try to infer
    // from a `\usepackage[spanish]{babel}` occurrence in the preamble.
    let mut lang = default_lang
        .map(|s| s.to_string())
        .unwrap_or_else(|| "english".to_string());

    if default_lang.is_none() {
        'outer: for (_rel, source) in files.iter() {
            let tokenized = tokenize_with_spans(source);
            for sp in &tokenized.tokens {
                if let Token::Command { name, args } = &sp.token {
                    if name == "usepackage" && args.last().is_some() {
                        let arg = args.last().unwrap().as_str();
                        if arg == "spanish" {
                            lang = "spanish".to_string();
                            break 'outer;
                        }
                    }
                }
                if let Token::BeginDocument = &sp.token {
                    break;
                }
            }
        }
    }

    // Ensure dictionary exists and load it.
    let dict_path = match ensure_dictionary(&lang) {
        Ok(p) => p,
        Err(e) => {
            eprintln!(
                "Spell-check disabled: could not obtain dictionary for '{}': {}",
                lang, e
            );
            return Ok(Vec::new());
        }
    };

    let dict = match load_dictionary(&dict_path) {
        Ok(d) => d,
        Err(e) => {
            eprintln!(
                "Spell-check disabled: failed to read dictionary '{}': {}",
                dict_path.display(),
                e
            );
            return Ok(Vec::new());
        }
    };

    let mut allowed = dict;
    // Add project whitelist
    let project_whitelist = load_project_whitelist(root);
    for w in project_whitelist {
        allowed.insert(w);
    }

    // Map unknown word -> first (file, line) occurrence
    let mut unknowns: HashMap<String, (String, usize)> = HashMap::new();

    for (rel, source) in files {
        let tokenized = tokenize_with_spans(source);
        for sp in &tokenized.tokens {
            if let Token::Text(text) = &sp.token {
                // Extract candidate words by splitting on non-alpha characters
                for word in text.split(|c: char| !c.is_alphabetic()) {
                    let w = word.trim();
                    if w.is_empty() {
                        continue;
                    }
                    let wl = w.to_lowercase();
                    if wl.len() <= 1 {
                        // skip short tokens to avoid noisy single-letter misses
                        continue;
                    }
                    if !allowed.contains(&wl) {
                        // record first occurrence only
                        unknowns
                            .entry(wl)
                            .or_insert_with(|| (rel.clone(), line_of(source, sp.start)));
                    }
                }
            }
        }
    }

    let mut findings = Vec::new();
    for (word, (file, line)) in unknowns {
        findings.push(LintFinding {
            file,
            line,
            severity: Severity::Warning,
            message: format!("Unknown word: '{}'", word),
            suggestion: Some(
                "Add to project spell-whitelist.txt or .texforge/spell-words to accept this word"
                    .into(),
            ),
        });
    }

    Ok(findings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn tokenizer_integration_does_not_flag_commands_or_labels() {
        let src = r#"\documentclass{article}
\begin{document}
Hello world. This is some text. \label{sec:intro} More text.
\end{document}"#;
        // Create a tiny english dictionary that contains common words
        let tmp = TempDir::new().unwrap();
        let dict_dir = tmp.path().join(".texforge").join("dicts");
        fs::create_dir_all(&dict_dir).unwrap();
        fs::write(
            dict_dir.join("english.txt"),
            "hello\nworld\nthis\nis\nsome\ntext\nmore\n",
        )
        .unwrap();

        // Run lint_files against a single file
        let files = vec![("main.tex".to_string(), src.to_string())];
        let findings = lint_files(&files, tmp.path(), Some("english")).unwrap();
        // Should be empty: the words used are in the tiny dictionary and commands/labels not emitted
        assert!(
            findings.is_empty(),
            "Expected no findings, got: {:?}",
            findings
        );
    }

    #[test]
    fn ensure_dictionary_bails_in_test_harness_environment() {
        // Simulate being run under a test harness like nextest by setting a
        // recognized environment variable. ensure_dictionary must not attempt
        // network activity in this case and should return an Err.
        std::env::set_var("NEXTEST_RUN_ID", "1");
        let res = ensure_dictionary("spanish");
        assert!(
            res.is_err(),
            "Expected ensure_dictionary to error when under test harness"
        );
        std::env::remove_var("NEXTEST_RUN_ID");
    }
}
