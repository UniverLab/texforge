//! Dictionary discovery: where a language's dictionary lives, how it is
//! installed, and the project/global whitelist files that override it.

use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// Project-local whitelist filenames to check, in order. Also the order
/// `texforge spell add` picks a target file in: the first that already
/// exists wins (decision 4).
pub const PROJECT_WHITELIST_FILES: &[&str] = &["spell-whitelist.txt", ".texforge/spell-words"];

/// The personal dictionary shared by every project: `~/.texforge/spell-words`.
/// `None` when the home directory cannot be determined.
pub fn global_whitelist_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".texforge").join("spell-words"))
}

/// Parse one whitelist file's contents into a lowercase word set. Blank
/// lines and `#`-prefixed comment lines are skipped. Comparison elsewhere is
/// case-insensitive because entries are lowercased here on load.
pub fn parse_whitelist_words(content: &str) -> HashSet<String> {
    let mut set = HashSet::new();
    for line in content.lines() {
        let w = line.trim();
        if w.is_empty() || w.starts_with('#') {
            continue;
        }
        set.insert(w.to_lowercase());
    }
    set
}

/// Managed dictionary directory under the user's home (`~/.texforge/dicts`).
fn dictionaries_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".texforge").join("dicts"))
}

pub(super) fn dictionary_path_for(lang: &str) -> Option<PathBuf> {
    dictionaries_dir().map(|d| d.join(format!("{}.txt", lang)))
}

pub(super) fn dictionary_dic_path_for(lang: &str) -> Option<PathBuf> {
    dictionaries_dir().map(|d| d.join(format!("{}.dic", lang)))
}

fn dictionary_aff_path_for(lang: &str) -> Option<PathBuf> {
    dictionaries_dir().map(|d| d.join(format!("{}.aff", lang)))
}

