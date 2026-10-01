//! Static linting rules.

mod engine;
mod glyphs;
pub(crate) mod spell;

pub use spell::{
    global_whitelist_path, installed_dictionaries, parse_whitelist_words, InstalledDictionary,
    PROJECT_WHITELIST_FILES,
};

use std::collections::{HashMap, HashSet};
use std::path::Path;

use anyhow::Result;

use crate::texparse::verbatim::VerbatimBlock;
use crate::texparse::{self, verbatim_blocks};
use crate::texutil;

/// Severity of a lint finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

impl std::fmt::Display for Severity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Severity::Error => write!(f, "ERROR"),
            Severity::Warning => write!(f, "WARNING"),
        }
    }
}

/// A lint finding with location, severity, and suggestion.
#[derive(Debug)]
pub struct LintFinding {
    pub file: String,
    pub line: usize,
    pub severity: Severity,
    pub message: String,
    pub suggestion: Option<String>,
}

impl std::fmt::Display for LintFinding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "  {}:{} — {}", self.file, self.line, self.message)?;
        if let Some(ref s) = self.suggestion {
            write!(f, "\n    suggestion: {}", s)?;
        }
        Ok(())
    }
}

/// Run all lint rules on a project directory.
pub fn lint(root: &Path, entry: &str, bib_file: Option<&str>) -> Result<Vec<LintFinding>> {
    let mut errors = Vec::new();

    let entry_path = root.join(entry);
    if !entry_path.exists() {
        errors.push(LintFinding {
            file: entry.to_string(),
            line: 0,
            severity: Severity::Error,
            message: "Entry point file does not exist".into(),
            suggestion: Some(format!("Create {}", entry)),
        });
        return Ok(errors);
    }

    let tex_files = collect_tex_files_and_report_circular(root, entry, &mut errors);
    let bib_keys = match bib_file {
        Some(bib) => parse_bib_keys(&root.join(bib)),
        None => HashSet::new(),
    };
    let all_labels = collect_labels(&tex_files)?;
    let file_contents = run_file_checks(
        root,
        &tex_files,
        bib_file,
        &bib_keys,
        &all_labels,
        &mut errors,
    )?;
    report_unused_bib(&bib_keys, &file_contents, bib_file, &mut errors);
    run_spell_and_engine(root, &file_contents, &mut errors);

    Ok(errors)
}

/// Collect `.tex` files reachable from the entry point, reporting circular
/// `\input` references as errors. Returns the reachable files in order.
fn collect_tex_files_and_report_circular(
    root: &Path,
    entry: &str,
    errors: &mut Vec<LintFinding>,
) -> Vec<std::path::PathBuf> {
    let collected = texutil::collect_tex_files(root, entry);
    for (entry, path) in &collected.circular {
        errors.push(LintFinding {
            severity: Severity::Error,
            file: entry.clone(),
            line: 0,
            message: format!("Circular \\input detected: {}", path.display()),
            suggestion: Some("Remove the circular reference".into()),
        });
    }
    collected.files
}

/// Collect every `\label` defined across files, skipping verbatim bodies.
///
/// Two sources define a label: a `\label{…}` command in document markup, and
/// a `label={…}` block option on a listing (`\begin{code}[caption={…},
/// label={lst:x}]`), which the highlighter turns into a real `\label`. Both
/// must be known or every `\ref` to a listing would be reported as dangling.
fn collect_labels(tex_files: &[std::path::PathBuf]) -> Result<HashSet<String>> {
    let mut all_labels = HashSet::new();
    for file in tex_files {
        let content = std::fs::read_to_string(file)?;
        let verbatim_lines = texparse::verbatim_body_lines(&content);
        for (index, line) in content.lines().enumerate() {
            if verbatim_lines.contains(&(index + 1)) {
                continue; // a `\label` in code is text, not a label
            }
            let line = texutil::strip_comment(line);
            for label in texutil::extract_commands(&line, "label") {
                all_labels.insert(label.to_string());
            }
        }
        for block in verbatim_blocks(&content) {
            if let Some(label) = listing_label_option(&content, &block) {
                all_labels.insert(label);
            }
        }
    }
    Ok(all_labels)
}

/// The `label={…}` a listing block declares, when it also has a `caption` —
/// the same pairing the highlighter enforces, since a label without a caption
/// is ignored there and must not be counted as a definition here.
///
/// Returns `None` for every other verbatim environment (`verbatim`,
/// `minted`, …), whose options belong to another package.
fn listing_label_option(content: &str, block: &VerbatimBlock) -> Option<String> {
    if block.env != "code" && block.env != "lstlisting" {
        return None;
    }
    let opts = &content[block.begin_start..block.body_start];
    let opts = opts.strip_prefix(&format!("\\begin{{{}}}", block.env))?;
    let (caption, label) = listing_caption_and_label(opts)?;
    caption?;
    Some(label)
}

/// The `caption` and `label` values of a listing's option list, honouring the
/// same brace nesting `texutil::parse_opts` implements (so `caption={a, b}` is
/// one value, not two). `None` when there is no option list at all.
fn listing_caption_and_label(opts: &str) -> Option<(Option<String>, String)> {
    let rest = opts.trim_start().strip_prefix('[')?;
    let mut values = HashMap::new();
    let mut depth = 0i32;
    let mut start = 0usize;
    let mut end = None;
    for (i, c) in rest.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => depth -= 1,
            ',' if depth == 0 => {
                record_option(&rest[start..i], &mut values);
                start = i + 1;
            }
            ']' if depth == 0 => {
                end = Some(i);
                break;
            }
            _ => {}
        }
    }
    record_option(rest.get(start..end?)?, &mut values);
    Some((
        values.get("caption").cloned(),
        values.get("label").cloned().unwrap_or_default(),
    ))
}

