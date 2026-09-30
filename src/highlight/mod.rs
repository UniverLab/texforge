//! Scan-and-rewrite pass for code listings, on build copies only.
//!
//! Mirrors [`crate::diagrams`]: the original `.tex` files are never touched —
//! everything happens on the copies `diagrams::process` just wrote into the
//! build directory, before `compiler::compile` runs. Without a `code` block
//! (and without the `lstlisting` opt-in) the pass does no work and no writes
//! at all, so existing documents build byte-identically to before this
//! feature existed.
//!
//! Emitted LaTeX depends only on `color.sty` and the LaTeX kernel: colours
//! are resolved here in Rust (route 3), never by `listings`, `minted`, or a
//! shell-escaped external tool.

use std::collections::{BTreeSet, HashMap};
use std::path::Path;

use anyhow::Result;

use crate::texparse;
use crate::texutil;

mod emit;
mod engine;
mod linemap;
mod preamble;

pub use engine::HighlightTheme;
pub use linemap::LineMap;

use emit::EmitOpts;
use engine::Rgb;

/// The `code` environment this pass owns; `check_collisions` guarantees
/// nothing else defines it.
pub const CODE_ENV: &str = "code";
/// Opt-in environment: only rewritten with `[highlight] lstlisting = true`.
pub const LST_ENV: &str = "lstlisting";

/// Option keys each environment consumes. Anything else warns through the
/// shared parser — which is exactly the "lstlisting options are dropped"
/// feedback, with no extra machinery.
const CODE_OPTION_KEYS: &[&str] = &["lang", "numbers"];
const LSTLISTING_OPTION_KEYS: &[&str] = &["language", "numbers"];

/// Everything `project.toml`'s `[highlight]` section contributes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settings {
    pub theme: HighlightTheme,
    /// Rewrite `\begin{lstlisting}` blocks too (off by default: `listings`
    /// users keep real `listings.sty` behaviour until they opt in).
    pub lstlisting: bool,
    /// Document-wide default for the line-number gutter; a block's
    /// `numbers=` option overrides it.
    pub numbers: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: HighlightTheme::Github,
            lstlisting: false,
            numbers: false,
        }
    }
}

/// A non-fatal finding. `line` is a line of the **build copy** — the same
/// coordinates Tectonic reports errors in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Warning {
    pub file: String,
    pub line: usize,
    pub message: String,
}

/// Run the pass and print warnings in the diagrams' `warning: …` shape.
/// Returns the build-copy → source line map for the compiler's warning
/// attribution.
pub fn process(build_dir: &Path, entry: &str, cfg: Settings) -> Result<LineMap> {
    let (warnings, line_map) = run_inner(build_dir, entry, cfg)?;
    for warning in &warnings {
        eprintln!(
            "warning: {}:{}: {}",
            warning.file, warning.line, warning.message
        );
    }
    Ok(line_map)
}

/// Run the pass, returning warnings instead of printing them.
/// Test-only helper: production builds go through [`process`] (which also
/// returns the line map); unit tests use this to assert warnings alone.
#[cfg(test)]
pub fn run(build_dir: &Path, entry: &str, cfg: Settings) -> Result<Vec<Warning>> {
    Ok(run_inner(build_dir, entry, cfg)?.0)
}

fn run_inner(build_dir: &Path, entry: &str, cfg: Settings) -> Result<(Vec<Warning>, LineMap)> {
    let paths = texutil::collect_tex_files(build_dir, entry).files;

    // Read everything first: the collision check must see every original —
    // a `\newenvironment{code}` can live in an `\input` preamble that has no
    // code block of its own (T3: check originals, before any rewrite).
    let mut sources: Vec<(String, String)> = Vec::with_capacity(paths.len());
    for path in &paths {
        let rel = path
            .strip_prefix(build_dir)
            .unwrap_or(path)
            .display()
            .to_string();
        sources.push((rel, std::fs::read_to_string(path)?));
    }

    // Discover the blocks this pass owns, per file. The guards come from the
    // shared tokenizer: a `\begin{code}` behind a `%`, or inside another
    // verbatim body, is *not* a code block — such a document stays
    // untouched (the "without a code block the pass is a no-op" guarantee),
    // and a `code` example quoted inside an `lstlisting` is left for the
    // listing that owns it.
    let discovered: Vec<Vec<Target>> = sources
        .iter()
        .map(|(_, content)| target_blocks(content, cfg))
        .collect();

    // Fast gate. No real block → nothing is rewritten, nothing is injected,
    // not one byte is written (T2: inertness is about writes, not output).
    if discovered.iter().all(Vec::is_empty) {
        return Ok((Vec::new(), LineMap::default()));
    }

    preamble::check_collisions(&sources)?;

    let mut colors: BTreeSet<Rgb> = BTreeSet::new();
    let mut has_gutter = false;
    let mut warnings = Vec::new();
    let mut rewritten_any = false;
    let mut line_map = LineMap::default();

    for (index, (rel, content)) in sources.iter_mut().enumerate() {
        let blocks = &discovered[index];
        if blocks.is_empty() {
            continue;
        }
        let mut origins = Vec::new();
        let mut state = RewriteState {
            colors: &mut colors,
            has_gutter: &mut has_gutter,
            warnings: &mut warnings,
            origins: &mut origins,
        };
        let rewritten = rewrite_file(rel, content, blocks, cfg, &mut state)?;
        std::fs::write(build_dir.join(rel.as_str()), &rewritten)?;
        if !origins.is_empty() {
            *line_map.file_mut(rel) = origins;
        }
        *content = rewritten;
        rewritten_any = true;
    }

    if rewritten_any {
        let color_loaded = preamble::color_pkg_visible_load(&sources);
        let block = preamble::injected_block(&colors, has_gutter, color_loaded, cfg.theme);
        let (anchor, entry_lines) = preamble::inject_entry(&build_dir.join(entry), &block)?;
        line_map.shift_for_injection(entry, anchor, block.lines().count(), entry_lines);
    }
    Ok((warnings, line_map))
}