/// Where a language's dictionary source lives remotely: a single wordlist
/// file, or a Hunspell `.dic` + `.aff` pair. English keeps the wordlist it
/// has always used; Spanish gets a Hunspell pair because a plain wordlist
/// cannot represent Hunspell's affix-generated forms (see spec rationale).
pub(super) enum RemoteSource {
    Wordlist(&'static str),
    Hunspell {
        dic_url: &'static str,
        aff_url: &'static str,
    },
}

/// Language -> remote dictionary source mapping (best-effort). Missing
/// entries mean the language is not supported remotely and spell-check will
/// be skipped with a clear message.
pub(super) fn remote_for_language(lang: &str) -> Option<RemoteSource> {
    match lang {
        "english" | "en" => Some(RemoteSource::Wordlist(
            "https://raw.githubusercontent.com/dwyl/english-words/master/words.txt",
        )),
        "spanish" | "es" => Some(RemoteSource::Hunspell {
            dic_url: "https://raw.githubusercontent.com/wooorm/dictionaries/main/dictionaries/es/index.dic",
            aff_url: "https://raw.githubusercontent.com/wooorm/dictionaries/main/dictionaries/es/index.aff",
        }),
        _ => None,
    }
}

/// Where a language's dictionary lives on disk once `ensure_dictionary` has
/// confirmed or fetched it. Two shapes because a plain wordlist is one file
/// and a Hunspell dictionary is a `.dic`/`.aff` pair.
pub(super) enum DictionaryLocation {
    Wordlist(PathBuf),
    Hunspell { dic: PathBuf, aff: PathBuf },
}

/// Whether both halves of a Hunspell `.dic`/`.aff` pair are on disk and
/// usable. Both files must exist: a lone `.dic` (or lone `.aff`) is not a
/// dictionary and falls through to the wordlist / download paths below.
fn hunspell_pair_present(
    dic_path: Option<&PathBuf>,
    aff_path: Option<&PathBuf>,
) -> Option<(PathBuf, PathBuf)> {
    let (dic, aff) = (dic_path?, aff_path?);
    if dic.exists() && aff.exists() {
        Some((dic.clone(), aff.clone()))
    } else {
        None
    }
}

/// Whether this process runs under a test harness (or CI). During tests —
/// and when running under harnesses such as nextest — avoid any network
/// activity: callers fail open with a clear message so the suite stays
/// offline-friendly and deterministic. Detected at runtime because
/// `cfg!(test)` is not reliable for code compiled into non-test binaries
/// that run under test harnesses.
fn is_test_harness() -> bool {
    std::env::var("RUST_TEST_THREADS").is_ok()
        || std::env::var("NEXTEST_CURRENT_RUN_ID").is_ok()
        || std::env::var("NEXTEST_RUN_ID").is_ok()
        || std::env::var("CI").is_ok()
}

/// Ensure a dictionary for `lang` is present, downloading and caching it on
/// first use. Returns its on-disk location on success.
pub(super) fn ensure_dictionary(lang: &str) -> Result<DictionaryLocation> {
    let dic_path = dictionary_dic_path_for(lang);
    let aff_path = dictionary_aff_path_for(lang);
    let txt_path = dictionary_path_for(lang);

    // The Hunspell pair wins when both backends are already present on disk
    // for this language — it is the better checker.
    if let Some((dic, aff)) = hunspell_pair_present(dic_path.as_ref(), aff_path.as_ref()) {
        return Ok(DictionaryLocation::Hunspell { dic, aff });
    }

    if let Some(txt) = txt_path.as_ref() {
        if txt.exists() {
            return Ok(DictionaryLocation::Wordlist(txt.clone()));
        }
    }

    let dicts_dir = dictionaries_dir().ok_or_else(|| {
        anyhow::anyhow!("Could not determine home directory for dictionary cache")
    })?;

    let Some(source) = remote_for_language(lang) else {
        anyhow::bail!(
            "no {} dictionary is available from the configured source",
            lang
        )
    };

    // During tests (and when running under a test harness such as nextest)
    // avoid any network activity — fail open with a clear message so the
    // test suite remains offline-friendly and deterministic. Detect at
    // runtime because cfg!(test) is not reliable for code compiled into
    // non-test binaries that run under test harnesses (see
    // [`is_test_harness`]).
    if is_test_harness() {
        anyhow::bail!(
            "Dictionary for '{}' not present and network disabled during tests",
            lang
        );
    }

    fs::create_dir_all(&dicts_dir).context("Failed to create dictionary cache directory")?;

    // Prefer the system 'curl' or 'wget' binary to avoid pulling in reqwest/
    // rustls at runtime (which has previously caused panics in some test
    // environments). If neither tool is available, degrade to a clear message
    // and do not attempt network activity.
    match source {
        RemoteSource::Wordlist(url) => {
            eprintln!(
                "Dictionary for '{}' not found locally. Downloading...",
                lang
            );
            let bytes = match download_with_tools(url, "curl", "wget") {
                Ok(bytes) => bytes,
                Err(failure) => anyhow::bail!(failure.describe(lang, url)),
            };

            let path = txt_path.expect("dictionaries_dir() succeeded above");
            let mut f = fs::File::create(&path)
                .with_context(|| format!("Failed to create dictionary file: {}", path.display()))?;
            f.write_all(&bytes)?;
            eprintln!("  ◇ Dictionary cached to {}", path.display());

            Ok(DictionaryLocation::Wordlist(path))
        }
        RemoteSource::Hunspell { dic_url, aff_url } => {
            eprintln!(
                "Dictionary for '{}' not found locally. Downloading Hunspell dictionary...",
                lang
            );

            // Fetch both files before writing anything to their final
            // location: a partially downloaded language must never be left
            // on disk (a stray `<lang>.dic` with no `.aff`, or vice versa,
            // would make every later run fail with nothing obviously wrong).
            let dic_bytes = match download_with_tools(dic_url, "curl", "wget") {
                Ok(bytes) => bytes,
                Err(failure) => anyhow::bail!(failure.describe(lang, dic_url)),
            };
            let aff_bytes = match download_with_tools(aff_url, "curl", "wget") {
                Ok(bytes) => bytes,
                Err(failure) => anyhow::bail!(failure.describe(lang, aff_url)),
            };

            let dic_final = dic_path.expect("dictionaries_dir() succeeded above");
            let aff_final = aff_path.expect("dictionaries_dir() succeeded above");
            let dic_tmp = dicts_dir.join(format!("{}.dic.part", lang));
            let aff_tmp = dicts_dir.join(format!("{}.aff.part", lang));

            fs::write(&dic_tmp, &dic_bytes)
                .with_context(|| format!("Failed to write {}", dic_tmp.display()))?;
            fs::write(&aff_tmp, &aff_bytes)
                .with_context(|| format!("Failed to write {}", aff_tmp.display()))?;

            fs::rename(&dic_tmp, &dic_final)
                .with_context(|| format!("Failed to install {}", dic_final.display()))?;
            if let Err(e) = fs::rename(&aff_tmp, &aff_final) {
                // Undo the first half of the move rather than leave a `.dic`
                // with no matching `.aff` on disk.
                let _ = fs::remove_file(&dic_final);
                return Err(e)
                    .with_context(|| format!("Failed to install {}", aff_final.display()));
            }

            eprintln!(
                "  ◇ Dictionary cached to {} and {}",
                dic_final.display(),
                aff_final.display()
            );

            Ok(DictionaryLocation::Hunspell {
                dic: dic_final,
                aff: aff_final,
            })
        }
    }
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

/// A dictionary asked exactly one question by every caller: "is this word
/// known?" Two backends answer it — a plain wordlist and a Hunspell
/// `.dic`/`.aff` pair — and nothing upstream learns which one did.
pub(super) enum WordDictionary {
    Wordlist(HashSet<String>),
    Hunspell(Box<spellbook::Dictionary>),
}

impl WordDictionary {
    pub(super) fn contains(&self, word: &str) -> bool {
        match self {
            WordDictionary::Wordlist(set) => set.contains(word),
            WordDictionary::Hunspell(dict) => dict.check(word),
        }
    }
}

pub(super) fn load_dictionary(loc: &DictionaryLocation) -> Result<WordDictionary> {
    match loc {
        DictionaryLocation::Wordlist(path) => {
            let content = fs::read_to_string(path).context("Failed to read dictionary file")?;
            let mut set = HashSet::new();
            for line in content.lines() {
                let w = line.trim();
                if w.is_empty() {
                    continue;
                }
                set.insert(w.to_lowercase());
            }
            Ok(WordDictionary::Wordlist(set))
        }
        DictionaryLocation::Hunspell { dic, aff } => {
            let aff_content = fs::read_to_string(aff).with_context(|| {
                format!("Failed to read Hunspell affix file: {}", aff.display())
            })?;
            let dic_content = fs::read_to_string(dic).with_context(|| {
                format!("Failed to read Hunspell dictionary file: {}", dic.display())
            })?;
            // A parse error here means a corrupt or truncated download, not
            // a bug in the caller — surface it through the existing skip
            // path with the language named, never as a panic.
            let dict = spellbook::Dictionary::new(&aff_content, &dic_content).map_err(|e| {
                anyhow::anyhow!(
                    "Failed to parse Hunspell dictionary ({}, {}): {}",
                    dic.display(),
                    aff.display(),
                    e
                )
            })?;
            Ok(WordDictionary::Hunspell(Box::new(dict)))
        }
    }
}

/// A dictionary found under the managed `~/.texforge/dicts` directory,
/// naming which backend it is so callers (e.g. `texforge doctor`) can report
/// on it without assuming a single-file wordlist.
pub enum InstalledDictionary {
    Wordlist {
        lang: String,
        path: PathBuf,
    },
    Hunspell {
        lang: String,
        dic_path: PathBuf,
        aff_path: PathBuf,
    },
}

impl InstalledDictionary {
    pub fn lang(&self) -> &str {
        match self {
            InstalledDictionary::Wordlist { lang, .. } => lang,
            InstalledDictionary::Hunspell { lang, .. } => lang,
        }
    }
}

/// List installed dictionaries, sorted by language.
///
/// Reports only what is actually present under `dir` — verified state, not
/// the set of languages texforge merely knows how to fetch remotely. A lone
/// `.dic` with no matching `.aff` is not usable and is not reported.
fn installed_dictionaries_in(dir: &Path) -> Vec<InstalledDictionary> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };

    let mut dicts: Vec<InstalledDictionary> = Vec::new();
    let mut hunspell_langs: Vec<String> = Vec::new();

    for path in entries.filter_map(Result::ok).map(|e| e.path()) {
        match path.extension().and_then(|s| s.to_str()) {
            Some("txt") => {
                if let Some(lang) = path.file_stem().and_then(|s| s.to_str()) {
                    dicts.push(InstalledDictionary::Wordlist {
                        lang: lang.to_string(),
                        path: path.clone(),
                    });
                }
            }
            Some("dic") => {
                if let Some(lang) = path.file_stem().and_then(|s| s.to_str()) {
                    hunspell_langs.push(lang.to_string());
                }
            }
            _ => {}
        }
    }

    for lang in hunspell_langs {
        let aff_path = dir.join(format!("{}.aff", lang));
        if aff_path.exists() {
            let dic_path = dir.join(format!("{}.dic", lang));
            dicts.push(InstalledDictionary::Hunspell {
                lang,
                dic_path,
                aff_path,
            });
        }
    }

    dicts.sort_by(|a, b| a.lang().cmp(b.lang()));
    dicts
}

/// List dictionaries installed under the managed `~/.texforge/dicts` directory.
pub fn installed_dictionaries() -> Vec<InstalledDictionary> {
    match dictionaries_dir() {
        Some(dir) => installed_dictionaries_in(&dir),
        None => Vec::new(),
    }
}