/// Record one `key=value` option, unwrapping the `{…}` form `parse_opts`
/// accepts. Only `caption` and `label` are kept; everything else is the
/// highlighter's business.
fn record_option(part: &str, values: &mut HashMap<String, String>) {
    let Some((key, value)) = part.trim().split_once('=') else {
        return;
    };
    let value = value.trim();
    let value = value
        .strip_prefix('{')
        .and_then(|v| v.strip_suffix('}'))
        .unwrap_or(value)
        .trim();
    if matches!(key.trim(), "caption" | "label") {
        values.insert(key.trim().to_string(), value.to_string());
    }
}

/// Run per-file reference, environment, diagram and glyph checks.
/// Returns the `(relative path, content)` pairs for later phases.
fn run_file_checks(
    root: &Path,
    tex_files: &[std::path::PathBuf],
    bib_file: Option<&str>,
    bib_keys: &HashSet<String>,
    all_labels: &HashSet<String>,
    errors: &mut Vec<LintFinding>,
) -> Result<Vec<(String, String)>> {
    let mut file_contents: Vec<(String, String)> = Vec::new();
    for file in tex_files {
        let rel = file
            .strip_prefix(root)
            .unwrap_or(file)
            .to_string_lossy()
            .to_string();
        let content = std::fs::read_to_string(file)?;
        // Lines wholly inside a verbatim body (`code`, `lstlisting`,
        // `verbatim`, `minted`) are source code: their `\input`, `\cite` and
        // `\begin{…}` are not document markup and must reach no line-based
        // rule. The tokenizer-based rules below already skip them.
        let verbatim_lines = texparse::verbatim_body_lines(&content);

        check_references(
            &RefCheckCtx {
                root,
                rel: &rel,
                bib_file,
                bib_keys,
                all_labels,
                verbatim_lines: &verbatim_lines,
            },
            &content,
            errors,
        );
        check_environments(&rel, &content, &verbatim_lines, errors);
        check_diagram_blocks(&rel, &content, "mermaid", &verbatim_lines, errors);
        check_diagram_blocks(&rel, &content, "graphviz", &verbatim_lines, errors);
        check_diagram_blocks(&rel, &content, "d2", &verbatim_lines, errors);
        errors.extend(glyphs::lint_file(&rel, &content));
        file_contents.push((rel, content));
    }
    Ok(file_contents)
}

/// Report `.bib` keys that are never cited. `\nocite{*}` cites every key.
fn report_unused_bib(
    bib_keys: &HashSet<String>,
    file_contents: &[(String, String)],
    bib_file: Option<&str>,
    errors: &mut Vec<LintFinding>,
) {
    let mut cited_keys = HashSet::new();
    let mut nocite_star = false;
    collect_cited_keys(file_contents, &mut cited_keys, &mut nocite_star);
    if nocite_star {
        return;
    }
    let mut unused: Vec<&str> = bib_keys
        .difference(&cited_keys)
        .map(String::as_str)
        .collect();
    if unused.is_empty() {
        return;
    }
    unused.sort();
    errors.push(LintFinding {
        severity: Severity::Warning,
        file: bib_file.unwrap_or("").to_string(),
        line: 0,
        message: format!("Unused .bib entries (never cited): {}", unused.join(", ")),
        suggestion: Some("Cite them with \\cite or remove them from the bibliography".into()),
    });
}

/// Run spell-checking (fail-open) and the engine-compatibility rules.
fn run_spell_and_engine(
    root: &Path,
    file_contents: &[(String, String)],
    errors: &mut Vec<LintFinding>,
) {
    // Spell-checking: attempt to load user-level default language and run spell
    // checks over the tokenized file contents. Fail-open: if spell-check cannot
    // obtain a dictionary, it prints a clear message and yields no findings.
    // To keep the test suite offline and deterministic, do not use the user's
    // config during unit tests — tests must explicitly opt-in by calling the
    // spell API with a local fixture language (see tests that pass Some("english")).
    let is_test_harness = std::env::var("RUST_TEST_THREADS").is_ok()
        || std::env::var("NEXTEST_CURRENT_RUN_ID").is_ok()
        || std::env::var("NEXTEST_RUN_ID").is_ok()
        || std::env::var("CI").is_ok();

    // When running under a test harness, ignore user config to avoid network
    // or environment leakage from the developer machine. Tests that want
    // spell-checking should pass a language and provide a local dictionary.
    let default_lang = if is_test_harness {
        None
    } else {
        crate::config::load()
            .ok()
            .and_then(|cfg| cfg.defaults.language)
    };

    if spell_check_enabled(default_lang.as_deref(), is_test_harness) {
        match spell::lint_files(file_contents, root, default_lang.as_deref()) {
            Ok(mut fs) => errors.append(&mut fs),
            Err(e) => eprintln!("Spell-check skipped: {}", e),
        }
    } else {
        eprintln!("Spell-check skipped: test harness detected and no default language configured");
    }

    errors.extend(engine::lint_files(file_contents));
}

/// Whether this run should attempt spell-checking at all: a configured
/// default language always opts in (the tests that want spell-checking pass
/// one explicitly); otherwise a test harness opts out, so the suite stays
/// offline and deterministic.
fn spell_check_enabled(default_lang: Option<&str>, is_test_harness: bool) -> bool {
    default_lang.is_some() || !is_test_harness
}

/// Document-wide state every line-based reference check needs: where the
/// document lives, which file is being checked, and the cross-file sets
/// (`\cite` keys, `\label`s) and verbatim lines collected once up front.
struct RefCheckCtx<'a> {
    root: &'a Path,
    rel: &'a str,
    bib_file: Option<&'a str>,
    bib_keys: &'a HashSet<String>,
    all_labels: &'a HashSet<String>,
    verbatim_lines: &'a HashSet<usize>,
}