/// One block this pass owns: the byte offset of its `\begin{…}` tag and the
/// environment name it is rewritten as. Offsets index the file as it was
/// read — the original build copy.
type Target = (usize, &'static str);

/// Every block in `content` the pass owns, in source order.
///
/// The scan itself is the literal `\begin{env}` search the pass always did,
/// narrowed by two guards the tokenizer already computed for us:
///
/// * an occurrence behind an unescaped `%` is a comment, not a block — the
///   "without a code block the pass is a no-op" guarantee must cover a
///   commented-out example, which the raw search cannot tell from a real
///   one;
/// * an occurrence inside a *foreign* verbatim body (`verbatim`, `minted`,
///   or a non-opted-in `lstlisting`) belongs to the environment quoting it:
///   a `code` sample shown inside a listing stays listing text.
///
/// Occurrences inside a block this pass itself owns are resolved by
/// [`rewrite_file`]'s cursor: the outer block starts first and swallows them.
fn target_blocks(content: &str, cfg: Settings) -> Vec<Target> {
    let envs: &[&'static str] = if cfg.lstlisting {
        &[CODE_ENV, LST_ENV]
    } else {
        &[CODE_ENV]
    };
    let foreign: Vec<(usize, usize)> = texparse::verbatim_blocks(content)
        .into_iter()
        .filter(|block| !envs.contains(&block.env.as_str()))
        .map(|block| (block.begin_start, block.end_end))
        .collect();

    let mut found: Vec<Target> = Vec::new();
    for &env in envs {
        let tag = format!("\\begin{{{env}}}");
        let mut search = 0usize;
        while let Some(rel) = content[search..].find(&tag) {
            let start = search + rel;
            search = start + tag.len();
            if in_comment(content, start) || foreign.iter().any(|&(a, b)| start >= a && start < b) {
                continue;
            }
            found.push((start, env));
        }
    }
    found.sort_unstable();
    found
}

/// Whether the byte at `offset` sits behind an unescaped `%` on its line.
fn in_comment(content: &str, offset: usize) -> bool {
    let line_start = content[..offset].rfind('\n').map_or(0, |i| i + 1);
    let prefix = &content[line_start..offset];
    texutil::strip_comment(prefix).len() < prefix.len()
}

/// Rewrite every block the pass owns, in one left-to-right pass over the
/// original content.
///
/// `blocks` comes from [`target_blocks`], so each entry is a real block this
/// pass owns; the loop itself needs no comment or nesting rules — everything
/// between two blocks (prose, comments, foreign verbatim bodies) is copied
/// through byte for byte.
///
/// Besides the rewritten text, `state.origins` receives one pass-input line
/// per output line, so the build can map finished-copy lines back to the
/// source (copied regions map 1:1; block lines map through the emission).
struct RewriteState<'a> {
    colors: &'a mut BTreeSet<Rgb>,
    has_gutter: &'a mut bool,
    warnings: &'a mut Vec<Warning>,
    origins: &'a mut Vec<usize>,
}

/// Push copied source text, recording one origin per output line. `src_line`
/// is the source line `text` starts on and advances past its newlines.
fn push_text(result: &mut String, origins: &mut Vec<usize>, text: &str, src_line: &mut usize) {
    result.push_str(text);
    for ch in text.chars() {
        if ch == '\n' {
            *src_line += 1;
            origins.push(*src_line);
        }
    }
}

