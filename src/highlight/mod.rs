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

use crate::texutil;

mod emit;
mod engine;
mod preamble;

pub use engine::HighlightTheme;

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
pub fn process(build_dir: &Path, entry: &str, cfg: Settings) -> Result<()> {
    for warning in run(build_dir, entry, cfg)? {
        eprintln!(
            "warning: {}:{}: {}",
            warning.file, warning.line, warning.message
        );
    }
    Ok(())
}

/// Run the pass, returning warnings instead of printing them.
pub fn run(build_dir: &Path, entry: &str, cfg: Settings) -> Result<Vec<Warning>> {
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

    // Fast gate. No hit → nothing is rewritten, nothing is injected, not one
    // byte is written (T2: inertness is about writes, not just output).
    if !sources.iter().any(|(_, content)| is_gated(content, cfg)) {
        return Ok(Vec::new());
    }

    preamble::check_collisions(&sources)?;

    let mut colors: BTreeSet<Rgb> = BTreeSet::new();
    let mut has_gutter = false;
    let mut warnings = Vec::new();
    let mut rewritten_any = false;

    for (rel, content) in sources.iter_mut() {
        if !is_gated(content, cfg) {
            continue;
        }
        let rewritten = rewrite_file(
            rel,
            content,
            cfg,
            &mut colors,
            &mut has_gutter,
            &mut warnings,
        )?;
        std::fs::write(build_dir.join(rel), &rewritten)?;
        *content = rewritten;
        rewritten_any = true;
    }

    if rewritten_any {
        let color_loaded = preamble::color_pkg_visible_load(&sources);
        let block = preamble::injected_block(&colors, has_gutter, color_loaded);
        preamble::inject_entry(&build_dir.join(entry), &block)?;
    }
    Ok(warnings)
}

/// Whether a file contains anything this pass could rewrite.
fn is_gated(content: &str, cfg: Settings) -> bool {
    content.contains("\\begin{code}") || (cfg.lstlisting && content.contains("\\begin{lstlisting}"))
}

/// Rewrite every `code` block (and, when opted in, `lstlisting` block) in
/// one file.
fn rewrite_file(
    rel: &str,
    content: &str,
    cfg: Settings,
    colors: &mut BTreeSet<Rgb>,
    has_gutter: &mut bool,
    warnings: &mut Vec<Warning>,
) -> Result<String> {
    let out = rewrite_env(
        rel,
        content,
        CODE_ENV,
        CODE_OPTION_KEYS,
        cfg,
        colors,
        has_gutter,
        warnings,
    )?;
    if cfg.lstlisting {
        rewrite_env(
            rel,
            &out,
            LST_ENV,
            LSTLISTING_OPTION_KEYS,
            cfg,
            colors,
            has_gutter,
            warnings,
        )
    } else {
        Ok(out)
    }
}

/// The begin → options → end search loop, structurally the same shape as
/// `diagrams::render_env` (whose caching/callback machinery does not apply
/// here — nothing external is rendered). The eight parameters are the loop's
/// whole context; bundling them would obscure the mirroring.
///
#[allow(clippy::too_many_arguments)]
fn rewrite_env(
    rel: &str,
    content: &str,
    env: &str,
    known: &[&str],
    cfg: Settings,
    colors: &mut BTreeSet<Rgb>,
    has_gutter: &mut bool,
    warnings: &mut Vec<Warning>,
) -> Result<String> {
    let begin_tag = format!("\\begin{{{env}}}");
    let end_tag = format!("\\end{{{env}}}");

    let mut result = String::with_capacity(content.len());
    let mut remaining = content;
    let mut pos = 0usize;

    while let Some(start) = remaining.find(&begin_tag) {
        result.push_str(&remaining[..start]);
        let block_start = pos + start;
        let first_line = 1 + content[..block_start].matches('\n').count();

        let after_begin = &remaining[start + begin_tag.len()..];
        let label = if env == CODE_ENV { CODE_ENV } else { LST_ENV };
        let (opts, after_opts) = texutil::parse_opts(after_begin, label, known)?;
        let end = texutil::find_end_tag(after_opts, &end_tag, env)?;

        let body = strip_one_newline(&after_opts[..end]).replace('\r', "");
        let numbers = numbers_for(env, &opts, cfg);
        if numbers {
            *has_gutter = true;
        }

        let lang = lang_for(env, &opts);
        let spans = match engine::highlight(&lang, &body, cfg.theme)? {
            Some(spans) => Some(spans),
            None => {
                let warning = unknown_language_warning(env, &lang, first_line, rel);
                warnings.push(warning);
                None
            }
        };

        let rendered = emit::render_block(
            &body,
            spans.as_deref(),
            &EmitOpts {
                file: rel,
                first_line,
                numbers,
            },
            colors,
            warnings,
        );
        result.push_str(&rendered);

        pos = block_start + begin_tag.len() + end + end_tag.len();
        remaining = &after_opts[end + end_tag.len()..];
    }

    result.push_str(remaining);
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
/// project default. `lstlisting` speaks `listings.sty`'s `left`/`right`.
fn numbers_for(env: &str, opts: &HashMap<String, String>, cfg: Settings) -> bool {
    match opts.get("numbers") {
        None => cfg.numbers,
        Some(value) => match value.to_ascii_lowercase().as_str() {
            "left" | "right" | "true" => true,
            "false" => false,
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
            !block.contains("\\textcolor{"),
            "monochrome block must not colorize:\n{block}"
        );
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
        assert!(out.contains("\\hbox to 2em{1\\hss}"));
        assert!(out.contains("\\hbox to 2em{3\\hss}"));
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

            crate::compiler::compile(dir.path(), "main.tex", false, None)
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
        crate::compiler::compile(dir.path(), "main.tex", false, None).unwrap();
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
