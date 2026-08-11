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
        anyhow::bail!(
            "no {} dictionary is available from the configured source",
            lang
        )
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
    let bytes = match download_with_tools(url, "curl", "wget") {
        Ok(bytes) => bytes,
        Err(failure) => anyhow::bail!(failure.describe(lang, url)),
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

/// Outcome of running a single download tool (curl or wget).
enum ToolOutcome {
    Success(Vec<u8>),
    /// The binary does not exist in PATH (spawn failed with `NotFound`).
    NotFound,
    /// The binary exists and ran, but did not produce the dictionary.
    Failed(ToolFailure),
}

/// Detail of a download tool that ran but failed, kept separate from "tool
/// not installed" so the two situations never collapse into one message.
struct ToolFailure {
    tool: &'static str,
    detail: String,
}

/// Reason a download could not be completed by any available tool.
enum DownloadFailure {
    /// Neither tool exists in PATH.
    NoToolFound,
    /// A tool ran but the transfer itself failed (bad exit status, HTTP
    /// error, network error, ...). Carries that tool's own error output so
    /// it can be reported verbatim instead of behind a generic message.
    ToolError(ToolFailure),
}

impl DownloadFailure {
    /// `lang` and `url` are folded in here (rather than left to the caller)
    /// so every branch names the specific language and source attempted,
    /// even though the caller also wraps this in its own "could not obtain
    /// dictionary for '{lang}'" context.
    fn describe(&self, lang: &str, url: &str) -> String {
        match self {
            DownloadFailure::NoToolFound => {
                "neither 'curl' nor 'wget' was found in PATH".to_string()
            }
            DownloadFailure::ToolError(f) => {
                let mut msg = format!("{} exited fetching {}: {}", f.tool, url, f.detail);
                if f.detail.contains("404") {
                    msg.push_str(&format!(
                        "; this looks like a missing resource (HTTP 404) — the source configured \
                         for '{}' may not have this dictionary, see remote_for_language()",
                        lang
                    ));
                }
                msg
            }
        }
    }
}

/// Last non-empty lines of tool output, trimmed, for embedding in an error
/// message without dumping an entire progress bar or stack trace.
fn tail_lines(text: &str, max_lines: usize) -> String {
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    let start = lines.len().saturating_sub(max_lines);
    lines[start..].join(" | ")
}

/// Run one download tool binary and classify what happened. Kept separate
/// from `download_with_tools` so both curl and wget go through identical
/// classification logic.
fn run_tool(bin: &str, args: &[&str], tool_label: &'static str) -> ToolOutcome {
    match std::process::Command::new(bin).args(args).output() {
        Ok(output) if output.status.success() => ToolOutcome::Success(output.stdout),
        Ok(output) => {
            let tail = tail_lines(&String::from_utf8_lossy(&output.stderr), 5);
            let detail = if tail.is_empty() {
                format!("exited with {}", output.status)
            } else {
                format!("exited with {}: {}", output.status, tail)
            };
            ToolOutcome::Failed(ToolFailure {
                tool: tool_label,
                detail,
            })
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => ToolOutcome::NotFound,
        Err(e) => ToolOutcome::Failed(ToolFailure {
            tool: tool_label,
            detail: format!("could not be run: {}", e),
        }),
    }
}

/// Try `curl_bin` then `wget_bin` to fetch `url`. Binary names are
/// parameterized (rather than hardcoded to "curl"/"wget") so tests can
/// exercise the "no download tool" and "tool ran but failed" branches
/// deterministically — e.g. a nonexistent binary name for "not found", or
/// `false`, which always exits 1, for "ran but failed" — without depending
/// on what happens to be installed on the machine running the tests.
fn download_with_tools(
    url: &str,
    curl_bin: &str,
    wget_bin: &str,
) -> std::result::Result<Vec<u8>, DownloadFailure> {
    let curl = match run_tool(curl_bin, &["-fsSL", url], "curl") {
        ToolOutcome::Success(bytes) => return Ok(bytes),
        other => other,
    };
    let wget = match run_tool(wget_bin, &["--no-verbose", "-O", "-", url], "wget") {
        ToolOutcome::Success(bytes) => return Ok(bytes),
        other => other,
    };

    match (curl, wget) {
        (ToolOutcome::Failed(f), _) => Err(DownloadFailure::ToolError(f)),
        (ToolOutcome::NotFound, ToolOutcome::Failed(f)) => Err(DownloadFailure::ToolError(f)),
        (ToolOutcome::NotFound, ToolOutcome::NotFound) => Err(DownloadFailure::NoToolFound),
        _ => unreachable!("success cases already returned above"),
    }
}

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
struct LanguageResolution {
    /// The language spell-check will use.
    language: String,
    /// `(language, file, line)` of the `\usepackage[...]{babel}` (or
    /// `polyglossia`) declaration found in the preamble, if any.
    declared: Option<(String, String, usize)>,
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
fn resolve_language(files: &[(String, String)], default_lang: Option<&str>) -> LanguageResolution {
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
fn language_disagreement_message(configured: &str, declared: &str) -> String {
    format!(
        "Configured default language is '{}', but this document declares '{}' via babel/polyglossia; using '{}'",
        configured, declared, declared
    )
}

/// Single-line message printed when spell-check must be skipped because the
/// dictionary for the resolved language is unavailable. Names the language,
/// the dictionary path that was expected, and why it could not be obtained —
/// so a wrong-language (or no-language) run is never silent.
fn skip_message(lang: &str, expected_path: Option<&Path>, reason: &str) -> String {
    let expected = expected_path
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "<unknown: could not determine home directory>".to_string());
    format!(
        "Spell-check skipped: resolved language '{}', but its dictionary ({}) is unavailable: {}",
        lang, expected, reason
    )
}

/// Single-line message printed when spell-check runs, naming the language
/// and dictionary file actually used.
fn using_message(lang: &str, dict_path: &Path) -> String {
    format!(
        "Spell-check: checking '{}' prose against {}",
        lang,
        dict_path.display()
    )
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

/// List installed dictionaries as `(language, path)` pairs, sorted by language.
///
/// Reports only what is actually present under `dir` — verified state, not
/// the set of languages texforge merely knows how to fetch remotely.
fn installed_dictionaries_in(dir: &Path) -> Vec<(String, PathBuf)> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut dicts: Vec<(String, PathBuf)> = entries
        .filter_map(Result::ok)
        .filter_map(|e| {
            let path = e.path();
            if path.extension().and_then(|s| s.to_str()) != Some("txt") {
                return None;
            }
            let lang = path.file_stem()?.to_str()?.to_string();
            Some((lang, path))
        })
        .collect();
    dicts.sort_by(|a, b| a.0.cmp(&b.0));
    dicts
}

/// List dictionaries installed under the managed `~/.texforge/dicts` directory.
pub fn installed_dictionaries() -> Vec<(String, PathBuf)> {
    match dictionaries_dir() {
        Some(dir) => installed_dictionaries_in(&dir),
        None => Vec::new(),
    }
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

    // Ensure dictionary exists and load it. Never fall back to a dictionary
    // for a different language: a missing dictionary means spell-check is
    // skipped for this run, not silently degraded.
    let dict_path = match ensure_dictionary(&lang) {
        Ok(p) => p,
        Err(e) => {
            eprintln!(
                "{}",
                skip_message(&lang, dictionary_path_for(&lang).as_deref(), &e.to_string())
            );
            return Ok(findings);
        }
    };

    let dict = match load_dictionary(&dict_path) {
        Ok(d) => d,
        Err(e) => {
            eprintln!(
                "{}",
                skip_message(&lang, Some(dict_path.as_path()), &e.to_string())
            );
            return Ok(findings);
        }
    };

    eprintln!("{}", using_message(&lang, &dict_path));

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
    fn installed_dictionaries_in_empty_dir_returns_empty() {
        let tmp = TempDir::new().unwrap();
        let dicts = installed_dictionaries_in(tmp.path());
        assert!(dicts.is_empty());
    }

    #[test]
    fn installed_dictionaries_in_missing_dir_returns_empty() {
        let tmp = TempDir::new().unwrap();
        let missing = tmp.path().join("does-not-exist");
        let dicts = installed_dictionaries_in(&missing);
        assert!(dicts.is_empty());
    }

    #[test]
    fn installed_dictionaries_in_lists_txt_files_sorted() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("spanish.txt"), "hola\nmundo\n").unwrap();
        fs::write(tmp.path().join("english.txt"), "hello\nworld\n").unwrap();
        fs::write(tmp.path().join("notes.md"), "ignored").unwrap();
        let dicts = installed_dictionaries_in(tmp.path());
        let langs: Vec<&str> = dicts.iter().map(|(lang, _)| lang.as_str()).collect();
        assert_eq!(langs, vec!["english", "spanish"]);
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

    /// Neither binary exists in PATH: must report that plainly, naming both
    /// tools, and must NOT claim a transfer failure that never happened.
    #[test]
    fn download_with_tools_reports_missing_tools_by_name() {
        let bogus_a = "definitely-not-a-real-binary-abc123";
        let bogus_b = "definitely-not-a-real-binary-xyz789";
        let err = download_with_tools("https://example.invalid/dict.txt", bogus_a, bogus_b)
            .expect_err("expected failure when neither tool exists");
        assert!(matches!(err, DownloadFailure::NoToolFound));
        let msg = err.describe("spanish", "https://example.invalid/dict.txt");
        assert!(msg.contains("curl"), "message should name curl: {}", msg);
        assert!(msg.contains("wget"), "message should name wget: {}", msg);
    }

    /// A tool that exists and runs but fails must have ITS failure reported
    /// (exit status), not the generic "no download tool" message — that
    /// message is reserved for the tool genuinely being absent.
    #[cfg(unix)]
    #[test]
    fn download_with_tools_reports_real_exit_status_when_tool_runs_but_fails() {
        // `false` always exists on Unix and always exits 1 without touching
        // the network, so this is deterministic and offline.
        let bogus_wget = "definitely-not-a-real-binary-xyz789";
        let err = download_with_tools("https://example.invalid/dict.txt", "false", bogus_wget)
            .expect_err("expected failure when curl exits non-zero");
        match &err {
            DownloadFailure::ToolError(f) => assert_eq!(f.tool, "curl"),
            DownloadFailure::NoToolFound => {
                panic!("curl exists and ran; must not report NoToolFound")
            }
        }
        let msg = err.describe("spanish", "https://example.invalid/dict.txt");
        assert!(msg.contains("curl"), "message should name curl: {}", msg);
        assert!(
            msg.contains("exit"),
            "message should carry the tool's exit status: {}",
            msg
        );
    }

    /// A 404 from a tool that ran must be called out explicitly as a
    /// missing-resource problem, not folded into a generic error.
    #[cfg(unix)]
    #[test]
    fn download_with_tools_flags_http_404_as_missing_source() {
        use std::io::Write as _;
        use std::os::unix::fs::PermissionsExt;

        let tmp = TempDir::new().unwrap();
        let fake_curl = tmp.path().join("curl");
        {
            let mut f = fs::File::create(&fake_curl).unwrap();
            writeln!(f, "#!/bin/sh").unwrap();
            writeln!(
                f,
                "echo 'curl: (22) The requested URL returned error: 404' 1>&2"
            )
            .unwrap();
            writeln!(f, "exit 22").unwrap();
        }
        let mut perms = fs::metadata(&fake_curl).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&fake_curl, perms).unwrap();

        let bogus_wget = "definitely-not-a-real-binary-xyz789";
        let err = download_with_tools(
            "https://example.invalid/spanish.txt",
            fake_curl.to_str().unwrap(),
            bogus_wget,
        )
        .expect_err("expected failure on HTTP 404");
        let msg = err.describe("spanish", "https://example.invalid/spanish.txt");
        assert!(
            msg.contains("404"),
            "message should surface the 404 the tool reported: {}",
            msg
        );
        assert!(
            msg.to_lowercase().contains("source") || msg.to_lowercase().contains("exist"),
            "message should call out that the dictionary may not exist at the source: {}",
            msg
        );
    }

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
    fn remote_for_language_has_no_spanish_source() {
        assert_eq!(remote_for_language("spanish"), None);
        assert_eq!(remote_for_language("es"), None);
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
    fn using_message_is_one_line_and_names_language_and_path() {
        let msg = using_message(
            "spanish",
            Path::new("/home/user/.texforge/dicts/spanish.txt"),
        );
        assert_eq!(msg.lines().count(), 1, "must be exactly one line: {}", msg);
        assert!(msg.contains("spanish"));
        assert!(msg.contains("/home/user/.texforge/dicts/spanish.txt"));
    }

    /// Reproduces the reported defect end-to-end: a Spanish document, no
    /// configured default language, and only an English dictionary present.
    /// Must emit ZERO `Unknown word` findings — not 214 false positives from
    /// checking Spanish prose against English words.
    #[test]
    fn spanish_document_with_only_english_dictionary_emits_no_unknown_word_warnings() {
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

    // --- TE10: a global default must not silently override the document's
    // own language declaration ---

    /// The disagreement warning must be a `Warning`, name both languages, and
    /// point at the line of the `\usepackage` that declared the language
    /// which won. Also proves the skip path still runs (zero `Unknown word`
    /// findings) so the disagreement warning is the only finding produced.
    #[test]
    fn disagreement_warning_names_both_languages_and_points_at_declaration() {
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
}