fn rewrite_file(
    rel: &str,
    content: &str,
    blocks: &[Target],
    cfg: Settings,
    state: &mut RewriteState,
) -> Result<String> {
    let mut result = String::with_capacity(content.len());
    let mut origins: Vec<usize> = vec![1];
    let mut src_line = 1usize;
    let mut cursor = 0usize;

    for &(start, env) in blocks {
        if start < cursor {
            // Quoted inside a block this pass already rewrote (a `code`
            // sample inside an opted-in `lstlisting`): the outer block owns
            // that text, this occurrence is not a block of its own.
            continue;
        }
        let begin_tag = format!("\\begin{{{env}}}");
        let end_tag = format!("\\end{{{env}}}");
        let first_line = 1 + content[..start].matches('\n').count();
        push_text(
            &mut result,
            &mut origins,
            &content[cursor..start],
            &mut src_line,
        );

        let after_begin = &content[start + begin_tag.len()..];
        let known = if env == CODE_ENV {
            CODE_OPTION_KEYS
        } else {
            LSTLISTING_OPTION_KEYS
        };
        let (opts, after_opts) = texutil::parse_opts(after_begin, env, known)?;
        let end = texutil::find_end_tag(after_opts, &end_tag, env)?;

        // Absolute offset of `after_opts` in `content`. `parse_opts` consumes
        // the option text (and may eat one leading newline), so the length
        // it consumed must be added back before computing any line number
        // or advancing `cursor` — otherwise every block after the first
        // drifts.
        let body_abs = start + begin_tag.len() + (after_begin.len() - after_opts.len());
        let raw_body = &after_opts[..end];
        // Line of the body's first source character: one past the newline
        // `strip_one_newline` will eat, or the same line for an inline body.
        let stripped_newline = if raw_body.starts_with("\r\n") {
            2
        } else if raw_body.starts_with('\n') {
            1
        } else {
            0
        };
        let body_line = 1 + content[..body_abs + stripped_newline].matches('\n').count();

        let body = strip_one_newline(raw_body).replace('\r', "");
        let numbers = numbers_for(env, &opts, cfg);
        if numbers {
            *state.has_gutter = true;
        }

        let lang = lang_for(env, &opts);
        let spans = match engine::highlight(&lang, &body, cfg.theme)? {
            Some(spans) => Some(spans),
            None => {
                let warning = unknown_language_warning(env, &lang, first_line, rel);
                state.warnings.push(warning);
                None
            }
        };

        // The `\end{env}` tag starts here: its line owns the stanza's
        // closing lines in the line map.
        let end_line = 1 + content[..body_abs + end].matches('\n').count();
        let mut block_origins = Vec::new();
        let rendered = emit::render_block(
            &body,
            spans.as_deref(),
            &EmitOpts {
                file: rel,
                first_line,
                body_line,
                end_line,
                numbers,
            },
            state.colors,
            state.warnings,
            &mut block_origins,
        );
        // The emission's first line continues the current output line, whose
        // origin the copied text above already recorded — append the rest.
        result.push_str(&rendered);
        if block_origins.len() > 1 {
            origins.extend_from_slice(&block_origins[1..]);
        }
        src_line = end_line;

        cursor = body_abs + end + end_tag.len();

        // Vertical rhythm: `\medskip` after every block, and `\noindent` for
        // the paragraph that follows — unless the author left a blank line,
        // in which case the normal paragraph indent applies.
        let rest = &content[cursor..];
        let after = rest
            .strip_prefix("\r\n")
            .or_else(|| rest.strip_prefix('\n'))
            .unwrap_or(rest);
        let followed_by_prose = after
            .lines()
            .next()
            .is_some_and(|line| !line.trim().is_empty());
        result.push('\n');
        origins.push(end_line);
        result.push_str("\\medskip");
        if followed_by_prose {
            result.push_str("\\noindent ");
        }
    }

    push_text(&mut result, &mut origins, &content[cursor..], &mut src_line);
    state.origins.extend(origins);
    Ok(result)
}

/// Strip exactly one leading newline and one trailing newline (either `\n`
/// or `\r\n`) from the raw block body — and nothing else: real code
/// indentation and interior blank lines survive (unlike diagrams' `trim()`).
fn strip_one_newline(body: &str) -> &str {
    let body = body
        .strip_prefix("\r\n")
        .or_else(|| body.strip_prefix('\n'))
        .unwrap_or(body);
    body.strip_suffix("\r\n")
        .or_else(|| body.strip_suffix('\n'))
        .unwrap_or(body)
}

/// The effective `numbers` value: the environment's option wins over the
/// project default. `lstlisting` speaks `listings.sty`'s `left`/`right`,
/// and `none` is how an author turns numbering off for one block.
fn numbers_for(env: &str, opts: &HashMap<String, String>, cfg: Settings) -> bool {
    match opts.get("numbers") {
        None => cfg.numbers,
        Some(value) => match value.to_ascii_lowercase().as_str() {
            "left" | "right" | "true" => true,
            "false" | "none" => false,
            _ if env == LST_ENV => cfg.numbers,
            _ => false,
        },
    }
}

/// The `lang=`/`language=` value, normalised: `lstlisting` takes a
/// case-insensitive language name and drops an `[ISO …]`-style prefix.
fn lang_for(env: &str, opts: &HashMap<String, String>) -> String {
    let key = if env == CODE_ENV { "lang" } else { "language" };
    let raw = opts.get(key).map(String::as_str).unwrap_or("");
    let trimmed = raw.trim();
    if env != LST_ENV {
        return trimmed.to_string();
    }
    match trimmed.strip_prefix('[') {
        Some(rest) => rest
            .split_once(']')
            .map_or_else(|| trimmed.to_string(), |(_, after)| after.to_string()),
        None => trimmed.to_string(),
    }
}