/// Check \input, \includegraphics, \cite, \ref references.
fn check_references(ctx: &RefCheckCtx<'_>, content: &str, errors: &mut Vec<LintFinding>) {
    for (i, line) in content.lines().enumerate() {
        let line_num = i + 1;
        if ctx.verbatim_lines.contains(&line_num) {
            continue; // code, not markup
        }
        let line = texutil::strip_comment(line);

        check_input_references(ctx.root, ctx.rel, line_num, &line, errors);
        check_includegraphics_references(ctx.root, ctx.rel, line_num, &line, errors);
        check_cite_references(ctx.rel, line_num, &line, ctx.bib_file, ctx.bib_keys, errors);
        check_ref_references(ctx.rel, line_num, &line, ctx.all_labels, errors);
        check_lstinputlisting_references(ctx.root, ctx.rel, line_num, &line, errors);
        check_inputminted_references(ctx.root, ctx.rel, line_num, &line, errors);
    }
}

/// Check \input references for file existence.
fn check_input_references(
    root: &Path,
    rel: &str,
    line_num: usize,
    line: &str,
    errors: &mut Vec<LintFinding>,
) {
    for arg in texutil::extract_commands(line, "input") {
        let input_path = texutil::resolve_tex_path(root, arg);
        if !input_path.exists() {
            errors.push(LintFinding {
                severity: Severity::Error,
                file: rel.to_string(),
                line: line_num,
                message: format!("\\input{{{}}} — file not found", arg),
                suggestion: Some(format!("Create {}", input_path.display())),
            });
        }
    }
}

/// Check \includegraphics references for file existence.
fn check_includegraphics_references(
    root: &Path,
    rel: &str,
    line_num: usize,
    line: &str,
    errors: &mut Vec<LintFinding>,
) {
    for arg in texutil::extract_commands(line, "includegraphics") {
        let img_path = root.join(arg);
        if !img_path.exists() {
            errors.push(LintFinding {
                severity: Severity::Error,
                file: rel.to_string(),
                line: line_num,
                message: format!("\\includegraphics{{{}}} — file not found", arg),
                suggestion: None,
            });
        }
    }
}

/// Check \cite references against bibliography keys.
fn check_cite_references(
    rel: &str,
    line_num: usize,
    line: &str,
    bib_file: Option<&str>,
    bib_keys: &HashSet<String>,
    errors: &mut Vec<LintFinding>,
) {
    if bib_file.is_none() {
        return;
    }

    for arg in texutil::extract_commands(line, "cite") {
        for key in arg.split(',') {
            let key = key.trim();
            if !key.is_empty() && !bib_keys.contains(key) {
                errors.push(LintFinding {
                    severity: Severity::Error,
                    file: rel.to_string(),
                    line: line_num,
                    message: format!("\\cite{{{}}} — key not found in .bib", key),
                    suggestion: None,
                });
            }
        }
    }
}

/// Collect every cited key from in-memory file contents for the unused-bib check.
///
/// `\cite{...}` and `\nocite{...}` mark their keys as cited; `\nocite{*}` cites
/// every key in the bibliography, so it sets `nocite_star` instead.
fn collect_cited_keys(
    file_contents: &[(String, String)],
    cited_keys: &mut HashSet<String>,
    nocite_star: &mut bool,
) {
    for (_, content) in file_contents {
        let verbatim_lines = texparse::verbatim_body_lines(content);
        for (i, line) in content.lines().enumerate() {
            if verbatim_lines.contains(&(i + 1)) {
                continue; // a `\cite` in code cites nothing
            }
            let line = texutil::strip_comment(line);
            collect_cited_keys_in_line(&line, cited_keys, nocite_star);
        }
    }
}

/// Collect cited keys from a single comment-stripped line.
fn collect_cited_keys_in_line(
    line: &str,
    cited_keys: &mut HashSet<String>,
    nocite_star: &mut bool,
) {
    for arg in texutil::extract_commands(line, "cite") {
        insert_csv_keys(arg, cited_keys);
    }
    for arg in texutil::extract_commands(line, "nocite") {
        if arg == "*" {
            *nocite_star = true;
            continue;
        }
        insert_csv_keys(arg, cited_keys);
    }
}

/// Insert every comma-separated key in `arg` into `set`.
fn insert_csv_keys(arg: &str, set: &mut HashSet<String>) {
    for key in arg.split(',') {
        let key = key.trim();
        if key.is_empty() {
            continue;
        }
        set.insert(key.to_string());
    }
}

/// Check \ref references against defined labels.
fn check_ref_references(
    rel: &str,
    line_num: usize,
    line: &str,
    all_labels: &HashSet<String>,
    errors: &mut Vec<LintFinding>,
) {
    for arg in texutil::extract_commands(line, "ref") {
        if !all_labels.contains(arg) {
            errors.push(LintFinding {
                severity: Severity::Error,
                file: rel.to_string(),
                line: line_num,
                message: format!("\\ref{{{}}} — no matching \\label found", arg),
                suggestion: None,
            });
        }
    }
}

/// Check \lstinputlisting references for file existence.
fn check_lstinputlisting_references(
    root: &Path,
    rel: &str,
    line_num: usize,
    line: &str,
    errors: &mut Vec<LintFinding>,
) {
    for arg in texutil::extract_commands(line, "lstinputlisting") {
        if !root.join(arg).exists() {
            errors.push(LintFinding {
                severity: Severity::Error,
                file: rel.to_string(),
                line: line_num,
                message: format!("\\lstinputlisting{{{}}} — file not found", arg),
                suggestion: None,
            });
        }
    }
}

/// Check \inputminted references for file existence.
fn check_inputminted_references(
    root: &Path,
    rel: &str,
    line_num: usize,
    line: &str,
    errors: &mut Vec<LintFinding>,
) {
    for arg in extract_inputminted_files(line) {
        if !root.join(arg).exists() {
            errors.push(LintFinding {
                severity: Severity::Error,
                file: rel.to_string(),
                line: line_num,
                message: format!("\\inputminted{{{}}} — file not found", arg),
                suggestion: None,
            });
        }
    }
}