/// Words accepted for this project: the union of whichever
/// `PROJECT_WHITELIST_FILES` exist under `root`, plus the user's global
/// personal dictionary (`global_whitelist_path`). Neither scope shadows the
/// other — a word accepted anywhere is accepted (decision 3). Reading either
/// source is best-effort: a missing or unreadable file (including a global
/// dictionary that was never created, or a home directory that can't be
/// determined) is the normal case, not an error.
pub(super) fn load_project_whitelist(root: &Path) -> HashSet<String> {
    let mut set = HashSet::new();
    for name in PROJECT_WHITELIST_FILES {
        if let Ok(text) = fs::read_to_string(root.join(name)) {
            set.extend(parse_whitelist_words(&text));
        }
    }
    if let Some(global) = global_whitelist_path() {
        if let Ok(text) = fs::read_to_string(&global) {
            set.extend(parse_whitelist_words(&text));
        }
    }
    set
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    use super::super::lint_files;
    use super::super::test_support::{hunspell_fixture_paths, ENV_MUTEX};

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
        let langs: Vec<&str> = dicts.iter().map(InstalledDictionary::lang).collect();
        assert_eq!(langs, vec!["english", "spanish"]);
    }

    /// A Hunspell pair (both `.dic` and `.aff` present) is reported as an
    /// installed dictionary in its own right (requirement 8) — `texforge
    /// doctor` must not go blind to a language once its Hunspell pair lands.
    #[test]
    fn installed_dictionaries_in_reports_hunspell_pair() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("spanish.dic"), "1\nsol/S\n").unwrap();
        fs::write(tmp.path().join("spanish.aff"), "SFX S Y 1\nSFX S 0 es .\n").unwrap();
        let dicts = installed_dictionaries_in(tmp.path());
        assert_eq!(dicts.len(), 1);
        match &dicts[0] {
            InstalledDictionary::Hunspell {
                lang,
                dic_path,
                aff_path,
            } => {
                assert_eq!(lang, "spanish");
                assert!(dic_path.ends_with("spanish.dic"));
                assert!(aff_path.ends_with("spanish.aff"));
            }
            InstalledDictionary::Wordlist { path, .. } => {
                panic!(
                    "expected a Hunspell entry, got a Wordlist entry: {}",
                    path.display()
                )
            }
        }
    }

    /// A lone `.dic` with no matching `.aff` is not a usable dictionary and
    /// must not be reported as installed.
    #[test]
    fn installed_dictionaries_in_ignores_dic_without_aff() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("spanish.dic"), "1\nsol/S\n").unwrap();
        let dicts = installed_dictionaries_in(tmp.path());
        assert!(
            dicts.is_empty(),
            "a .dic with no .aff must not be reported as installed: got entries for {:?}",
            dicts
                .iter()
                .map(InstalledDictionary::lang)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn ensure_dictionary_bails_in_test_harness_environment() {
        let _lock = ENV_MUTEX.lock().unwrap();
        // Simulate being run under a test harness like nextest by setting a
        // recognized environment variable. ensure_dictionary must not attempt
        // network activity in this case and should return an Err.
        let home = TempDir::new().unwrap();
        let orig_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", home.path());
        std::env::set_var("NEXTEST_RUN_ID", "1");
        let res = ensure_dictionary("spanish");
        assert!(
            res.is_err(),
            "Expected ensure_dictionary to error when under test harness"
        );
        std::env::remove_var("NEXTEST_RUN_ID");
        match orig_home {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
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

    /// TE11: Spanish gets a Hunspell `.dic`+`.aff` pair (not a plain
    /// wordlist, and not the "no source" state TE10 left it in), for both
    /// spellings of the language.
    #[test]
    fn remote_for_language_returns_hunspell_pair_for_spanish() {
        assert!(matches!(
            remote_for_language("spanish"),
            Some(RemoteSource::Hunspell { .. })
        ));
        assert!(matches!(
            remote_for_language("es"),
            Some(RemoteSource::Hunspell { .. })
        ));
    }

    /// English keeps its single-URL wordlist source, unmigrated (decision 3).
    #[test]
    fn remote_for_language_returns_single_url_for_english() {
        assert!(matches!(
            remote_for_language("english"),
            Some(RemoteSource::Wordlist(_))
        ));
    }

    #[test]
    fn hunspell_backend_accepts_a_stem() {
        let (dic, aff) = hunspell_fixture_paths();
        let dict = load_dictionary(&DictionaryLocation::Hunspell { dic, aff }).unwrap();
        assert!(
            dict.contains("gato"),
            "'gato' is a bare stem in the fixture .dic"
        );
    }

    /// The whole point of the change: a form that is NOT itself a line in
    /// the fixture `.dic`, but IS generated by the fixture `.aff`'s suffix
    /// rule (`sol/S` plus `SFX S 0 es .` yields "soles"), must be accepted.
    /// A test that only checked stems would pass against the old
    /// plain-wordlist backend too and would prove nothing about this change.
    #[test]
    fn hunspell_backend_accepts_affix_generated_form() {
        let (dic, aff) = hunspell_fixture_paths();
        let dict = load_dictionary(&DictionaryLocation::Hunspell { dic, aff }).unwrap();
        assert!(
            dict.contains("soles"),
            "'soles' is generated from stem 'sol' by the SFX S rule; it is not present verbatim in mini.dic"
        );
    }

    #[test]
    fn hunspell_backend_rejects_word_not_in_stems_or_generated_forms() {
        let (dic, aff) = hunspell_fixture_paths();
        let dict = load_dictionary(&DictionaryLocation::Hunspell { dic, aff }).unwrap();
        assert!(!dict.contains("xylophone"));
    }

    #[test]
    fn wordlist_backend_still_accepts_and_rejects_exactly_as_before() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("english.txt");
        fs::write(&path, "hello\nworld\n").unwrap();
        let dict = load_dictionary(&DictionaryLocation::Wordlist(path)).unwrap();
        assert!(dict.contains("hello"));
        assert!(dict.contains("world"));
        assert!(!dict.contains("goodbye"));
    }

    /// Decision 4: when both a `.txt` and a `.dic`/`.aff` exist on disk for
    /// one language, `ensure_dictionary` must choose the Hunspell pair.
    #[test]
    fn ensure_dictionary_prefers_hunspell_pair_when_both_present() {
        let _lock = ENV_MUTEX.lock().unwrap();
        let home = TempDir::new().unwrap();
        let dicts_dir = home.path().join(".texforge").join("dicts");
        fs::create_dir_all(&dicts_dir).unwrap();
        fs::write(dicts_dir.join("spanish.txt"), "hola\n").unwrap();
        let (fixture_dic, fixture_aff) = hunspell_fixture_paths();
        fs::copy(&fixture_dic, dicts_dir.join("spanish.dic")).unwrap();
        fs::copy(&fixture_aff, dicts_dir.join("spanish.aff")).unwrap();

        let orig_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", home.path());
        let loc = ensure_dictionary("spanish");
        match orig_home {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }

        match loc.unwrap() {
            DictionaryLocation::Hunspell { .. } => {}
            DictionaryLocation::Wordlist(p) => panic!(
                "expected the Hunspell pair to win over the wordlist, got wordlist path: {}",
                p.display()
            ),
        }
    }

    /// A `.dic` present with no `.aff` is not usable — it must be treated
    /// the same as "no dictionary available" (falling through to the normal
    /// missing-dictionary path and its skip message), never a panic.
    #[test]
    fn lint_files_treats_dic_without_aff_as_no_dictionary_available() {
        let _lock = ENV_MUTEX.lock().unwrap();
        let home = TempDir::new().unwrap();
        let dicts_dir = home.path().join(".texforge").join("dicts");
        fs::create_dir_all(&dicts_dir).unwrap();
        let (fixture_dic, _fixture_aff) = hunspell_fixture_paths();
        fs::copy(&fixture_dic, dicts_dir.join("spanish.dic")).unwrap();

        let orig_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", home.path());
        std::env::set_var("NEXTEST_RUN_ID", "te11-dic-without-aff");

        let src = "\\usepackage[spanish]{babel}\n\\begin{document}\nHola\n\\end{document}";
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
            "a lone .dic with no .aff must be treated as no dictionary available, not crash or \
             check against it: {:?}",
            findings
        );
    }

    // --- TE13: global personal dictionary unions with the project whitelist ---

    /// `global_whitelist_path` must be `~/.texforge/spell-words` — the exact
    /// path a user already tried before this feature existed (decision 2).
    #[test]
    fn global_whitelist_path_is_home_texforge_spell_words() {
        let _lock = ENV_MUTEX.lock().unwrap();
        let home = TempDir::new().unwrap();
        let orig_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", home.path());

        let path = global_whitelist_path().unwrap();

        match orig_home {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }

        assert_eq!(path, home.path().join(".texforge").join("spell-words"));
    }

    #[test]
    fn parse_whitelist_words_skips_blank_and_comment_lines_and_lowercases() {
        let content = "Docker\n# a comment\n\nAcme\n";
        let words = parse_whitelist_words(content);
        assert_eq!(words.len(), 2);
        assert!(words.contains("docker"));
        assert!(words.contains("acme"));
    }

    /// A word present only in the global personal dictionary must be
    /// accepted in a project that has no whitelist file at all
    /// (requirement 6).
    #[test]
    fn a_global_only_word_is_accepted_in_a_project_with_no_whitelist_file() {
        let _lock = ENV_MUTEX.lock().unwrap();
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
        fs::write(
            home.path().join(".texforge").join("spell-words"),
            "docker\n",
        )
        .unwrap();

        let orig_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", home.path());

        let src = "\\begin{document}\nHello docker world\n\\end{document}";
        let files = vec![("main.tex".to_string(), src.to_string())];
        // No whitelist file at all under the project root.
        let project_root = TempDir::new().unwrap();
        let findings = lint_files(&files, project_root.path(), Some("english"));

        match orig_home {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }

        let findings = findings.unwrap();
        assert!(
            findings.is_empty(),
            "'docker' is in the global personal dictionary and must be accepted: {:?}",
            findings
        );
    }

    /// A missing (or unreadable) global personal dictionary must not fail
    /// `check`, and must not change which findings are produced — reading it
    /// is best-effort, same as the project-local files.
    #[test]
    fn missing_global_whitelist_yields_no_error_and_no_findings_change() {
        let _lock = ENV_MUTEX.lock().unwrap();
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
        // Deliberately do NOT create ~/.texforge/spell-words.

        let orig_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", home.path());

        let src = "\\begin{document}\nHello docker world\n\\end{document}";
        let files = vec![("main.tex".to_string(), src.to_string())];
        let project_root = TempDir::new().unwrap();
        let findings = lint_files(&files, project_root.path(), Some("english"));

        match orig_home {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }

        let findings = findings.unwrap();
        assert_eq!(
            findings.len(),
            1,
            "no global dictionary present: 'docker' should still be flagged, and lint_files \
             must not error: {:?}",
            findings
        );
        assert!(findings[0].message.contains("docker"));
    }

    /// Both scopes union rather than either shadowing the other: a word only
    /// in the project file, and a different word only in the global file,
    /// are both accepted together (decision 3).
    #[test]
    fn project_and_global_whitelists_union_rather_than_override() {
        let _lock = ENV_MUTEX.lock().unwrap();
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
        fs::write(home.path().join(".texforge").join("spell-words"), "acme\n").unwrap();

        let orig_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", home.path());

        let project_root = TempDir::new().unwrap();
        fs::write(project_root.path().join("spell-whitelist.txt"), "docker\n").unwrap();

        let src = "\\begin{document}\nHello docker acme world\n\\end{document}";
        let files = vec![("main.tex".to_string(), src.to_string())];
        let findings = lint_files(&files, project_root.path(), Some("english"));

        match orig_home {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }

        let findings = findings.unwrap();
        assert!(
            findings.is_empty(),
            "words from both the project and global lists must be accepted together: {:?}",
            findings
        );
    }

    #[test]
    fn hunspell_pair_present_needs_both_files() {
        let tmp = TempDir::new().unwrap();
        let dic = tmp.path().join("spanish.dic");
        let aff = tmp.path().join("spanish.aff");
        assert!(hunspell_pair_present(Some(&dic), Some(&aff)).is_none());
        fs::write(&dic, "1\nsol\n").unwrap();
        assert!(
            hunspell_pair_present(Some(&dic), Some(&aff)).is_none(),
            "a lone .dic is not a usable pair"
        );
        fs::write(&aff, "SET UTF-8\n").unwrap();
        let (d, a) = hunspell_pair_present(Some(&dic), Some(&aff)).expect("both present");
        assert_eq!(d, dic);
        assert_eq!(a, aff);
        fs::remove_file(&dic).unwrap();
        assert!(
            hunspell_pair_present(Some(&dic), Some(&aff)).is_none(),
            "a lone .aff is not a usable pair"
        );
    }

    #[test]
    fn hunspell_pair_present_rejects_missing_paths() {
        let tmp = TempDir::new().unwrap();
        let dic = tmp.path().join("spanish.dic");
        assert!(hunspell_pair_present(None, None).is_none());
        assert!(hunspell_pair_present(Some(&dic), None).is_none());
        assert!(hunspell_pair_present(None, Some(&dic)).is_none());
    }

    /// Save the harness-detection vars, clear them, run `f`, then restore.
    /// Callers must hold [`ENV_MUTEX`]: the environment is process-global.
    fn with_harness_env_cleared(f: impl FnOnce()) {
        const VARS: &[&str] = &[
            "RUST_TEST_THREADS",
            "NEXTEST_CURRENT_RUN_ID",
            "NEXTEST_RUN_ID",
            "CI",
        ];
        let saved: Vec<(&str, Option<String>)> =
            VARS.iter().map(|v| (*v, std::env::var(v).ok())).collect();
        for v in VARS {
            std::env::remove_var(v);
        }
        f();
        for (v, val) in saved {
            match val {
                Some(s) => std::env::set_var(v, s),
                None => std::env::remove_var(v),
            }
        }
    }

    #[test]
    fn is_test_harness_detects_each_signal_alone() {
        let _lock = ENV_MUTEX.lock().unwrap();
        for var in [
            "RUST_TEST_THREADS",
            "NEXTEST_CURRENT_RUN_ID",
            "NEXTEST_RUN_ID",
            "CI",
        ] {
            with_harness_env_cleared(|| {
                std::env::set_var(var, "1");
                assert!(is_test_harness(), "{var} alone must signal a harness");
            });
        }
    }

    #[test]
    fn is_test_harness_is_false_with_no_signal() {
        let _lock = ENV_MUTEX.lock().unwrap();
        with_harness_env_cleared(|| {
            assert!(!is_test_harness());
        });
    }

    /// A tool that exists and exits 0 must report success: the success guard
    /// is what separates "downloaded" from "ran but failed".
    #[cfg(unix)]
    #[test]
    fn run_tool_reports_success_when_the_tool_succeeds() {
        match run_tool("true", &[], "true") {
            ToolOutcome::Success(_) => {}
            ToolOutcome::NotFound => panic!("'true' exists; must not report NotFound"),
            ToolOutcome::Failed(f) => {
                panic!("'true' exits 0; must not report failure: {}", f.detail)
            }
        }
    }

    /// A path that exists but cannot be executed fails with a spawn error
    /// other than `NotFound` — it must be reported as a failure, never as a
    /// missing tool.
    #[cfg(unix)]
    #[test]
    fn run_tool_reports_failure_not_absence_for_unrunnable_binaries() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = TempDir::new().unwrap();
        let target = tmp.path().join("not-executable");
        fs::write(&target, "not a binary\n").unwrap();
        let mut perms = fs::metadata(&target).unwrap().permissions();
        perms.set_mode(0o644);
        fs::set_permissions(&target, perms).unwrap();
        match run_tool(target.to_str().unwrap(), &[], "fixture") {
            ToolOutcome::Failed(_) => {}
            ToolOutcome::Success(_) => panic!("a non-executable must not succeed"),
            ToolOutcome::NotFound => {
                panic!("the file exists; a permission error must not report NotFound")
            }
        }
    }

    /// Curl missing but wget present-and-failing must report wget's own
    /// failure: the `(NotFound, Failed)` arm is what keeps a real transfer
    /// error from collapsing into "no download tool".
    #[cfg(unix)]
    #[test]
    fn download_with_tools_reports_wget_failure_when_curl_is_missing() {
        let bogus_curl = "definitely-not-a-real-binary-abc123";
        let err = download_with_tools("https://example.invalid/dict.txt", bogus_curl, "false")
            .expect_err("expected failure when wget exits non-zero");
        match &err {
            DownloadFailure::ToolError(f) => assert_eq!(f.tool, "wget"),
            DownloadFailure::NoToolFound => {
                panic!("wget exists and ran; must not report NoToolFound")
            }
        }
        let msg = err.describe("spanish", "https://example.invalid/dict.txt");
        assert!(msg.contains("wget"), "message should name wget: {}", msg);
    }

    #[test]
    fn installed_dictionaries_lists_the_managed_dir() {
        let _lock = ENV_MUTEX.lock().unwrap();
        let home = TempDir::new().unwrap();
        let dicts = home.path().join(".texforge").join("dicts");
        fs::create_dir_all(&dicts).unwrap();
        fs::write(dicts.join("english.txt"), "hello\n").unwrap();

        let orig_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", home.path());
        let listed = installed_dictionaries();
        match orig_home {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }

        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].lang(), "english");
    }
}