fn unknown_language_warning(env: &str, lang: &str, first_line: usize, rel: &str) -> Warning {
    const TAIL: &str =
        "rendered without highlighting; see docs/listings.md for supported languages";
    let message = if env == CODE_ENV {
        format!("unknown code language '{lang}' — {TAIL}")
    } else {
        format!("unknown language '{lang}' in lstlisting — {TAIL}")
    };
    Warning {
        file: rel.to_string(),
        line: first_line,
        message,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal project fixture written into a temp build directory: prose,
    /// a diagram block, a `lstlisting` block, and (unless `with_code`) no
    /// `code` block at all.
    fn fixture(with_code: bool) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let mut main = String::from(
            "\\documentclass{article}\n\\begin{document}\nSome prose.\n\n\
             \\begin{lstlisting}[language=Python]\nx = 1\n\\end{lstlisting}\n\n\
             \\begin{mermaid}\nflowchart LR\n  A --> B\n\\end{mermaid}\n",
        );
        if with_code {
            main.push_str("\n\\begin{code}[lang=python]\ndef fib(n):\n    return n\n\\end{code}\n");
        }
        main.push_str("\\end{document}\n");
        std::fs::write(dir.path().join("main.tex"), main).unwrap();
        dir
    }

    fn snapshot(name: &str) -> String {
        let path = format!(
            "{}/src/highlight/snapshots/{name}",
            env!("CARGO_MANIFEST_DIR")
        );
        std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("golden snapshot missing ({e}): {path}"))
    }

    /// F1 — the golden end-to-end fixture: every escaping, layout and
    /// preamble rule is pinned by the committed snapshot.
    #[test]
    fn run_rewrites_code_blocks_golden() {
        let dir = tempfile::tempdir().unwrap();
        let main = "\\documentclass{article}\n\\begin{document}\n\
                    \\section{Demo}\n\
                    \\begin{code}[lang=python, numbers=true]\n\
                    def fib(n):\n    \ts = \"hi\"\n    if n < 2:\n        return n  # base\n\
                    \t$_%&#{}^~\\|<>\n\n    return fib(n - 1) + fib(n - 2)\n\
                    \\end{code}\n\n\
                    \\begin{code}[lang=markdown]\n\
                    # Notes\n\nSome *markdown* here.\n\
                    \\end{code}\n\n\
                    Prose after the blocks.\n\\end{document}\n";
        std::fs::write(dir.path().join("main.tex"), main).unwrap();

        let warnings = run(dir.path(), "main.tex", Settings::default()).unwrap();
        assert!(warnings.is_empty(), "warnings: {warnings:?}");

        let actual = std::fs::read_to_string(dir.path().join("main.tex")).unwrap();
        let name = "entry-github.tex";
        let path = format!(
            "{}/src/highlight/snapshots/{name}",
            env!("CARGO_MANIFEST_DIR")
        );
        if std::env::var_os("TEXFORGE_BLESS").is_some() {
            std::fs::create_dir_all(format!(
                "{}/src/highlight/snapshots",
                env!("CARGO_MANIFEST_DIR")
            ))
            .unwrap();
            std::fs::write(&path, &actual).unwrap();
            eprintln!("blessed {path}");
        }
        assert_eq!(actual, snapshot(name));
    }

    /// F3 — an unknown language must never fail the build: it warns once and
    /// renders monochrome.
    #[test]
    fn unknown_language_falls_back_to_monochrome() {
        let dir = fixture(false);
        let main = std::fs::read_to_string(dir.path().join("main.tex")).unwrap();
        let with_block = main.replace(
            "\\end{document}",
            "\\begin{code}[lang=brainfuck]\n+++[->+<]\n\\end{code}\n\\end{document}",
        );
        std::fs::write(dir.path().join("main.tex"), with_block).unwrap();

        let warnings = run(dir.path(), "main.tex", Settings::default()).unwrap();
        assert_eq!(warnings.len(), 1, "warnings: {warnings:?}");
        assert!(warnings[0].message.contains("brainfuck"), "{warnings:?}");
        assert!(warnings[0].message.contains("docs/listings.md"));
        assert_eq!(warnings[0].file, "main.tex");

        let out = std::fs::read_to_string(dir.path().join("main.tex")).unwrap();
        let block_start = out.find("\\noindent").unwrap();
        let block_end = out.find("\\par\n}").unwrap();
        let block = &out[block_start..block_end];
        assert!(
            !block.contains("\\textcolor{tfxcol"),
            "monochrome block must not colorize:\n{block}"
        );
        assert!(
            block.contains("\\textcolor{tfxtint}"),
            "even a monochrome block sits in the frame:\n{block}"
        );
    }

    /// A commented-out block is not a block: the "without a code block the
    /// pass is a no-op" guarantee covers it, so a document whose only
    /// `\begin{code}` sits behind a `%` stays byte-identical and compiles
    /// exactly as it did before the feature existed.
    #[test]
    fn commented_code_block_is_inert() {
        let dir = tempfile::tempdir().unwrap();
        let main = "\\documentclass{article}\n\\begin{document}\n\
                    % \\begin{code}[lang=python]\n% def f(): pass\n% \\end{code}\n\
                    Hello.\n\\end{document}\n";
        std::fs::write(dir.path().join("main.tex"), main).unwrap();

        let warnings = run(dir.path(), "main.tex", Settings::default()).unwrap();
        assert!(warnings.is_empty(), "warnings: {warnings:?}");
        assert_eq!(
            std::fs::read_to_string(dir.path().join("main.tex")).unwrap(),
            main,
            "a commented-out \\begin{{code}} must not be rewritten or injected"
        );
    }

    /// With a commented example *and* a real block in the same file, only the
    /// real one is rewritten: the comment keeps its text and the preamble is
    /// injected exactly once.
    #[test]
    fn commented_occurrence_does_not_shadow_the_real_block() {
        let dir = tempfile::tempdir().unwrap();
        let main = "\\documentclass{article}\n\\begin{document}\n\
                    % \\begin{code}[lang=python]\n% x = 1\n% \\end{code}\n\
                    \\begin{code}[lang=python]\ny = 2\n\\end{code}\n\
                    \\end{document}\n";
        std::fs::write(dir.path().join("main.tex"), main).unwrap();

        run(dir.path(), "main.tex", Settings::default()).unwrap();
        let out = std::fs::read_to_string(dir.path().join("main.tex")).unwrap();
        assert!(
            out.contains("% \\begin{code}[lang=python]\n% x = 1\n% \\end{code}"),
            "the commented example must survive verbatim:\n{out}"
        );
        assert_eq!(
            out.matches("{\n\\tfxcodestyle").count(),
            1,
            "exactly one block may be rewritten:\n{out}"
        );
        assert_eq!(
            out.matches("texforge code listings (injected").count(),
            1,
            "the preamble is injected once"
        );
    }

    /// A `code` example quoted *inside* another verbatim environment is text
    /// the quoting environment owns: without a real block of its own the
    /// whole file stays untouched (the raw `find`-based gate would have
    /// opened on the quoted tag and corrupted the listing).
    #[test]
    fn code_example_quoted_inside_a_listing_is_not_a_block() {
        for opener in ["lstlisting", "verbatim"] {
            let dir = tempfile::tempdir().unwrap();
            let main = format!(
                "\\documentclass{{article}}\n\\begin{{document}}\n\
                 \\begin{{{opener}}}\n\\begin{{code}}[lang=python]\nx = 1\n\\end{{code}}\n\
                 \\end{{{opener}}}\n\\end{{document}}\n"
            );
            std::fs::write(dir.path().join("main.tex"), &main).unwrap();

            let warnings = run(dir.path(), "main.tex", Settings::default()).unwrap();
            assert!(warnings.is_empty(), "{opener}: warnings: {warnings:?}");
            assert_eq!(
                std::fs::read_to_string(dir.path().join("main.tex")).unwrap(),
                main,
                "{opener}: a quoted \\begin{{code}} must stay inert"
            );
        }
    }

    /// With the opt-in on, a `code` example quoted inside an `lstlisting` is
    /// owned by the listing: the listing is rewritten once and the quoted
    /// tag inside it is not a second block (it would slice the output
    /// backwards if it were).
    #[test]
    fn quoted_code_inside_an_opted_in_listing_is_swallowed_by_the_listing() {
        let dir = tempfile::tempdir().unwrap();
        let main = "\\documentclass{article}\n\\begin{document}\n\
                    \\begin{lstlisting}[language=Python]\n\\begin{code}\nx = 1\n\\end{code}\n\
                    \\end{lstlisting}\n\\end{document}\n";
        std::fs::write(dir.path().join("main.tex"), main).unwrap();

        let cfg = Settings {
            lstlisting: true,
            ..Settings::default()
        };
        let warnings = run(dir.path(), "main.tex", cfg).unwrap();
        assert!(warnings.is_empty(), "warnings: {warnings:?}");
        let out = std::fs::read_to_string(dir.path().join("main.tex")).unwrap();
        assert_eq!(
            out.matches("{\n\\tfxcodestyle").count(),
            1,
            "the listing owns its quoted \\begin{{code}}:\n{out}"
        );
        assert!(!out.contains("\\begin{lstlisting}"), "rewritten:\n{out}");
    }

    /// F4 — without `code` (and without the opt-in) the pass is a no-op:
    /// byte-identical files, zero warnings, no injection marker.
    #[test]
    fn inert_without_code_blocks_or_opt_in() {
        let dir = fixture(false);
        let before = std::fs::read(dir.path().join("main.tex")).unwrap();

        let warnings = run(dir.path(), "main.tex", Settings::default()).unwrap();
        assert!(warnings.is_empty());
        let after = std::fs::read(dir.path().join("main.tex")).unwrap();
        assert_eq!(
            before, after,
            "an untouched document must stay byte-identical"
        );
        assert!(!String::from_utf8_lossy(&after).contains("texforge code listings"));

        // Second half: the same document with the opt-in DOES rewrite the
        // lstlisting block — proving the gate is the config, not the scanner.
        let opt_in = Settings {
            lstlisting: true,
            ..Settings::default()
        };
        let warnings = run(dir.path(), "main.tex", opt_in).unwrap();
        assert!(warnings.is_empty(), "warnings: {warnings:?}");
        let out = std::fs::read_to_string(dir.path().join("main.tex")).unwrap();
        assert_ne!(before, out.as_bytes(), "opt-in must rewrite lstlisting");
        assert!(
            !out.contains("\\begin{lstlisting}"),
            "block was replaced:\n{out}"
        );
        assert!(out.contains("texforge code listings"), "preamble injected");
    }

    /// F5 — a user-owned `code` environment or `\tfx` command fails before
    /// anything is written.
    #[test]
    fn collisions_fail_before_any_write() {
        for definition in [
            "\\newenvironment{code}[2]{a}{b}",
            "\\renewenvironment{code}{a}{b}",
            "\\NewDocumentEnvironment{code}{O{}m}{a}{b}",
            "\\def\\code{a}",
            "\\let\\code\\relax",
        ] {
            // F5 needs a block to rewrite: without one the pass is inert by
            // design (D14 gates the check on ≥1 block), so sabotage the
            // fixture that has a `code` block.
            let dir = fixture(true);
            let main = std::fs::read_to_string(dir.path().join("main.tex")).unwrap();
            let sabotage = main.replace(
                "\\begin{document}",
                &format!("{definition}\n\\begin{{document}}"),
            );
            std::fs::write(dir.path().join("main.tex"), &sabotage).unwrap();

            let err = run(dir.path(), "main.tex", Settings::default())
                .unwrap_err()
                .to_string();
            assert!(err.contains("main.tex"), "{definition}: {err}");
            assert!(err.contains("already defined"), "{definition}: {err}");
            let after = std::fs::read_to_string(dir.path().join("main.tex")).unwrap();
            assert_eq!(after, sabotage, "{definition}: nothing may be written");
        }
    }

    #[test]
    fn reserved_tfx_prefix_fails_before_any_write() {
        let dir = fixture(true);
        let main = std::fs::read_to_string(dir.path().join("main.tex")).unwrap();
        let sabotage = main.replace(
            "\\begin{document}",
            "\\newcommand{\\tfxcode}{x}\n\\begin{document}",
        );
        std::fs::write(dir.path().join("main.tex"), &sabotage).unwrap();

        let err = run(dir.path(), "main.tex", Settings::default())
            .unwrap_err()
            .to_string();
        assert!(err.contains("\\tfx"), "{err}");
        assert!(err.contains("main.tex"), "{err}");
        assert_eq!(
            std::fs::read_to_string(dir.path().join("main.tex")).unwrap(),
            sabotage
        );
    }

    #[test]
    fn collisions_ignores_commented_definitions() {
        let dir = fixture(true);
        let main = std::fs::read_to_string(dir.path().join("main.tex")).unwrap();
        let with_comment = main.replace(
            "\\begin{document}",
            "% \\newenvironment{code}{a}{b}\n% \\tfxowned\n\\begin{document}",
        );
        std::fs::write(dir.path().join("main.tex"), with_comment).unwrap();
        assert!(run(dir.path(), "main.tex", Settings::default()).is_ok());
    }

    /// F7 — gutter width and the overfull warning, end to end.
    #[test]
    fn numbers_and_overfull_are_reported_with_lines() {
        let dir = tempfile::tempdir().unwrap();
        let long = "x".repeat(100);
        let main = format!(
            "\\documentclass{{article}}\n\\begin{{document}}\n\
             \\begin{{code}}[lang=python, numbers=true]\n\
             a = 1\n{long}\nb = 2\n\
             \\end{{code}}\n\\end{{document}}\n"
        );
        std::fs::write(dir.path().join("main.tex"), main).unwrap();

        let warnings = run(dir.path(), "main.tex", Settings::default()).unwrap();
        assert_eq!(warnings.len(), 1, "warnings: {warnings:?}");
        assert!(
            warnings[0].message.contains("103 chars wide"),
            "{:?}",
            warnings[0]
        );
        assert!(warnings[0].message.contains("split it"));
        assert_eq!(
            warnings[0].line, 5,
            "the long line is line 5 of the build copy"
        );

        let out = std::fs::read_to_string(dir.path().join("main.tex")).unwrap();
        assert!(out.contains("\\hbox to 2em{\\hss 1}"));
        assert!(out.contains("\\hbox to 2em{\\hss 3}"));
        assert!(out.contains("\\definecolor{tfxgutter}"));
    }

    /// The document-wide default turns the gutter on for every block unless
    /// the block says otherwise.
    #[test]
    fn config_numbers_applies_when_the_block_is_silent() {
        let dir = fixture(true);
        let cfg = Settings {
            numbers: true,
            ..Settings::default()
        };
        run(dir.path(), "main.tex", cfg).unwrap();
        let out = std::fs::read_to_string(dir.path().join("main.tex")).unwrap();
        assert!(out.contains("\\textcolor{tfxgutter}"), "gutter expected");
        assert!(out.contains("\\definecolor{tfxgutter}"));
    }

    /// `numbers=none` is how a `listings` author turns numbering off for one
    /// block; it must override the document-wide default like `false` does.
    #[test]
    fn lstlisting_numbers_none_overrides_document_default() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("main.tex"),
            "\\documentclass{article}\n\\begin{document}\n\
             \\begin{lstlisting}[language=python, numbers=none]\nx = 1\n\\end{lstlisting}\n\
             \\end{document}\n",
        )
        .unwrap();
        let cfg = Settings {
            lstlisting: true,
            numbers: true,
            ..Settings::default()
        };
        let warnings = run(dir.path(), "main.tex", cfg).unwrap();
        assert!(warnings.is_empty(), "warnings: {warnings:?}");
        let out = std::fs::read_to_string(dir.path().join("main.tex")).unwrap();
        assert!(
            !out.contains("tfxgutter"),
            "numbers=none means no gutter:\n{out}"
        );
    }

    /// Warnings in the second and later blocks must name the right build-copy
    /// lines: the option text the first block consumed (and the newline
    /// `parse_opts` ate) must not shift every line number after it.
    #[test]
    fn warnings_in_later_blocks_keep_correct_lines() {
        let dir = tempfile::tempdir().unwrap();
        let long = "y".repeat(100);
        let main = format!(
            "\\documentclass{{article}}\n\\begin{{document}}\n\
             \\begin{{code}}[lang=python]\nx = 1\n\\end{{code}}\n\n\
             \\begin{{code}}[lang=brainfuck, numbers=true]\n+++[->+<]\nshort\n\
             {long}\n\\end{{code}}\n\\end{{document}}\n"
        );
        std::fs::write(dir.path().join("main.tex"), main).unwrap();

        let warnings = run(dir.path(), "main.tex", Settings::default()).unwrap();
        assert_eq!(warnings.len(), 2, "warnings: {warnings:?}");
        assert!(warnings[0].message.contains("brainfuck"), "{warnings:?}");
        assert_eq!(warnings[0].line, 7, "second \\begin sits on line 7");
        assert!(warnings[1].message.contains("chars wide"), "{warnings:?}");
        assert_eq!(warnings[1].line, 10, "the long line is line 10");
    }

    /// Options may span several lines; overfull warnings count from the body,
    /// not from the `\begin` line.
    #[test]
    fn multiline_options_do_not_shift_line_warnings() {
        let dir = tempfile::tempdir().unwrap();
        let long = "z".repeat(100);
        let main = format!(
            "\\documentclass{{article}}\n\\begin{{document}}\n\
             \\begin{{code}}[lang=python,\n numbers=true]\na = 1\n{long}\n\
             \\end{{code}}\n\\end{{document}}\n"
        );
        std::fs::write(dir.path().join("main.tex"), main).unwrap();

        let warnings = run(dir.path(), "main.tex", Settings::default()).unwrap();
        assert_eq!(warnings.len(), 1, "warnings: {warnings:?}");
        assert_eq!(warnings[0].line, 6, "body starts on line 5, long line is 6");
    }

    /// `input{}`ed files are rewritten too, and the preamble lands in the
    /// entry file only.
    #[test]
    fn input_files_are_rewritten_and_entry_owns_the_preamble() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("main.tex"),
            "\\documentclass{article}\n\\input{body}\n\\begin{document}\nHi.\n\\end{document}\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("body.tex"),
            "\\begin{code}[lang=rust]\nfn main() {}\n\\end{code}\n",
        )
        .unwrap();

        run(dir.path(), "main.tex", Settings::default()).unwrap();

        let body = std::fs::read_to_string(dir.path().join("body.tex")).unwrap();
        assert!(!body.contains("\\begin{code}"), "{body}");
        assert!(body.contains("\\noindent"), "{body}");
        let main = std::fs::read_to_string(dir.path().join("main.tex")).unwrap();
        assert!(main.contains("texforge code listings"), "{main}");
        assert!(!body.contains("texforge code listings"));
    }

    /// F6 — `xcolor` loaded from an `\input`ed preamble file drops the
    /// `\usepackage{color}` line but keeps the runtime guard.
    #[test]
    fn input_preamble_xcolor_load_keeps_guard_drops_usepackage() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("main.tex"),
            "\\documentclass{article}\n\\input{preamble}\n\\begin{document}\n\
             \\begin{code}[lang=python]\nx = 1\n\\end{code}\n\\end{document}\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("preamble.tex"),
            "\\usepackage[dvipsnames]{xcolor}\n",
        )
        .unwrap();

        let warnings = run(dir.path(), "main.tex", Settings::default()).unwrap();
        assert!(warnings.is_empty(), "warnings: {warnings:?}");
        let main = std::fs::read_to_string(dir.path().join("main.tex")).unwrap();
        assert!(main.contains("texforge code listings"), "{main}");
        assert!(!main.contains("\\usepackage{color}"), "{main}");
        assert!(main.contains("\\@ifpackageloaded{color}"), "{main}");
        assert!(main.contains("\\newcommand{\\tfxcodestyle}"), "{main}");
    }

    /// FR2 — the paragraph after a block starts without indent, unless the
    /// author left a blank line after `\end{code}` (then the normal
    /// paragraph indent applies).
    #[test]
    fn noindent_follows_the_block_unless_the_author_left_a_blank_line() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("main.tex"),
            "\\documentclass{article}\n\\begin{document}\n\
             \\begin{code}[lang=python]\nx = 1\n\\end{code}\nProse.\n\\end{document}\n",
        )
        .unwrap();
        run(dir.path(), "main.tex", Settings::default()).unwrap();
        let out = std::fs::read_to_string(dir.path().join("main.tex")).unwrap();
        assert!(
            out.contains("}\n\\medskip\\noindent \nProse."),
            "the glued paragraph must be suppressed:\n{out}"
        );

        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("main.tex"),
            "\\documentclass{article}\n\\begin{document}\n\
             \\begin{code}[lang=python]\nx = 1\n\\end{code}\n\nProse.\n\\end{document}\n",
        )
        .unwrap();
        run(dir.path(), "main.tex", Settings::default()).unwrap();
        let out = std::fs::read_to_string(dir.path().join("main.tex")).unwrap();
        assert!(
            out.contains("}\n\\medskip\n\nProse."),
            "a blank line keeps the normal indent:\n{out}"
        );
    }

    /// FR5 — every output line of a rewritten file maps back to the source
    /// line the pass received.
    #[test]
    fn line_map_attributes_a_code_line_to_its_source_line() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("main.tex"),
            "\\documentclass{article}\n\\begin{document}\n\
             \\begin{code}\na = 1\nb = 2\n\\end{code}\n\\end{document}\n",
        )
        .unwrap();
        let map = process(dir.path(), "main.tex", Settings::default()).unwrap();
        let rewritten = std::fs::read_to_string(dir.path().join("main.tex")).unwrap();
        // Source line 5 holds `b = 2` (plain block: escaped `b~=~2`).
        let build_line = rewritten
            .lines()
            .position(|line| line.contains("b~=~2"))
            .expect("the second code line must be in the build copy")
            + 1;
        assert_eq!(map.get("main.tex", build_line), Some(("main.tex", 5)));
        // The blank separator right after it — the line Tectonic reports for
        // the paragraph — maps to the code line above, not below.
        assert_eq!(map.get("main.tex", build_line + 1), Some(("main.tex", 5)));
    }

    /// The injection target need not own a block: when the only block lives
    /// in an `\input`, the entry's own lines still shift with the injected
    /// preamble and must stay mapped. Regression: the shift used to size the
    /// entry's map from an empty (unrecorded) origins vector, so every entry
    /// line below the anchor fell out of range and its warning kept the
    /// build-copy number.
    #[test]
    fn entry_without_its_own_block_stays_mapped() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("main.tex"),
            "\\documentclass{article}\n\\begin{document}\n\\input{body}\nProse.\n\\end{document}\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("body.tex"),
            "\\begin{code}[lang=python]\nx = 1\n\\end{code}\n",
        )
        .unwrap();

        let map = process(dir.path(), "main.tex", Settings::default()).unwrap();
        let rewritten = std::fs::read_to_string(dir.path().join("main.tex")).unwrap();
        assert!(rewritten.contains("texforge code listings"), "{rewritten}");

        // `Prose.` is source line 4; injection moved it down the build copy.
        let build_line = rewritten
            .lines()
            .position(|line| line == "Prose.")
            .expect("Prose. must survive into the build copy")
            + 1;
        assert!(
            build_line > 4,
            "the injected preamble must have shifted it: {build_line}"
        );
        assert_eq!(
            map.get("main.tex", build_line),
            Some(("main.tex", 4)),
            "an entry line below the anchor must map back to its source line"
        );
    }

    /// Bless-or-compare helper for the frame snapshots below.
    fn bless_or_compare(name: &str, actual: &str) {
        let path = format!(
            "{}/src/highlight/snapshots/{name}",
            env!("CARGO_MANIFEST_DIR")
        );
        if std::env::var_os("TEXFORGE_BLESS").is_some() {
            std::fs::create_dir_all(format!(
                "{}/src/highlight/snapshots",
                env!("CARGO_MANIFEST_DIR")
            ))
            .unwrap();
            std::fs::write(&path, actual).unwrap();
            eprintln!("blessed {path}");
        }
        assert_eq!(actual, snapshot(name));
    }

    /// A framed block without numbers: no gutter, no separator, frame and
    /// rhythm intact.
    #[test]
    fn framed_block_without_numbers_golden() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("main.tex"),
            "\\documentclass{article}\n\\begin{document}\n\
             \\begin{code}[lang=rust]\nfn main() {}\n\\end{code}\n\\end{document}\n",
        )
        .unwrap();
        run(dir.path(), "main.tex", Settings::default()).unwrap();
        let actual = std::fs::read_to_string(dir.path().join("main.tex")).unwrap();
        assert!(!actual.contains("tfxgutter"), "no gutter:\n{actual}");
        bless_or_compare("frame-no-numbers.tex", &actual);
    }

    /// A 77-line plain block: the full per-line emission a page split
    /// renders, including the two `\vadjust` penalties.
    #[test]
    fn page_split_block_golden() {
        let dir = tempfile::tempdir().unwrap();
        let mut body = String::new();
        for i in 1..=77 {
            body.push_str(&format!("code line {i}\n"));
        }
        let main = format!(
            "\\documentclass{{article}}\n\\begin{{document}}\n\
             \\begin{{code}}\n{body}\\end{{code}}\n\\end{{document}}\n"
        );
        std::fs::write(dir.path().join("main.tex"), main).unwrap();
        run(dir.path(), "main.tex", Settings::default()).unwrap();
        let actual = std::fs::read_to_string(dir.path().join("main.tex")).unwrap();
        assert_eq!(
            actual.matches("\\vadjust{\\penalty10000}").count(),
            2,
            "penalties on lines 1 and 76 only"
        );
        bless_or_compare("frame-page-split.tex", &actual);
    }

    /// F10 — the emitted LaTeX compiles under the real engine with only
    /// `color.sty`, whether or not the author loads `xcolor`, and unknown
    /// languages degrade without failing. Skips (never fails) without Tectonic.
    #[test]
    fn emitted_listings_compile_under_tectonic() {
        if crate::compiler::locate_tectonic().is_none() {
            eprintln!("skipping: tectonic not available in environment");
            return;
        }
        for (name, preamble, block, expect_warning, expect_text) in [
            (
                "plain",
                "",
                "\\begin{code}[lang=python]\ndef fib(n):\n    return n\n\\end{code}\n",
                false,
                "fib",
            ),
            (
                "xcolor",
                "\\usepackage{xcolor}\n",
                "\\begin{code}[lang=rust, numbers=true]\nfn fib(n: u64) -> u64 {\n    n\n}\n\\end{code}\n",
                false,
                "fib",
            ),
            (
                "unknown",
                "",
                "\\begin{code}[lang=brainfuck]\n+++[->+<]\n\\end{code}\n",
                true,
                "+++",
            ),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let main = format!(
                "\\documentclass{{article}}\n{preamble}\\begin{{document}}\n{block}\\end{{document}}\n"
            );
            std::fs::write(dir.path().join("main.tex"), &main).unwrap();

            let warnings = run(dir.path(), "main.tex", Settings::default()).unwrap();
            assert_eq!(!warnings.is_empty(), expect_warning, "{name}: {warnings:?}");

            crate::compiler::compile(dir.path(), "main.tex", false, None, &crate::highlight::LineMap::default())
                .unwrap_or_else(|e| panic!("{name}: tectonic failed: {e}"));
            let pdf = dir.path().join("main.pdf");
            assert!(pdf.exists(), "{name}: no pdf written");
            let text = crate::pdftext::extract_text(&pdf).unwrap();
            assert!(
                text.contains(expect_text),
                "{name}: listing text missing from pdf: {text:?}"
            );
        }
    }

    /// F10 (page breaks) — a 60-line block that starts halfway down page 1
    /// flows onto page 2: per-line paragraphs break freely, so TeX never has
    /// to overfull-hbox a listing it cannot fit. Both ends of the block are
    /// asserted, on their respective pages.
    #[test]
    fn long_listing_breaks_across_pages() {
        if crate::compiler::locate_tectonic().is_none() {
            eprintln!("skipping: tectonic not available in environment");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let prose = "The quick brown fox jumps over the lazy dog. ".repeat(140);
        let mut body = String::new();
        for i in 1..=60 {
            body.push_str(&format!("line_number_{i} = {i}\n"));
        }
        let main = format!(
            "\\documentclass{{article}}\n\\begin{{document}}\n{prose}\n\
             \\begin{{code}}[lang=python]\n{body}\\end{{code}}\n\\end{{document}}\n"
        );
        std::fs::write(dir.path().join("main.tex"), main).unwrap();

        run(dir.path(), "main.tex", Settings::default()).unwrap();
        crate::compiler::compile(
            dir.path(),
            "main.tex",
            false,
            None,
            &crate::highlight::LineMap::default(),
        )
        .unwrap();
        let pages = crate::pdftext::extract_text_by_pages(&dir.path().join("main.pdf")).unwrap();
        assert!(
            pages.len() >= 2,
            "expected a two-page document, got {}",
            pages.len()
        );
        let first = pages
            .iter()
            .position(|p| p.contains("line_number_1"))
            .expect("listing start missing from pdf");
        assert!(
            pages[first + 1..]
                .iter()
                .any(|p| p.contains("line_number_60")),
            "the block must continue past its first page: {pages:?}"
        );
    }
}