/// Check for unclosed \begin{env} environments.
fn check_environments(
    rel: &str,
    content: &str,
    verbatim_lines: &HashSet<usize>,
    errors: &mut Vec<LintFinding>,
) {
    // Stack of (env_name, line_number)
    let mut stack: Vec<(&str, usize)> = Vec::new();

    for (i, line) in content.lines().enumerate() {
        let line_num = i + 1;
        // Body lines of a verbatim region are opaque — but the `\begin` and
        // `\end` lines themselves are visible, so `code`/`lstlisting` still
        // have to balance like every other environment.
        if verbatim_lines.contains(&line_num) {
            continue;
        }
        let trimmed = line.trim();

        // Skip comments
        if trimmed.starts_with('%') {
            continue;
        }

        for env in texutil::extract_commands(trimmed, "begin") {
            stack.push((env, line_num));
        }

        for env in texutil::extract_commands(trimmed, "end") {
            if let Some((open_env, _)) = stack.last() {
                if *open_env == env {
                    stack.pop();
                } else {
                    errors.push(LintFinding {
                        severity: Severity::Error,
                        file: rel.to_string(),
                        line: line_num,
                        message: format!("\\end{{{}}} does not match \\begin{{{}}}", env, open_env),
                        suggestion: Some(format!("Expected \\end{{{}}}", open_env)),
                    });
                }
            } else {
                errors.push(LintFinding {
                    severity: Severity::Error,
                    file: rel.to_string(),
                    line: line_num,
                    message: format!("\\end{{{}}} without matching \\begin", env),
                    suggestion: None,
                });
            }
        }
    }

    // Report unclosed environments
    for (env, line_num) in stack {
        errors.push(LintFinding {
            severity: Severity::Error,
            file: rel.to_string(),
            line: line_num,
            message: format!("\\begin{{{}}} never closed", env),
            suggestion: Some(format!("Add \\end{{{}}}", env)),
        });
    }
}

/// Check mermaid/graphviz blocks: unclosed and invalid pos option.
fn check_diagram_blocks(
    rel: &str,
    content: &str,
    env: &str,
    verbatim_lines: &HashSet<usize>,
    errors: &mut Vec<LintFinding>,
) {
    for (i, line) in content.lines().enumerate() {
        let line_num = i + 1;
        // A `\begin{mermaid}` quoted inside `code` (or any verbatim body) is
        // sample text, not a diagram block.
        if verbatim_lines.contains(&line_num) {
            continue;
        }
        let trimmed = line.trim();

        if !trimmed.starts_with(&format!("\\begin{{{}}}", env)) {
            continue;
        }

        check_unclosed_diagram_block(rel, content, env, line_num, i, errors);
        check_diagram_pos_option(rel, trimmed, env, line_num, errors);
    }
}

/// Check if a diagram block is properly closed.
fn check_unclosed_diagram_block(
    rel: &str,
    content: &str,
    env: &str,
    line_num: usize,
    line_index: usize,
    errors: &mut Vec<LintFinding>,
) {
    let end_tag = format!("\\end{{{}}}", env);
    let rest = &content[content
        .lines()
        .take(line_index)
        .map(|l| l.len() + 1)
        .sum::<usize>()..];
    if !rest.contains(&*end_tag) {
        errors.push(LintFinding {
            severity: Severity::Error,
            file: rel.to_string(),
            line: line_num,
            message: format!("\\begin{{{}}} without matching \\end{{{}}}", env, env),
            suggestion: Some(format!("Add \\end{{{}}}", env)),
        });
    }
}

/// Check if the pos option in diagram block is valid.
fn check_diagram_pos_option(
    rel: &str,
    line: &str,
    env: &str,
    line_num: usize,
    errors: &mut Vec<LintFinding>,
) {
    const VALID_POS: &[&str] = &["H", "t", "b", "h", "p"];

    let Some(opts_start) = line.find('[') else {
        return;
    };
    let Some(opts_end) = line.find(']') else {
        return;
    };
    let opts = &line[opts_start + 1..opts_end];
    for part in opts.split(',') {
        let Some((k, v)) = part.split_once('=') else {
            continue;
        };
        if k.trim() != "pos" {
            continue;
        }
        let pos = v.trim();
        if VALID_POS.contains(&pos) {
            continue;
        }
        errors.push(LintFinding {
            severity: Severity::Error,
            file: rel.to_string(),
            line: line_num,
            message: format!(
                "\\begin{{{}}} invalid pos='{}' — valid values: H, t, b, h, p",
                env, pos
            ),
            suggestion: Some("Use pos=H, pos=t, pos=b, pos=h, or pos=p".into()),
        });
    }
}

/// Extract the file argument from `\inputminted{lang}{file}` (second brace group).
fn extract_inputminted_files(line: &str) -> Vec<&str> {
    let mut results = Vec::new();
    let mut search = line;
    while let Some(pos) = search.find("\\inputminted") {
        let after = &search[pos + "\\inputminted".len()..];
        // skip optional [...]
        let after = if after.starts_with('[') {
            match after.find(']') {
                Some(e) => &after[e + 1..],
                None => break,
            }
        } else {
            after
        };
        // skip first {lang}
        let after = if after.starts_with('{') {
            match after.find('}') {
                Some(e) => &after[e + 1..],
                None => break,
            }
        } else {
            break;
        };
        // extract second {file}
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
        break;
    }
    results
}

/// Parse `@type{key, ...}` entries from a .bib file.
fn parse_bib_keys(path: &Path) -> HashSet<String> {
    let Ok(content) = std::fs::read_to_string(path) else {
        return HashSet::new();
    };
    content
        .lines()
        .filter(|line| {
            let t = line.trim();
            t.starts_with('@') && !t.starts_with("@comment")
        })
        .filter_map(|line| {
            let t = line.trim();
            let start = t.find('{')?;
            let rest = &t[start + 1..];
            let end = rest.find(',')?;
            let key = rest[..end].trim();
            if key.is_empty() {
                None
            } else {
                Some(key.to_string())
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn setup(tex: &str) -> (TempDir, String) {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("main.tex"), tex).unwrap();
        (dir, "main.tex".to_string())
    }

    fn has_error(errors: &[LintFinding], fragment: &str) -> bool {
        errors.iter().any(|e| e.message.contains(fragment))
    }

    fn has_finding_with_severity(errors: &[LintFinding], fragment: &str, sev: Severity) -> bool {
        errors
            .iter()
            .any(|e| e.message.contains(fragment) && e.severity == sev)
    }

    #[test]
    fn includegraphics_missing_file_is_error() {
        let (dir, entry) = setup("\\includegraphics{missing.png}");
        let errors = lint(dir.path(), &entry, None).unwrap();
        assert!(has_error(&errors, "missing.png"));
    }

    #[test]
    fn includegraphics_existing_file_no_error() {
        let (dir, entry) = setup("\\includegraphics{img.png}");
        fs::write(dir.path().join("img.png"), b"").unwrap();
        let errors = lint(dir.path(), &entry, None).unwrap();
        assert!(!has_error(&errors, "img.png"));
    }

    #[test]
    fn cite_missing_key_is_error() {
        let (dir, entry) = setup("\\cite{ghost2020}");
        fs::write(dir.path().join("refs.bib"), "@article{real2020,}").unwrap();
        let errors = lint(dir.path(), &entry, Some("refs.bib")).unwrap();
        assert!(has_error(&errors, "ghost2020"));
    }

    #[test]
    fn cite_valid_key_no_error() {
        let (dir, entry) = setup("\\cite{real2020}");
        fs::write(dir.path().join("refs.bib"), "@article{real2020,}").unwrap();
        let errors = lint(dir.path(), &entry, Some("refs.bib")).unwrap();
        assert!(!has_error(&errors, "real2020"));
    }

    /// The spell-check gate: an explicit default language always opts in;
    /// without one, a test harness must stay out (offline, deterministic)
    /// while a real run falls through to the configured-default path.
    #[test]
    fn spell_check_gate_opts_in_for_a_language_and_out_for_a_harness() {
        assert!(spell_check_enabled(Some("english"), true));
        assert!(spell_check_enabled(Some("english"), false));
        assert!(spell_check_enabled(None, false));
        assert!(!spell_check_enabled(None, true));
    }

    /// A `\cite` inside a verbatim region is sample text, not a citation:
    /// the key must stay uncited, while a prose `\cite` after the block
    /// still counts.
    #[test]
    fn cite_inside_verbatim_cites_nothing_but_a_prose_cite_still_counts() {
        let content = "\\begin{verbatim}\n\\cite{ghost}\n\\end{verbatim}\n\\cite{real2020}\n";
        let mut cited = std::collections::HashSet::new();
        let mut nocite_star = false;
        collect_cited_keys(
            &[("main.tex".to_string(), content.to_string())],
            &mut cited,
            &mut nocite_star,
        );
        assert!(
            !cited.contains("ghost"),
            "a \\cite in code cites nothing: {cited:?}"
        );
        assert!(
            cited.contains("real2020"),
            "the prose cite after the block still counts: {cited:?}"
        );
        assert!(!nocite_star);
    }

    #[test]
    fn begin_without_end_is_error() {
        let (dir, entry) = setup("\\begin{figure}");
        let errors = lint(dir.path(), &entry, None).unwrap();
        assert!(has_error(&errors, "never closed"));
    }

    #[test]
    fn ref_without_label_is_error() {
        let (dir, entry) = setup("\\ref{fig:missing}");
        let errors = lint(dir.path(), &entry, None).unwrap();
        assert!(has_error(&errors, "fig:missing"));
    }

    #[test]
    fn mermaid_invalid_pos_is_error() {
        let (dir, entry) = setup("\\begin{mermaid}[pos=x]\n\\end{mermaid}");
        let errors = lint(dir.path(), &entry, None).unwrap();
        assert!(has_error(&errors, "invalid pos"));
    }

    #[test]
    fn mermaid_without_end_is_error() {
        let (dir, entry) = setup("\\begin{mermaid}");
        let errors = lint(dir.path(), &entry, None).unwrap();
        assert!(has_error(&errors, "without matching \\end{mermaid}"));
    }

    #[test]
    fn lstinputlisting_missing_file_is_error() {
        let (dir, entry) = setup("\\lstinputlisting{code.py}");
        let errors = lint(dir.path(), &entry, None).unwrap();
        assert!(has_error(&errors, "code.py"));
    }

    #[test]
    fn inputminted_missing_file_is_error() {
        let (dir, entry) = setup("\\inputminted{python}{code.py}");
        let errors = lint(dir.path(), &entry, None).unwrap();
        assert!(has_error(&errors, "code.py"));
    }

    #[test]
    fn graphviz_invalid_pos_is_error() {
        let (dir, entry) = setup("\\begin{graphviz}[pos=Z]\n\\end{graphviz}");
        let errors = lint(dir.path(), &entry, None).unwrap();
        assert!(has_error(&errors, "invalid pos"));
    }

    #[test]
    fn graphviz_without_end_is_error() {
        let (dir, entry) = setup("\\begin{graphviz}");
        let errors = lint(dir.path(), &entry, None).unwrap();
        assert!(has_error(&errors, "without matching \\end{graphviz}"));
    }

    #[test]
    fn d2_invalid_pos_is_error() {
        let (dir, entry) = setup("\\begin{d2}[pos=Z]\n\\end{d2}");
        let errors = lint(dir.path(), &entry, None).unwrap();
        assert!(has_error(&errors, "invalid pos"));
    }

    #[test]
    fn d2_without_end_is_error() {
        let (dir, entry) = setup("\\begin{d2}");
        let errors = lint(dir.path(), &entry, None).unwrap();
        assert!(has_error(&errors, "without matching \\end{d2}"));
    }

    #[test]
    fn entry_not_exists_is_error() {
        let dir = TempDir::new().unwrap();
        let errors = lint(dir.path(), "nonexistent.tex", None).unwrap();
        assert!(has_error(&errors, "does not exist"));
    }

    #[test]
    fn input_missing_file_is_error() {
        let (dir, entry) = setup("\\input{missing}");
        let errors = lint(dir.path(), &entry, None).unwrap();
        assert!(has_error(&errors, "missing"));
    }

    #[test]
    fn input_existing_file_no_error() {
        let (dir, entry) = setup("\\input{chapter1}");
        fs::write(dir.path().join("chapter1.tex"), "").unwrap();
        let errors = lint(dir.path(), &entry, None).unwrap();
        assert!(!has_error(&errors, "chapter1"));
    }

    #[test]
    fn begin_end_matched_no_error() {
        let (dir, entry) = setup("\\begin{figure}\n\\end{figure}");
        let errors = lint(dir.path(), &entry, None).unwrap();
        assert!(!has_error(&errors, "never closed"));
    }

    #[test]
    fn end_without_begin_is_error() {
        let (dir, entry) = setup("\\end{figure}");
        let errors = lint(dir.path(), &entry, None).unwrap();
        assert!(has_error(&errors, "without matching \\begin"));
    }

    #[test]
    fn mismatched_end_is_error() {
        let (dir, entry) = setup("\\begin{figure}\n\\end{table}");
        let errors = lint(dir.path(), &entry, None).unwrap();
        assert!(has_error(&errors, "does not match"));
    }

    #[test]
    fn comment_not_linted() {
        let (dir, entry) = setup("% \\includegraphics{missing.png}");
        let errors = lint(dir.path(), &entry, None).unwrap();
        assert!(!has_error(&errors, "missing.png"));
    }

    #[test]
    fn cite_no_bib_file_no_error() {
        let (dir, entry) = setup("\\cite{anything}");
        let errors = lint(dir.path(), &entry, None).unwrap();
        assert!(!has_error(&errors, "anything"));
    }

    #[test]
    fn mermaid_valid_pos_no_error() {
        let (dir, entry) = setup("\\begin{mermaid}[pos=H]\n\\end{mermaid}");
        let errors = lint(dir.path(), &entry, None).unwrap();
        assert!(!has_error(&errors, "invalid pos"));
    }

    #[test]
    fn graphviz_valid_pos_no_error() {
        let (dir, entry) = setup("\\begin{graphviz}[pos=t]\n\\end{graphviz}");
        let errors = lint(dir.path(), &entry, None).unwrap();
        assert!(!has_error(&errors, "invalid pos"));
    }

    #[test]
    fn d2_valid_pos_no_error() {
        let (dir, entry) = setup("\\begin{d2}[pos=b]\n\\end{d2}");
        let errors = lint(dir.path(), &entry, None).unwrap();
        assert!(!has_error(&errors, "invalid pos"));
    }

    #[test]
    fn lstinputlisting_existing_file_no_error() {
        let (dir, entry) = setup("\\lstinputlisting{code.py}");
        fs::write(dir.path().join("code.py"), "").unwrap();
        let errors = lint(dir.path(), &entry, None).unwrap();
        assert!(!has_error(&errors, "code.py"));
    }

    #[test]
    fn inputminted_existing_file_no_error() {
        let (dir, entry) = setup("\\inputminted{python}{code.py}");
        fs::write(dir.path().join("code.py"), "").unwrap();
        let errors = lint(dir.path(), &entry, None).unwrap();
        assert!(!has_error(&errors, "code.py"));
    }

    #[test]
    fn ref_with_matching_label_no_error() {
        let (dir, entry) = setup("\\label{fig:test}\n\\ref{fig:test}");
        let errors = lint(dir.path(), &entry, None).unwrap();
        assert!(!has_error(&errors, "fig:test"));
    }

    #[test]
    fn includegraphics_with_options() {
        let (dir, entry) = setup("\\includegraphics[width=0.5\\textwidth]{img.png}");
        fs::write(dir.path().join("img.png"), b"").unwrap();
        let errors = lint(dir.path(), &entry, None).unwrap();
        assert!(!has_error(&errors, "img.png"));
    }

    #[test]
    fn cite_multiple_keys() {
        let (dir, entry) = setup("\\cite{key1,key2}");
        fs::write(
            dir.path().join("refs.bib"),
            "@article{key1,}\n@article{key2,}",
        )
        .unwrap();
        let errors = lint(dir.path(), &entry, Some("refs.bib")).unwrap();
        assert!(!has_error(&errors, "key1"));
        assert!(!has_error(&errors, "key2"));
    }

    #[test]
    fn mixed_errors_and_valid() {
        let tex = "\\cite{missing}\n\\includegraphics{img.png}";
        let (dir, entry) = setup(tex);
        fs::write(dir.path().join("img.png"), b"").unwrap();
        fs::write(dir.path().join("refs.bib"), "").unwrap();
        let errors = lint(dir.path(), &entry, Some("refs.bib")).unwrap();
        assert!(has_error(&errors, "missing"));
        assert!(!has_error(&errors, "img.png"));
    }

    #[test]
    fn nested_begin_end() {
        let (dir, entry) =
            setup("\\begin{document}\n\\begin{figure}\n\\end{figure}\n\\end{document}");
        let errors = lint(dir.path(), &entry, None).unwrap();
        assert!(errors.is_empty());
    }

    #[test]
    fn unclosed_inner_environment() {
        let (dir, entry) = setup("\\begin{document}\n\\begin{figure}\n\\end{document}");
        let errors = lint(dir.path(), &entry, None).unwrap();
        assert!(has_error(&errors, "never closed"));
    }

    #[test]
    fn input_with_tex_extension() {
        let (dir, entry) = setup("\\input{chapter1.tex}");
        fs::write(dir.path().join("chapter1.tex"), "").unwrap();
        let errors = lint(dir.path(), &entry, None).unwrap();
        assert!(!has_error(&errors, "chapter1"));
    }

    #[test]
    fn empty_project_no_errors() {
        let (dir, entry) =
            setup("\\documentclass{article}\n\\begin{document}\nHello\n\\end{document}");
        let errors = lint(dir.path(), &entry, None).unwrap();
        assert!(errors.is_empty());
    }

    // --- Severity tests ---

    /// All current rules produce Error-severity findings.
    #[test]
    fn existing_findings_have_error_severity() {
        let (dir, entry) = setup("\\includegraphics{missing.png}");
        let findings = lint(dir.path(), &entry, None).unwrap();
        assert!(!findings.is_empty());
        assert!(has_finding_with_severity(
            &findings,
            "missing.png",
            Severity::Error
        ));
    }

    /// A clean project with no errors contains no findings.
    #[test]
    fn clean_project_has_no_findings() {
        let (dir, entry) =
            setup("\\documentclass{article}\n\\begin{document}\nHello\n\\end{document}");
        let findings = lint(dir.path(), &entry, None).unwrap();
        // No errors → check command would exit 0 regardless of --deny-warnings
        assert!(
            findings
                .iter()
                .filter(|f| f.severity == Severity::Error)
                .count()
                == 0
        );
    }

    /// A `code` block is verbatim to the linter (`code` is in
    /// `crate::texparse`'s verbatim list, like `lstlisting` and `minted`):
    /// bodies full of prose-fatal glyphs (`&`, `#`, `$`, `100%`, straight
    /// quotes, `...`) must produce no findings at any severity — documented
    /// in docs/listings.md.
    #[test]
    fn code_environment_body_is_opaque_to_every_rule() {
        let (dir, entry) = setup(
            "\\documentclass{article}\n\\begin{document}\n\
             \\begin{code}[lang=bash]\n#!/bin/bash\n\
             if [ \"$a\" = 1 ] && grep -q 'x' f; then echo 100% ...; fi\n\
             \\end{code}\n\\end{document}",
        );
        let findings = lint(dir.path(), &entry, None).unwrap();
        assert!(
            findings.is_empty(),
            "code body must be opaque to every linter rule: {findings:?}"
        );
    }

    /// Markup *quoted* inside a verbatim body is code, not document: the
    /// line-based reference, environment and diagram checks must not see it
    /// (the tokenizer-based rules already skip it). Markup just outside the
    /// block keeps being checked, so the guard is the body, not the file.
    #[test]
    fn verbatim_body_markup_is_not_checked_as_document_markup() {
        let (dir, entry) = setup(
            "\\documentclass{article}\n\\begin{document}\n\
             \\begin{code}[lang=latex]\n\\input{inside.tex}\n\\includegraphics{inside.png}\n\
             \\cite{inside}\n\\ref{inside}\n\\begin{mermaid}\n\\end{code}\n\
             \\input{outside.tex}\n\\end{document}",
        );
        let findings = lint(dir.path(), &entry, None).unwrap();
        assert!(
            has_error(&findings, "outside.tex"),
            "markup outside the block must still be checked: {findings:?}"
        );
        for inside in [
            "inside.tex",
            "inside.png",
            "\\ref{inside}",
            "\\cite{inside}",
            "mermaid",
        ] {
            assert!(
                !has_error(&findings, inside),
                "{inside:?} inside a code block must be opaque: {findings:?}"
            );
        }
    }

    /// The same opacity for `lstlisting` — its body was always verbatim to
    /// LaTeX, and now it is verbatim to the linter too.
    #[test]
    fn lstlisting_body_markup_is_not_checked_as_document_markup() {
        let (dir, entry) = setup(
            "\\documentclass{article}\n\\begin{document}\n\
             \\begin{lstlisting}\n\\input{inside.tex}\n\\begin{figure}\n\\end{lstlisting}\n\
             \\end{document}",
        );
        let findings = lint(dir.path(), &entry, None).unwrap();
        assert!(
            findings.is_empty(),
            "listing body must be opaque: {findings:?}"
        );
    }

    /// A `\label` that only exists inside a code block defines nothing — the
    /// block is verbatim — so a real `\ref` to it is reported.
    #[test]
    fn a_label_defined_only_inside_code_is_not_a_definition() {
        let (dir, entry) = setup(
            "\\documentclass{article}\n\\begin{document}\n\
             \\begin{code}[lang=latex]\n\\label{sec:ghost}\n\\end{code}\n\
             \\ref{sec:ghost}\n\\end{document}",
        );
        let findings = lint(dir.path(), &entry, None).unwrap();
        assert!(
            has_error(&findings, "\\ref{sec:ghost}"),
            "a code body cannot define a label: {findings:?}"
        );
    }

    /// A captioned listing's `label={…}` option defines a real label — the
    /// highlighter turns it into `\label{…}` — so a `\ref` to it resolves.
    #[test]
    fn a_listing_label_option_defines_a_label() {
        let (dir, entry) = setup(
            "\\documentclass{article}\n\\begin{document}\n\
             \\begin{code}[lang=python, caption={Fibo, clásico}, label={lst:fib}]\n\
             x = 1\n\\end{code}\n\
             Ver \\ref{lst:fib}.\n\\end{document}",
        );
        let findings = lint(dir.path(), &entry, None).unwrap();
        assert!(
            !has_error(&findings, "\\ref{lst:fib}"),
            "the listing option defines the label: {findings:?}"
        );
    }

    /// The same on an opted-in `lstlisting` block.
    #[test]
    fn a_lstlisting_label_option_defines_a_label() {
        let (dir, entry) = setup(
            "\\documentclass{article}\n\\begin{document}\n\
             \\begin{lstlisting}[language=Python, caption={Fibo}, label={lst:fib}]\n\
             x = 1\n\\end{lstlisting}\n\
             Ver \\ref{lst:fib}.\n\\end{document}",
        );
        let findings = lint(dir.path(), &entry, None).unwrap();
        assert!(
            !has_error(&findings, "\\ref{lst:fib}"),
            "the listing option defines the label: {findings:?}"
        );
    }

    /// A `label=` without a `caption=` is ignored by the highlighter, so the
    /// `\ref` to it really is dangling and must still be reported.
    #[test]
    fn a_listing_label_without_a_caption_is_still_dangling() {
        let (dir, entry) = setup(
            "\\documentclass{article}\n\\begin{document}\n\
             \\begin{code}[lang=python, label={lst:fib}]\nx = 1\n\\end{code}\n\
             Ver \\ref{lst:fib}.\n\\end{document}",
        );
        let findings = lint(dir.path(), &entry, None).unwrap();
        assert!(
            has_error(&findings, "\\ref{lst:fib}"),
            "no caption means no label: {findings:?}"
        );
    }

    /// `verbatim`/`minted` options belong to another package: a `label=`
    /// there is never a document label.
    #[test]
    fn a_foreign_environment_label_option_is_not_a_definition() {
        let (dir, entry) = setup(
            "\\documentclass{article}\n\\begin{document}\n\
             \\begin{verbatim}[caption={C}, label={lst:ghost}]\n\
             x = 1\n\\end{verbatim}\n\
             Ver \\ref{lst:ghost}.\n\\end{document}",
        );
        let findings = lint(dir.path(), &entry, None).unwrap();
        assert!(
            has_error(&findings, "\\ref{lst:ghost}"),
            "foreign options do not define labels: {findings:?}"
        );
    }

    /// Error-severity findings are detected independently of any deny-warnings flag
    /// (flag logic lives in the command layer; the linter just tags severity).
    #[test]
    fn error_severity_finding_present_when_error_rule_fires() {
        let (dir, entry) = setup("\\cite{ghost}");
        std::fs::write(dir.path().join("refs.bib"), "@article{real,}").unwrap();
        let findings = lint(dir.path(), &entry, Some("refs.bib")).unwrap();
        assert!(findings.iter().any(|f| f.severity == Severity::Error));
    }

    // --- Unused .bib entries (TF12) ---

    /// Unused bib keys are reported once, as a Warning naming the .bib file.
    #[test]
    fn unused_bib_entries_warn_as_single_finding() {
        let (dir, entry) = setup("\\cite{key1}\n\\cite{key2}");
        fs::write(
            dir.path().join("refs.bib"),
            "@article{key1,}\n@article{key2,}\n@article{unused3,}",
        )
        .unwrap();
        let findings = lint(dir.path(), &entry, Some("refs.bib")).unwrap();
        let unused: Vec<_> = findings
            .iter()
            .filter(|f| f.message.starts_with("Unused"))
            .collect();
        assert_eq!(unused.len(), 1, "grouped into one finding, not one per key");
        let finding = unused[0];
        assert_eq!(finding.severity, Severity::Warning);
        assert_eq!(finding.file, "refs.bib");
        assert!(finding.message.contains("unused3"));
        assert!(!finding.message.contains("key1"));
        assert!(!finding.message.contains("key2"));
    }

    /// `\nocite{*}` cites every key, so nothing is reported as unused.
    #[test]
    fn nocite_star_suppresses_unused_warning() {
        let (dir, entry) = setup("\\nocite{*}");
        fs::write(
            dir.path().join("refs.bib"),
            "@article{key1,}\n@article{key2,}",
        )
        .unwrap();
        let findings = lint(dir.path(), &entry, Some("refs.bib")).unwrap();
        assert!(
            !findings.iter().any(|f| f.message.starts_with("Unused")),
            "\\nocite{{*}} reaches every key; none are unused"
        );
    }

    /// Keys listed in `\nocite{...}` count as cited for the unused check.
    #[test]
    fn nocite_key_counts_as_cited() {
        let (dir, entry) = setup("\\nocite{key2}");
        fs::write(
            dir.path().join("refs.bib"),
            "@article{key1,}\n@article{key2,}",
        )
        .unwrap();
        let findings = lint(dir.path(), &entry, Some("refs.bib")).unwrap();
        let unused: Vec<_> = findings
            .iter()
            .filter(|f| f.message.starts_with("Unused"))
            .collect();
        assert_eq!(unused.len(), 1);
        assert!(unused[0].message.contains("key1"));
        assert!(!unused[0].message.contains("key2"));
    }

    /// A bibliography whose keys are all cited produces no unused warning.
    #[test]
    fn all_bib_entries_cited_no_unused_warning() {
        let (dir, entry) = setup("\\cite{key1,key2}");
        fs::write(
            dir.path().join("refs.bib"),
            "@article{key1,}\n@article{key2,}",
        )
        .unwrap();
        let findings = lint(dir.path(), &entry, Some("refs.bib")).unwrap();
        assert!(
            !findings.iter().any(|f| f.message.starts_with("Unused")),
            "all keys are cited"
        );
    }

    /// No bibliography means no unused-key warning.
    #[test]
    fn no_bib_file_no_unused_warning() {
        let (dir, entry) = setup("\\cite{key1}");
        let findings = lint(dir.path(), &entry, None).unwrap();
        assert!(
            !findings.iter().any(|f| f.message.starts_with("Unused")),
            "no .bib, nothing to check"
        );
    }
}
