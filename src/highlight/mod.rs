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

mod caption;
mod emit;
mod engine;
mod linemap;
mod palette;
mod preamble;

pub use engine::language_key;
pub use linemap::LineMap;
pub use palette::{HighlightStyle, HighlightTheme};
pub use preamble::ListingFont;

use emit::{EmitCaption, EmitOpts};
use engine::Rgb;
use palette::BlockStyle;

/// The `code` environment this pass owns; `check_collisions` guarantees
/// nothing else defines it.
pub const CODE_ENV: &str = "code";
/// Opt-in environment: only rewritten with `[highlight] lstlisting = true`.
pub const LST_ENV: &str = "lstlisting";

/// Option keys each environment consumes. Anything else warns through the
/// shared parser — which is exactly the "lstlisting options are dropped"
/// feedback, with no extra machinery.
// `style` is deliberately absent from `LSTLISTING_OPTION_KEYS`: `lstlisting`
// takes its style from `[highlight.by_lang]` or the document default, so a
// `style=` there keeps the usual "unknown option" warning instead of
// silently picking a treatment the author may have meant for a `code` block.
const CODE_OPTION_KEYS: &[&str] = &[
    "lang", "numbers", "caption", "label", "pos", "size", "style",
];
const LSTLISTING_OPTION_KEYS: &[&str] = &[
    "language",
    "numbers",
    "caption",
    "label",
    "float",
    "placement",
    "basicstyle",
];

/// Everything `project.toml`'s `[highlight]` section contributes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub theme: HighlightTheme,
    /// Document-wide default style; a block's `style=` or a `[highlight.by_lang]`
    /// entry wins over it.
    pub style: HighlightStyle,
    /// Per-language styles from `[highlight.by_lang]`, keyed by
    /// [`language_key`] of the language name or alias.
    pub by_lang: HashMap<String, HighlightStyle>,
    /// Rewrite `\begin{lstlisting}` blocks too (off by default: `listings`
    /// users keep real `listings.sty` behaviour until they opt in).
    pub lstlisting: bool,
    /// Document-wide default for the line-number gutter; a block's
    /// `numbers=` option overrides it.
    pub numbers: bool,
    pub caption_name: Option<String>,
    pub list_name: Option<String>,
    /// Typewriter family for code blocks; `Document` (default) injects nothing.
    pub font: ListingFont,
    /// Global `defaults.language`; `None` → english (spell-checker precedence).
    pub fallback_language: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: HighlightTheme::Github,
            style: HighlightStyle::Light,
            by_lang: HashMap::new(),
            lstlisting: false,
            numbers: false,
            caption_name: None,
            list_name: None,
            font: ListingFont::Document,
            fallback_language: None,
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
    let Settings {
        theme,
        style,
        by_lang,
        lstlisting,
        numbers,
        caption_name,
        list_name,
        font,
        fallback_language,
    } = cfg;
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
        .map(|(_, content)| target_blocks(content, lstlisting))
        .collect();

    // Fast gate. No real block → nothing is rewritten, nothing is injected,
    // not one byte is written (T2: inertness is about writes, not output).
    if discovered.iter().all(Vec::is_empty) {
        return Ok((Vec::new(), LineMap::default()));
    }

    preamble::check_collisions(&sources)?;

    let language = crate::linter::spell::document_language(&sources, fallback_language.as_deref());
    let names = caption::resolve_names(&language, caption_name.as_deref(), list_name.as_deref());
    let mut colors: BTreeSet<Rgb> = BTreeSet::new();
    let mut has_gutter = false;
    let mut has_caption = false;
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
            has_caption: &mut has_caption,
            warnings: &mut warnings,
            origins: &mut origins,
        };
        let rewritten = rewrite_file(
            rel,
            content,
            blocks,
            numbers,
            theme,
            &StyleDefaults {
                by_lang: &by_lang,
                document: style,
            },
            &mut state,
        )?;
        std::fs::write(build_dir.join(rel.as_str()), &rewritten)?;
        if !origins.is_empty() {
            *line_map.file_mut(rel) = origins;
        }
        *content = rewritten;
        rewritten_any = true;
    }

    if rewritten_any {
        let color_loaded = preamble::color_pkg_visible_load(&sources);
        let block = if has_caption {
            preamble::injected_block_with_caption(
                &colors,
                has_gutter,
                Some(&names),
                color_loaded,
                theme,
                font,
            )
        } else {
            preamble::injected_block(&colors, has_gutter, color_loaded, theme, font)
        };
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
fn target_blocks(content: &str, lstlisting: bool) -> Vec<Target> {
    let envs: &[&'static str] = if lstlisting {
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
        for (start, _) in content.match_indices(&tag) {
            if in_comment(content, start) || foreign.iter().any(|&(a, b)| (a..b).contains(&start)) {
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
    let prefix = content[..offset].rsplit('\n').next().unwrap_or_default();
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
    has_caption: &'a mut bool,
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
    default_numbers: bool,
    theme: HighlightTheme,
    styles: &StyleDefaults<'_>,
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
        // Line of the body's first source character: one past the leading
        // newline `strip_block_edges` will drop, or the same line for an
        // inline body.
        let stripped_newline = if raw_body.starts_with("\r\n") {
            2
        } else if raw_body.starts_with('\n') {
            1
        } else {
            0
        };
        let body_line = 1 + content[..body_abs + stripped_newline].matches('\n').count();

        let body = strip_block_edges(raw_body).replace('\r', "");
        let block_numbers = numbers_for(env, &opts, default_numbers);
        if block_numbers {
            *state.has_gutter = true;
        }

        let lang = lang_for(env, &opts);
        let style = styles.resolve(env, &opts, &lang)?;
        let block_palette = palette::palette(theme, style);
        let block_style =
            block_palette.block_style(block_numbers, style.paints_base(), state.colors);
        let spans = match engine::highlight(&lang, &body, theme, style)? {
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
        let resolved = resolve_block_options(env, &opts, &body, rel, first_line, state.warnings);
        if resolved.has_caption {
            *state.has_caption = true;
        }
        let emit_opts = resolved.emit_opts(
            rel,
            first_line,
            body_line,
            end_line,
            block_numbers,
            &block_style,
        );
        let floated = emit_opts.float.is_some();
        let mut block_origins = Vec::new();
        let rendered = emit::render_block(
            &body,
            spans.as_deref(),
            &emit_opts,
            state.colors,
            state.warnings,
            &mut block_origins,
        );
        // The emission's first line continues the current output line, whose
        // origin the copied text above already recorded — append the rest.
        result.push_str(&rendered);
        if let Some(rest) = block_origins.get(1..) {
            origins.extend_from_slice(rest);
        }
        src_line = end_line;

        cursor = body_abs + end + end_tag.len();
        push_block_tail(&mut result, &content[cursor..], floated);
        origins.push(end_line);
    }

    push_text(&mut result, &mut origins, &content[cursor..], &mut src_line);
    state.origins.extend(origins);
    Ok(result)
}

/// Vertical rhythm after a block: a new line, then `\medskip` after every
/// inline block, and `\noindent` for the paragraph that follows — unless the
/// author left a blank line, in which case the normal paragraph indent
/// applies. A floated block leaves the text flow: no vertical rhythm around
/// it. The caller records the new line's origin.
fn push_block_tail(result: &mut String, rest: &str, floated: bool) {
    let after = rest
        .strip_prefix("\r\n")
        .or_else(|| rest.strip_prefix('\n'))
        .unwrap_or(rest);
    let followed_by_prose = after
        .lines()
        .next()
        .is_some_and(|line| !line.trim().is_empty());
    result.push('\n');
    if !floated {
        result.push_str("\\medskip");
    }
    if followed_by_prose {
        result.push_str("\\noindent ");
    }
}

/// Strip the raw block body's edge lines — one leading newline and a final
/// line that holds only spaces or tabs, together with the newline before it —
/// and nothing else: real code indentation and interior blank lines survive
/// (unlike diagrams' `trim()`).
///
/// The block scanner leaves the `\end{code}` tag's indentation in the raw
/// body, so a body typically ends `\n  ` (newline plus the end tag's two
/// spaces); a raw body that ends in `\n` followed by zero or more spaces or
/// tabs is cut back to that newline. Trailing spaces on a real code line (no
/// newline after them) are kept, because there is no final whitespace-only
/// *line* to drop.
fn strip_block_edges(body: &str) -> &str {
    let body = match body.rfind('\n') {
        Some(newline) if body[newline + 1..].bytes().all(|b| b == b' ' || b == b'\t') => {
            let end = if body[..newline].ends_with('\r') {
                newline - 1
            } else {
                newline
            };
            &body[..end]
        }
        _ => body,
    };
    body.strip_prefix("\r\n")
        .or_else(|| body.strip_prefix('\n'))
        .unwrap_or(body)
}

/// The effective `numbers` value: the environment's option wins over the
/// project default. `lstlisting` speaks `listings.sty`'s `left`/`right`,
/// and `none` is how an author turns numbering off for one block.
fn numbers_for(env: &str, opts: &HashMap<String, String>, default_numbers: bool) -> bool {
    match opts.get("numbers") {
        None => default_numbers,
        Some(value) => match value.to_ascii_lowercase().as_str() {
            "left" | "right" | "true" => true,
            "false" | "none" => false,
            _ if env == LST_ENV => default_numbers,
            _ => false,
        },
    }
}

/// Resolved caption/size/placement for one block.
struct ResolvedBlock<'a> {
    caption: Option<&'a str>,
    label: Option<&'a str>,
    float: Option<String>,
    size_command: Option<&'static str>,
    has_caption: bool,
}

impl<'a> ResolvedBlock<'a> {
    /// Everything the emission needs: the block's own caption/float/size
    /// options, where it sits in the build copy, and the frame colours its
    /// style resolved to.
    fn emit_opts(
        &'a self,
        file: &'a str,
        first_line: usize,
        body_line: usize,
        end_line: usize,
        numbers: bool,
        style: &'a BlockStyle,
    ) -> EmitOpts<'a> {
        EmitOpts {
            file,
            first_line,
            body_line,
            end_line,
            numbers,
            caption: self.caption.map(|text| EmitCaption {
                text,
                label: self.label,
            }),
            float: self.float.as_deref(),
            size_command: self.size_command,
            colors: &style.colors,
            base: style.base,
        }
    }
}

/// Where a block's style comes from when the block does not say: the
/// `[highlight.by_lang]` table (keyed by [`language_key`], so aliases agree)
/// over the document-wide `[highlight] style`.
struct StyleDefaults<'a> {
    by_lang: &'a HashMap<String, HighlightStyle>,
    document: HighlightStyle,
}

impl StyleDefaults<'_> {
    /// Resolve one block's style, most specific first: the `code` block's own
    /// `style=`, then the language's entry, then the document default. An
    /// unknown `style=` value fails the build by name, like an unknown theme.
    fn resolve(
        &self,
        env: &str,
        opts: &HashMap<String, String>,
        lang: &str,
    ) -> Result<HighlightStyle> {
        if env == CODE_ENV {
            if let Some(name) = opts.get("style") {
                return HighlightStyle::parse(name);
            }
        }
        Ok(self
            .by_lang
            .get(&language_key(lang))
            .copied()
            .unwrap_or(self.document))
    }
}

/// Resolve `caption=`/`label=`/`pos=`/`size=` (and the `lstlisting`
/// spellings) for one block, pushing warnings for misuse.
fn resolve_block_options<'a>(
    env: &str,
    opts: &'a HashMap<String, String>,
    body: &str,
    rel: &str,
    first_line: usize,
    warnings: &mut Vec<Warning>,
) -> ResolvedBlock<'a> {
    let caption = opts.get("caption").map(String::as_str);
    let label = match (opts.get("label").map(String::as_str), caption) {
        (Some(label), Some(_)) => Some(label),
        (Some(_), None) => {
            warnings.push(Warning {
                file: rel.to_string(),
                line: first_line,
                message: "label without caption is ignored".to_string(),
            });
            None
        }
        (None, _) => None,
    };
    let size = size_for(env, opts, rel, first_line, warnings);
    let mut placement = placement_for(env, opts, rel, first_line, warnings);
    if let caption::Placement::Float(_) = placement {
        let lines = if body.is_empty() {
            1
        } else {
            body.split('\n').count()
        };
        if !caption::fits_on_page(lines, size) {
            warnings.push(Warning {
                file: rel.to_string(),
                line: first_line,
                message: "floated listing is too tall to fit on one page — rendering it inline"
                    .to_string(),
            });
            placement = caption::Placement::Inline;
        }
    }
    let float = match placement {
        caption::Placement::Inline => None,
        caption::Placement::Float(pos) => Some(pos),
    };
    ResolvedBlock {
        caption,
        label,
        float,
        size_command: size.command(),
        has_caption: caption.is_some(),
    }
}

/// The effective `size`: `code` reads `size=`; `lstlisting` reads a
/// `basicstyle=` font-size command. Unknown `size=` warns and uses `small`.
fn size_for(
    env: &str,
    opts: &HashMap<String, String>,
    rel: &str,
    first_line: usize,
    warnings: &mut Vec<Warning>,
) -> caption::Size {
    if env == CODE_ENV {
        match opts.get("size").map(String::as_str) {
            None => caption::Size::Small,
            Some(value) => match caption::Size::parse(value) {
                Some(size) => size,
                None => {
                    warnings.push(Warning {
                        file: rel.to_string(),
                        line: first_line,
                        message: format!(
                            "unknown code size '{value}' — valid values are scriptsize, footnotesize, small, normalsize; using small"
                        ),
                    });
                    caption::Size::Small
                }
            },
        }
    } else {
        opts.get("basicstyle")
            .map(String::as_str)
            .and_then(basicstyle_size)
            .unwrap_or(caption::Size::Small)
    }
}

/// Extract a recognised font-size command from a `basicstyle=` value, with a
/// whole-word match so `\small` never matches `\smallskip`.
fn basicstyle_size(value: &str) -> Option<caption::Size> {
    for (command, size) in [
        ("\\scriptsize", caption::Size::Scriptsize),
        ("\\footnotesize", caption::Size::Footnotesize),
        ("\\small", caption::Size::Small),
        ("\\normalsize", caption::Size::Normalsize),
    ] {
        let mut search = value;
        while let Some(pos) = search.find(command) {
            let after = &search[pos + command.len()..];
            if after
                .chars()
                .next()
                .is_none_or(|c| !c.is_ascii_alphabetic())
            {
                return Some(size);
            }
            search = after;
        }
    }
    None
}

/// The effective placement: `code` reads `pos=`; `lstlisting` reads `float=`
/// or `placement=` (first present). Unknown values warn and render inline.
fn placement_for(
    env: &str,
    opts: &HashMap<String, String>,
    rel: &str,
    first_line: usize,
    warnings: &mut Vec<Warning>,
) -> caption::Placement {
    let raw = if env == CODE_ENV {
        opts.get("pos").map(String::as_str)
    } else {
        opts.get("float")
            .or_else(|| opts.get("placement"))
            .map(String::as_str)
    };
    match caption::Placement::parse(raw) {
        Ok(placement) => placement,
        Err(value) => {
            warnings.push(Warning {
                file: rel.to_string(),
                line: first_line,
                message: format!(
                    "unknown placement '{value}' — expected one of H, h, t, b, p; rendering inline"
                ),
            });
            caption::Placement::Inline
        }
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

    // ── The scanners this pass gates on, at unit level ─────────────

    #[test]
    fn target_blocks_finds_the_real_block_and_skips_the_commented_one() {
        let content = "\\begin{code}x\\end{code}\n% \\begin{code}y\\end{code}\n";
        let targets = target_blocks(content, false);
        assert_eq!(targets.len(), 1, "only the uncommented block: {targets:?}");
        assert_eq!(targets[0].0, 0, "the block starts at offset 0");
    }

    /// A `\begin{code}` that starts exactly where a foreign block ends is
    /// the pass's own block: the foreign region covers everything up to —
    /// never including — its first byte after `\end{verbatim}` — and a
    /// block before the foreign region is unaffected by it.
    #[test]
    fn blocks_around_a_foreign_region_are_still_owned() {
        let content =
            "\\begin{code}a\\end{code}\\begin{verbatim}v\\end{verbatim}\\begin{code}b\\end{code}";
        let targets = target_blocks(content, false);
        assert_eq!(
            targets.len(),
            2,
            "the code block before and the one right after \\end{{verbatim}}: {targets:?}"
        );
        assert_eq!(targets[0].0, 0, "the first block starts at offset 0");
        let second_start = content.rfind("\\begin{code}").expect("second block");
        assert_eq!(targets[1].0, second_start);
    }

    /// Two blocks with nothing between them are two blocks: the second
    /// starts exactly where the first ended, and the quoted-block swallow
    /// guard (`start < cursor`) must not eat it.
    #[test]
    fn adjacent_blocks_are_both_rewritten() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("main.tex"),
            "\\documentclass{article}\n\\begin{document}\n\\begin{code}\na = 1\n\\end{code}\\begin{code}\nb = 2\n\\end{code}\n\\end{document}\n",
        )
        .unwrap();
        run(dir.path(), "main.tex", Settings::default()).unwrap();
        let out = std::fs::read_to_string(dir.path().join("main.tex")).unwrap();
        assert_eq!(
            out.matches("{\n\\tfxcodestyle").count(),
            2,
            "both adjacent blocks must be rewritten:\n{out}"
        );
    }

    #[test]
    fn in_comment_only_fires_behind_an_unescaped_percent_on_the_same_line() {
        assert!(!in_comment("x", 1), "no % at all");
        assert!(!in_comment("a\nb", 2), "b starts its own line");
        assert!(in_comment("x% tail", 6), "behind an unescaped %");
        assert!(!in_comment("x\\% y", 3), "an escaped % is not a comment");
    }

    /// The environment's own `numbers=` option wins over the project
    /// default — `lstlisting` falls back to the default for an unrecognised
    /// value, `code` never opts itself in.
    #[test]
    fn numbers_for_gives_the_block_option_the_final_say() {
        let opts = |v: &str| HashMap::from([("numbers".to_string(), v.to_string())]);

        assert!(numbers_for(CODE_ENV, &opts("left"), false));
        assert!(!numbers_for(CODE_ENV, &opts("none"), true));
        assert!(
            !numbers_for(CODE_ENV, &opts("bogus"), true),
            "an unknown value never opts a code block in"
        );
        assert!(
            numbers_for(LST_ENV, &opts("bogus"), true),
            "lstlisting falls back to the project default"
        );
        assert!(!numbers_for(LST_ENV, &opts("bogus"), false));
        assert!(numbers_for(CODE_ENV, &HashMap::new(), true));
    }

    /// Resolution order, most specific first: the `code` block's own
    /// `style=`, then the language's `[highlight.by_lang]` entry, then the
    /// document-wide default.
    #[test]
    fn style_resolution_runs_block_then_by_lang_then_document() {
        let rules = StyleDefaults {
            by_lang: &HashMap::from([("bash".to_string(), HighlightStyle::Dark)]),
            document: HighlightStyle::LightMono,
        };
        let code = |v: &str| HashMap::from([("style".to_string(), v.to_string())]);

        assert_eq!(
            rules.resolve(CODE_ENV, &code("dark-mono"), "bash").unwrap(),
            HighlightStyle::DarkMono,
            "the block option beats the language entry"
        );
        assert_eq!(
            rules.resolve(CODE_ENV, &HashMap::new(), "bash").unwrap(),
            HighlightStyle::Dark,
            "by_lang beats the document default"
        );
        assert_eq!(
            rules.resolve(CODE_ENV, &HashMap::new(), "python").unwrap(),
            HighlightStyle::LightMono,
            "an unlisted language falls back to the document"
        );
    }

    /// `[highlight.by_lang]` keys are normalised on both sides, so one entry
    /// covers every spelling of its language.
    #[test]
    fn by_lang_matches_a_block_through_the_aliases() {
        let rules = StyleDefaults {
            by_lang: &HashMap::from([("bash".to_string(), HighlightStyle::Dark)]),
            document: HighlightStyle::Light,
        };
        for spelling in ["bash", "sh", "shell", "zsh", "Bash"] {
            assert_eq!(
                rules.resolve(CODE_ENV, &HashMap::new(), spelling).unwrap(),
                HighlightStyle::Dark,
                "lang={spelling}"
            );
        }
    }

    /// An unknown `style=` is a build failure naming the value and the valid
    /// names — never a warning that silently paints the wrong thing.
    #[test]
    fn unknown_block_style_fails_naming_the_value_and_valid_names() {
        let rules = StyleDefaults {
            by_lang: &HashMap::new(),
            document: HighlightStyle::Light,
        };
        let err = rules
            .resolve(
                CODE_ENV,
                &HashMap::from([("style".into(), "neon".into())]),
                "python",
            )
            .unwrap_err()
            .to_string();
        assert!(err.contains("neon"), "{err}");
        for name in ["light", "light-mono", "dark", "dark-mono"] {
            assert!(err.contains(name), "missing {name}: {err}");
        }
    }

    /// `lstlisting` has no `style=` option, so one there warns as unknown and
    /// the block still honours `by_lang` — the warning path and the resolution
    /// path are independent.
    #[test]
    fn lstlisting_ignores_a_block_style_but_honours_by_lang() {
        let rules = StyleDefaults {
            by_lang: &HashMap::from([("python".to_string(), HighlightStyle::Dark)]),
            document: HighlightStyle::Light,
        };
        let with_style = HashMap::from([("style".to_string(), "dark-mono".to_string())]);
        assert_eq!(
            rules.resolve(LST_ENV, &with_style, "python").unwrap(),
            HighlightStyle::Dark,
            "the block option is not consulted for lstlisting"
        );

        let dir = captioned_fixture(
            "\\documentclass{article}\n\\begin{document}\n\
             \\begin{lstlisting}[language=Python, style=dark]\nx = 1\n\\end{lstlisting}\n\
             \\end{document}\n",
        );
        let cfg = Settings {
            lstlisting: true,
            style: HighlightStyle::Light,
            by_lang: HashMap::from([("python".to_string(), HighlightStyle::Dark)]),
            ..Settings::default()
        };
        // The unknown option is reported by the shared option parser on
        // stderr (it is not a block warning), and by_lang still applies.
        let warnings = run(dir.path(), "main.tex", cfg).unwrap();
        assert!(warnings.is_empty(), "warnings: {warnings:?}");
        let out = std::fs::read_to_string(dir.path().join("main.tex")).unwrap();
        assert!(
            out.contains("\\definecolor{tfxcol22272e}"),
            "by_lang still applies: {out}"
        );
    }

    /// End to end: a dark block paints its own tint, frame, gutter and base
    /// colour under its own colour names, while the fixed `tfxtint` keeps the
    /// light palette — the two coexist in one document.
    #[test]
    fn dark_block_emits_its_own_frame_and_base_colour() {
        let dir = captioned_fixture(
            "\\documentclass{article}\n\\begin{document}\n\
             \\begin{code}[lang=bash, style=dark, numbers=true]\necho hi\n\\end{code}\n\
             \\end{document}\n",
        );
        let warnings = run(dir.path(), "main.tex", Settings::default()).unwrap();
        assert!(warnings.is_empty(), "warnings: {warnings:?}");
        let out = std::fs::read_to_string(dir.path().join("main.tex")).unwrap();

        assert!(
            out.contains("\\definecolor{tfxcol22272e}{rgb}{0.133,0.153,0.180}"),
            "the dark tint must be defined: {out}"
        );
        assert!(
            out.contains("\\definecolor{tfxcol8b949e}{rgb}{0.545,0.580,0.620}"),
            "the dark frame/gutter must be defined: {out}"
        );
        assert!(
            out.contains("\\definecolor{tfxcoladbac7}{rgb}{0.678,0.729,0.780}"),
            "the dark base foreground must be defined: {out}"
        );
        // The light names still exist, at the light palette's values: the
        // preamble is unchanged for a document that mixes the two.
        assert!(
            out.contains("\\definecolor{tfxtint}{rgb}{0.965,0.973,0.980}"),
            "the light tint must stay: {out}"
        );
        assert!(out.contains("\\textcolor{tfxcol22272e}"), "dark tint rule");
        assert!(out.contains("\\textcolor{tfxcol8b949e}"), "dark frame rule");
        assert!(
            out.contains("\\textcolor{tfxcol8b949e}{\\hbox to 2em{\\hss 1}}"),
            "the gutter uses the dark comment colour: {out}"
        );
        assert!(
            out.contains("{\n\\tfxcodestyle\n\\color{tfxcoladbac7}"),
            "the base colour opens the style group: {out}"
        );
    }

    /// A `light-mono` block leans on weight and slant: the keyword is bold,
    /// the comment italic, and nothing carries hue.
    #[test]
    fn light_mono_block_emits_bold_keywords_and_italic_comments() {
        let dir = captioned_fixture(
            "\\documentclass{article}\n\\begin{document}\n\
             \\begin{code}[lang=python, style=light-mono]\n\
             # note\ndef f():\n    return \"x\"\n\\end{code}\n\\end{document}\n",
        );
        let warnings = run(dir.path(), "main.tex", Settings::default()).unwrap();
        assert!(warnings.is_empty(), "warnings: {warnings:?}");
        let out = std::fs::read_to_string(dir.path().join("main.tex")).unwrap();
        assert!(
            out.contains("\\definecolor{tfxcolf6f6f6}"),
            "the mono tint must be defined: {out}"
        );
        assert!(out.contains("\\textit{"), "comments are italic: {out}");
        assert!(out.contains("\\textbf{"), "keywords are bold: {out}");
        // No hue: every defined colour is a grey.
        for line in out.lines().filter(|l| l.contains("\\definecolor{tfxcol")) {
            let rgb = line
                .rsplit_once('{')
                .and_then(|(_, rest)| rest.split_once('}'))
                .expect("an rgb triple");
            let parts: Vec<f64> = rgb
                .0
                .split(',')
                .map(|v| v.parse().unwrap_or(-1.0))
                .collect();
            assert_eq!(parts.len(), 3, "{line}");
            assert!(
                parts.iter().all(|v| *v >= 0.0),
                "light-mono must define no hue-carrying colour: {line}"
            );
            assert!(
                (parts[0] - parts[1]).abs() < 0.002 && (parts[1] - parts[2]).abs() < 0.002,
                "light-mono must be greyscale: {line}"
            );
        }
    }

    /// The `dark` default in `project.toml` reaches every block that names no
    /// language the table covers, and a block may still opt back out.
    #[test]
    fn document_default_style_applies_and_a_block_can_override_it() {
        let dir = captioned_fixture(
            "\\documentclass{article}\n\\begin{document}\n\
             \\begin{code}[lang=python]\nx = 1\n\\end{code}\n\
             \\begin{code}[lang=rust, style=light]\nlet x = 1;\n\\end{code}\n\
             \\end{document}\n",
        );
        let cfg = Settings {
            style: HighlightStyle::Dark,
            ..Settings::default()
        };
        let warnings = run(dir.path(), "main.tex", cfg).unwrap();
        assert!(warnings.is_empty(), "warnings: {warnings:?}");
        let out = std::fs::read_to_string(dir.path().join("main.tex")).unwrap();
        assert_eq!(
            out.matches("\\color{tfxcoladbac7}").count(),
            1,
            "only the block that inherited `dark` gets a base colour: {out}"
        );
        assert_eq!(
            out.matches("\\textcolor{tfxcol22272e}").count(),
            1,
            "one tinted line in the dark block: {out}"
        );
        assert_eq!(
            out.matches("\\textcolor{tfxtint}").count(),
            1,
            "the overriding block keeps the light frame: {out}"
        );
    }

    #[test]
    fn lang_for_reads_the_environments_own_key_and_trims() {
        let code = |v: &str| HashMap::from([("lang".to_string(), v.to_string())]);
        let lst = |v: &str| HashMap::from([("language".to_string(), v.to_string())]);

        assert_eq!(
            lang_for(CODE_ENV, &code("[ISO Rust]")),
            "[ISO Rust]",
            "`code` keeps the author's spelling verbatim"
        );
        assert_eq!(
            lang_for(LST_ENV, &lst("[ISO Rust]Rust")),
            "Rust",
            "`lstlisting` drops the [ISO …] prefix"
        );
        assert_eq!(lang_for(CODE_ENV, &code(" rust ")), "rust");
    }

    #[test]
    fn unknown_language_warning_names_the_environment_it_came_from() {
        let code = unknown_language_warning(CODE_ENV, "brainfuck", 12, "main.tex");
        let lst = unknown_language_warning(LST_ENV, "brainfuck", 12, "main.tex");
        assert!(
            code.message.contains("unknown code language 'brainfuck'"),
            "{}",
            code.message
        );
        assert!(
            lst.message
                .contains("unknown language 'brainfuck' in lstlisting"),
            "{}",
            lst.message
        );
        assert_ne!(code.message, lst.message);
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

    /// A floated block leaves the text flow: its figure opens with `\par` (no
    /// `\medskip` before it) and no `\medskip` sits between `\end{figure}` and
    /// the prose that follows.
    #[test]
    fn floated_block_leaves_no_medskip_directly_after_the_figure() {
        let dir = captioned_fixture(
            "\\documentclass{article}\n\\begin{document}\n\
             \\begin{code}[lang=python, pos=t]\nx = 1\n\\end{code}\nProse.\n\\end{document}\n",
        );
        run(dir.path(), "main.tex", Settings::default()).unwrap();
        let out = std::fs::read_to_string(dir.path().join("main.tex")).unwrap();
        assert!(
            out.contains("\\end{figure}\n\\noindent \nProse."),
            "the prose follows the float with no vertical rhythm: {out}"
        );
        let end = out.find("\\end{figure}").unwrap();
        let prose = out[end..].find("Prose.").unwrap() + end;
        assert!(
            !out[end..prose].contains("\\medskip"),
            "no \\medskip may sit between the figure and the prose: {out}"
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
        // Source line 5 holds `b = 2` (plain block: escaped `b\tfxsp{}=\tfxsp{}2`).
        let build_line = rewritten
            .lines()
            .position(|line| line.contains("b\\tfxsp{}=\\tfxsp{}2"))
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
        // Spawns tectonic: the child inherits the process env, so hold
        // ENV_LOCK or a concurrent HOME swap gives it a cold bundle cache.
        let _env = crate::test_sync::ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if crate::compiler::locate_tectonic().is_none() {
            eprintln!("skipping: tectonic not available in environment");
            return;
        }
        let _tectonic = crate::test_support::tectonic_lock();
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
                // spanish `babel` redefines `\%` to eat the preceding interword
                // glue and insert a thin space, which used to knock every glyph
                // after a `$ %` off the monospace grid; the emitter prints `%`
                // as `\char37{}` instead. The second line compiles `<`/`>`
                // (`\char60{}`/`\char62{}`) under the same spanish shorthands.
                "spanish-percent",
                "\\usepackage[spanish]{babel}\n",
                "\\begin{code}[lang=bash]\necho '$ % & # _ ^ ~ { } \\'\ncat < in.txt > out 2>&1\n\\end{code}\n",
                false,
                "echo",
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
        // Spawns tectonic: the child inherits the process env, so hold
        // ENV_LOCK or a concurrent HOME swap gives it a cold bundle cache.
        let _env = crate::test_sync::ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if crate::compiler::locate_tectonic().is_none() {
            eprintln!("skipping: tectonic not available in environment");
            return;
        }
        let _tectonic = crate::test_support::tectonic_lock();
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

    /// `push_text` advances the source line once per newline and records the
    /// new line as the origin of the output line it just closed.
    #[test]
    fn push_text_advances_one_line_per_newline() {
        let mut result = String::new();
        let mut origins = Vec::new();
        let mut src_line = 5usize;
        push_text(&mut result, &mut origins, "a\nb\nc", &mut src_line);
        assert_eq!(result, "a\nb\nc");
        assert_eq!(src_line, 7);
        assert_eq!(origins, vec![6, 7]);
    }

    /// Without a newline nothing advances: the origin list stays empty.
    #[test]
    fn push_text_without_newline_leaves_the_line_alone() {
        let mut result = String::new();
        let mut origins = Vec::new();
        let mut src_line = 5usize;
        push_text(&mut result, &mut origins, "abc", &mut src_line);
        assert_eq!(result, "abc");
        assert_eq!(src_line, 5);
        assert!(origins.is_empty());
    }

    /// The `\end{code}` line owns the closing stanza: a block ending on
    /// source line 5 maps its trailing `\medskip` there, not line 1.
    /// (The block emission opens with its own `\par\medskip`, so the
    /// trailing one — the last `\medskip` in the file — is asserted.)
    #[test]
    fn end_line_of_a_block_points_at_the_end_tag_line() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("main.tex"),
            "\\documentclass{article}\n\\begin{document}\n\
             \\begin{code}\na = 1\n\\end{code}\n\\end{document}\n",
        )
        .unwrap();
        let map = process(dir.path(), "main.tex", Settings::default()).unwrap();
        let rewritten = std::fs::read_to_string(dir.path().join("main.tex")).unwrap();
        let lines: Vec<&str> = rewritten.lines().collect();
        let medskip = lines
            .iter()
            .rposition(|line| line.contains("\\medskip"))
            .expect("rewritten block must emit \\medskip")
            + 1;
        assert_eq!(
            map.get("main.tex", medskip),
            Some(("main.tex", 5)),
            "the line after the block ends on source line 5"
        );
    }

    /// `strip_block_edges` drops a final whitespace-only line together with
    /// the newline before it — the `\end{code}` tag's indentation the scanner
    /// leaves in the raw body — while keeping code indentation and the
    /// author's interior blank lines.
    #[test]
    fn strip_block_edges_drops_a_trailing_whitespace_only_line() {
        assert_eq!(strip_block_edges("\nx\n  "), "x");
        assert_eq!(strip_block_edges("\nx\n\t"), "x");
        assert_eq!(strip_block_edges("\nx\n"), "x");
        assert_eq!(strip_block_edges("\r\nx\r\n  "), "x");
        assert_eq!(
            strip_block_edges("\nx\n\n  "),
            "x\n",
            "an interior blank line the author wrote survives"
        );
        assert_eq!(
            strip_block_edges("\n  x\n  "),
            "  x",
            "code indentation survives; only the whitespace-only tail goes"
        );
    }

    /// Regression: an indented `\end{code}` used to leave the tag's
    /// indentation in the highlighted body, rendering one extra, empty,
    /// numbered line. A flush and a two-space-indented block with the same
    /// source lines must emit the same listing — gutter lines 1 and 2 only.
    #[test]
    fn indented_end_tag_emits_no_extra_numbered_line() {
        let flush = captioned_fixture(
            "\\documentclass{article}\n\\begin{document}\n\
             \\begin{code}[lang=python, numbers=true]\na = 1\nb = 2\n\\end{code}\n\
             \\end{document}\n",
        );
        let indented = captioned_fixture(
            "\\documentclass{article}\n\\begin{document}\n\
             \\begin{code}[lang=python, numbers=true]\na = 1\nb = 2\n  \\end{code}\n\
             \\end{document}\n",
        );
        run(flush.path(), "main.tex", Settings::default()).unwrap();
        run(indented.path(), "main.tex", Settings::default()).unwrap();
        let flush_out = std::fs::read_to_string(flush.path().join("main.tex")).unwrap();
        let indented_out = std::fs::read_to_string(indented.path().join("main.tex")).unwrap();

        for (name, out) in [("flush", &flush_out), ("indented", &indented_out)] {
            assert!(
                out.contains("\\hbox to 2em{\\hss 1}"),
                "{name} line 1: {out}"
            );
            assert!(
                out.contains("\\hbox to 2em{\\hss 2}"),
                "{name} line 2: {out}"
            );
            assert!(
                !out.contains("\\hbox to 2em{\\hss 3}"),
                "{name}: an indented \\end{{code}} must not add a third, empty numbered line:\n{out}"
            );
        }
        assert_eq!(
            flush_out, indented_out,
            "the indentation of \\end{{code}} must not change the emitted listing"
        );
    }

    /// The line map stays correct when `\end{code}` is indented: the echoed
    /// code line and the closing `\medskip` still map to their original
    /// source lines (5 and 6).
    #[test]
    fn line_map_is_stable_for_an_indented_end_tag() {
        let dir = captioned_fixture(
            "\\documentclass{article}\n\\begin{document}\n\
             \\begin{code}\na = 1\nb = 2\n  \\end{code}\n\\end{document}\n",
        );
        let map = process(dir.path(), "main.tex", Settings::default()).unwrap();
        let rewritten = std::fs::read_to_string(dir.path().join("main.tex")).unwrap();

        let build_line = rewritten
            .lines()
            .position(|line| line.contains("b\\tfxsp{}=\\tfxsp{}2"))
            .expect("the second code line must be in the build copy")
            + 1;
        assert_eq!(
            map.get("main.tex", build_line),
            Some(("main.tex", 5)),
            "`b = 2` is source line 5"
        );

        let lines: Vec<&str> = rewritten.lines().collect();
        let medskip = lines
            .iter()
            .rposition(|line| line.contains("\\medskip"))
            .expect("rewritten block must emit \\medskip")
            + 1;
        assert_eq!(
            map.get("main.tex", medskip),
            Some(("main.tex", 6)),
            "the indented \\end{{code}} still ends the block on source line 6"
        );
    }

    /// Write a one-block document into a temp build dir.
    fn captioned_fixture(main: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("main.tex"), main).unwrap();
        dir
    }

    #[test]
    fn code_accepts_caption_label_pos_size_without_unknown_option_warnings() {
        let dir = captioned_fixture(
            "\\documentclass{article}\n\\begin{document}\n\
             \\begin{code}[lang=python, caption={Hi}, label={lst:hi}, pos=t, size=footnotesize]\n\
             x = 1\n\\end{code}\n\\end{document}\n",
        );
        let warnings = run(dir.path(), "main.tex", Settings::default()).unwrap();
        assert!(warnings.is_empty(), "warnings: {warnings:?}");
        let out = std::fs::read_to_string(dir.path().join("main.tex")).unwrap();
        assert!(out.contains("\\refstepcounter{tfxlisting}"), "{out}");
        assert!(out.contains("\\begin{figure}[t]"), "{out}");
        assert!(out.contains("\\footnotesize"), "{out}");
    }

    /// `pos=` floats the block on its own: no caption, no counter, no
    /// list entry — but still a float.
    #[test]
    fn pos_without_a_caption_still_floats_the_block() {
        let dir = captioned_fixture(
            "\\documentclass{article}\n\\begin{document}\n\
             \\begin{code}[lang=python, pos=t]\nx = 1\n\\end{code}\n\\end{document}\n",
        );
        let warnings = run(dir.path(), "main.tex", Settings::default()).unwrap();
        assert!(warnings.is_empty(), "warnings: {warnings:?}");
        let out = std::fs::read_to_string(dir.path().join("main.tex")).unwrap();
        assert!(out.contains("\\begin{figure}[t]"), "{out}");
        assert!(out.contains("\\end{figure}"), "{out}");
        assert!(
            !out.contains("tfxlisting"),
            "no caption machinery is injected: {out}"
        );
    }

    /// `pos=H` is the default and means inline.
    #[test]
    fn pos_h_renders_inline_like_the_default() {
        let dir = captioned_fixture(
            "\\documentclass{article}\n\\begin{document}\n\
             \\begin{code}[lang=python, caption={Hi}, pos=H]\nx = 1\n\\end{code}\n\\end{document}\n",
        );
        run(dir.path(), "main.tex", Settings::default()).unwrap();
        let out = std::fs::read_to_string(dir.path().join("main.tex")).unwrap();
        assert!(!out.contains("figure"), "{out}");
        assert!(
            out.contains("\\vadjust{\\penalty10000\\kern\\medskipamount}"),
            "{out}"
        );
    }

    #[test]
    fn label_without_caption_warns_with_line_and_is_ignored() {
        let dir = captioned_fixture(
            "\\documentclass{article}\n\\begin{document}\n\
             \\begin{code}[lang=python, label={lst:lonely}]\nx = 1\n\\end{code}\n\\end{document}\n",
        );
        let warnings = run(dir.path(), "main.tex", Settings::default()).unwrap();
        assert_eq!(warnings.len(), 1, "warnings: {warnings:?}");
        assert!(
            warnings[0]
                .message
                .contains("label without caption is ignored"),
            "{warnings:?}"
        );
        assert_eq!(warnings[0].line, 3, "{warnings:?}");
        let out = std::fs::read_to_string(dir.path().join("main.tex")).unwrap();
        assert!(!out.contains("\\refstepcounter"), "{out}");
        assert!(!out.contains("\\label{lst:lonely}"), "{out}");
    }

    #[test]
    fn unknown_size_warns_lists_valid_values_and_uses_small() {
        let dir = captioned_fixture(
            "\\documentclass{article}\n\\begin{document}\n\
             \\begin{code}[lang=python, caption={Hi}, size=huge]\nx = 1\n\\end{code}\n\\end{document}\n",
        );
        let warnings = run(dir.path(), "main.tex", Settings::default()).unwrap();
        assert_eq!(warnings.len(), 1, "warnings: {warnings:?}");
        assert!(warnings[0].message.contains("huge"), "{warnings:?}");
        assert!(
            warnings[0]
                .message
                .contains("scriptsize, footnotesize, small, normalsize"),
            "{warnings:?}"
        );
        let out = std::fs::read_to_string(dir.path().join("main.tex")).unwrap();
        assert!(!out.contains("\\huge"), "{out}");
        assert!(out.contains("\\tfxcodestyle\n\n\\noindent"), "{out}");
    }

    #[test]
    fn unknown_placement_warns_and_renders_inline() {
        let dir = captioned_fixture(
            "\\documentclass{article}\n\\begin{document}\n\
             \\begin{code}[lang=python, caption={Hi}, pos=Z]\nx = 1\n\\end{code}\n\\end{document}\n",
        );
        let warnings = run(dir.path(), "main.tex", Settings::default()).unwrap();
        assert_eq!(warnings.len(), 1, "warnings: {warnings:?}");
        assert!(
            warnings[0].message.contains("unknown placement"),
            "{warnings:?}"
        );
        assert!(warnings[0].message.contains('Z'), "{warnings:?}");
        let out = std::fs::read_to_string(dir.path().join("main.tex")).unwrap();
        assert!(!out.contains("\\begin{figure}"), "{out}");
        assert!(out.contains("\\refstepcounter{tfxlisting}"), "{out}");
    }

    #[test]
    fn caption_emits_counter_label_and_lol_entry() {
        let dir = captioned_fixture(
            "\\documentclass{article}\n\\begin{document}\n\
             \\begin{code}[lang=python, caption={Fibonacci}, label={lst:fib}]\n\
             x = 1\n\\end{code}\n\\end{document}\n",
        );
        run(dir.path(), "main.tex", Settings::default()).unwrap();
        let out = std::fs::read_to_string(dir.path().join("main.tex")).unwrap();
        assert!(
            out.contains("\\refstepcounter{tfxlisting}\\label{lst:fib}%"),
            "{out}"
        );
        assert!(out.contains("\\tfxlistingname~\\thetfxlisting:"), "{out}");
        assert!(
            out.contains("\\vadjust{\\penalty10000\\kern\\medskipamount}\\par"),
            "{out}"
        );
        assert!(out.contains("\\addcontentsline{lol}{listing}"), "{out}");
        assert!(out.contains("\\listoflistings"), "{out}");
        assert!(out.contains("\\newcounter{tfxlisting}"), "{out}");
    }

    /// A caption with a comma needs braces, like every option value: the
    /// shared parser keeps the braced text whole and the block keeps its
    /// other options.
    #[test]
    fn caption_with_a_comma_survives_the_brace_parser() {
        let dir = captioned_fixture(
            "\\documentclass{article}\n\\begin{document}\n\
             \\begin{code}[lang=python, caption={Pila, versión 2}, label={lst:pila}, size=scriptsize]\n\
             x = 1\n\\end{code}\n\\end{document}\n",
        );
        let warnings = run(dir.path(), "main.tex", Settings::default()).unwrap();
        assert!(warnings.is_empty(), "warnings: {warnings:?}");
        let out = std::fs::read_to_string(dir.path().join("main.tex")).unwrap();
        assert!(out.contains("Pila, versión 2"), "{out}");
        assert!(out.contains("\\label{lst:pila}"), "{out}");
        assert!(out.contains("\\scriptsize"), "{out}");
    }

    #[test]
    fn float_too_tall_falls_back_inline_with_warning() {
        let mut body = String::new();
        for i in 1..=500 {
            body.push_str(&format!("code line {i}\n"));
        }
        let main = format!(
            "\\documentclass{{article}}\n\\begin{{document}}\n\
             \\begin{{code}}[lang=python, caption={{Tall}}, pos=t]\n{body}\\end{{code}}\n\\end{{document}}\n"
        );
        let dir = captioned_fixture(&main);
        let warnings = run(dir.path(), "main.tex", Settings::default()).unwrap();
        assert!(
            warnings
                .iter()
                .any(|w| w.message.contains("too tall to fit on one page")),
            "{warnings:?}"
        );
        let out = std::fs::read_to_string(dir.path().join("main.tex")).unwrap();
        assert!(!out.contains("\\begin{figure}"), "{out}");
        assert!(out.contains("\\refstepcounter{tfxlisting}"), "{out}");
    }

    #[test]
    fn lstlisting_maps_caption_label_float_and_basicstyle() {
        let dir = captioned_fixture(
            "\\documentclass{article}\n\\begin{document}\n\
             \\begin{lstlisting}[language=Python, caption={Hi}, label={lst:hi}, float=t, basicstyle=\\footnotesize]\n\
             x = 1\n\\end{lstlisting}\n\\end{document}\n",
        );
        let cfg = Settings {
            lstlisting: true,
            ..Settings::default()
        };
        let warnings = run(dir.path(), "main.tex", cfg).unwrap();
        assert!(warnings.is_empty(), "warnings: {warnings:?}");
        let out = std::fs::read_to_string(dir.path().join("main.tex")).unwrap();
        assert!(
            out.contains("\\refstepcounter{tfxlisting}\\label{lst:hi}%"),
            "{out}"
        );
        assert!(out.contains("\\begin{figure}[t]"), "{out}");
        assert!(out.contains("\\footnotesize"), "{out}");
    }

    #[test]
    fn lstlisting_basicstyle_size_does_not_false_match_smallskip() {
        let dir = captioned_fixture(
            "\\documentclass{article}\n\\begin{document}\n\
             \\begin{lstlisting}[language=Python, caption={Hi}, basicstyle=\\smallskip]\n\
             x = 1\n\\end{lstlisting}\n\\end{document}\n",
        );
        let cfg = Settings {
            lstlisting: true,
            ..Settings::default()
        };
        run(dir.path(), "main.tex", cfg).unwrap();
        let out = std::fs::read_to_string(dir.path().join("main.tex")).unwrap();
        assert!(!out.contains("\\small\n"), "{out}");
        assert!(out.contains("\\refstepcounter{tfxlisting}"), "{out}");
    }

    #[test]
    fn captioned_document_uses_spanish_names() {
        let dir = captioned_fixture(
            "\\documentclass{article}\n\\usepackage[spanish]{babel}\n\\begin{document}\n\
             \\begin{code}[lang=python, caption={Hola}]\nx = 1\n\\end{code}\n\\end{document}\n",
        );
        run(dir.path(), "main.tex", Settings::default()).unwrap();
        let out = std::fs::read_to_string(dir.path().join("main.tex")).unwrap();
        assert!(
            out.contains("\\newcommand{\\tfxlistingname}{Listado}"),
            "{out}"
        );
        assert!(
            out.contains("\\newcommand{\\tfxlistname}{Índice de listados}"),
            "{out}"
        );
    }

    #[test]
    fn fallback_language_selects_names_without_babel() {
        let dir = captioned_fixture(
            "\\documentclass{article}\n\\begin{document}\n\
             \\begin{code}[lang=python, caption={Hola}]\nx = 1\n\\end{code}\n\\end{document}\n",
        );
        let cfg = Settings {
            fallback_language: Some("spanish".to_string()),
            ..Settings::default()
        };
        run(dir.path(), "main.tex", cfg).unwrap();
        let out = std::fs::read_to_string(dir.path().join("main.tex")).unwrap();
        assert!(
            out.contains("\\newcommand{\\tfxlistingname}{Listado}"),
            "{out}"
        );
    }

    #[test]
    fn plain_blocks_keep_the_old_golden_output() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("main.tex"),
            "\\documentclass{article}\n\\begin{document}\n\
             \\begin{code}[lang=rust]\nfn main() {}\n\\end{code}\n\\end{document}\n",
        )
        .unwrap();
        run(dir.path(), "main.tex", Settings::default()).unwrap();
        let out = std::fs::read_to_string(dir.path().join("main.tex")).unwrap();
        assert!(!out.contains("tfxlisting"), "{out}");
        assert!(!out.contains("listoflistings"), "{out}");
        assert!(!out.contains("\\begin{figure}"), "{out}");
    }

    /// End-to-end: a captioned listing with `\ref` and `\listoflistings`
    /// compiles, and the PDF text carries the caption, the reference number
    /// and the list entry. Skips (never fails) without Tectonic.
    #[test]
    fn captioned_listing_pdf_has_caption_reference_and_list() {
        let _env = crate::test_sync::ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if crate::compiler::locate_tectonic().is_none() {
            eprintln!("skipping: tectonic not available in environment");
            return;
        }
        let _tectonic = crate::test_support::tectonic_lock();
        let dir = tempfile::tempdir().unwrap();
        let main = "\\documentclass{article}\n\\begin{document}\n\
             \\listoflistings\n\
             See Listing~\\ref{lst:fib}.\n\
             \\begin{code}[lang=python, caption={Fibonacci numbers}, label={lst:fib}]\n\
             def fib(n):\n    return n\n\\end{code}\n\\end{document}\n";
        std::fs::write(dir.path().join("main.tex"), main).unwrap();
        run(dir.path(), "main.tex", Settings::default()).unwrap();
        crate::compiler::compile(
            dir.path(),
            "main.tex",
            false,
            None,
            &crate::highlight::LineMap::default(),
        )
        .expect("tectonic failed");
        let text = crate::pdftext::extract_text(&dir.path().join("main.pdf")).unwrap();
        assert!(text.contains("Listing 1:"), "caption missing: {text:?}");
        assert!(
            text.contains("List of Listings"),
            "list heading missing: {text:?}"
        );
        assert!(
            text.matches("Fibonacci numbers").count() >= 2,
            "caption and list entry expected: {text:?}"
        );
    }

    /// A `report` class numbers listings per chapter: `Listing 1.1` with a
    /// `\ref` resolving to `1.1`. Skips (never fails) without Tectonic.
    #[test]
    fn chapter_class_numbers_listings_per_chapter() {
        let _env = crate::test_sync::ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if crate::compiler::locate_tectonic().is_none() {
            eprintln!("skipping: tectonic not available in environment");
            return;
        }
        let _tectonic = crate::test_support::tectonic_lock();
        let dir = tempfile::tempdir().unwrap();
        let main = "\\documentclass{report}\n\\begin{document}\n\
             \\chapter{First}\n\
             See \\ref{lst:one}.\n\
             \\begin{code}[lang=python, caption={One}, label={lst:one}]\n\
             x = 1\n\\end{code}\n\\end{document}\n";
        std::fs::write(dir.path().join("main.tex"), main).unwrap();
        run(dir.path(), "main.tex", Settings::default()).unwrap();
        crate::compiler::compile(
            dir.path(),
            "main.tex",
            false,
            None,
            &crate::highlight::LineMap::default(),
        )
        .expect("tectonic failed");
        let text = crate::pdftext::extract_text(&dir.path().join("main.pdf")).unwrap();
        assert!(
            text.contains("Listing 1.1:"),
            "chapter numbering missing: {text:?}"
        );
    }

    /// Font setting reaches the injected preamble exactly once.
    #[test]
    fn font_setting_reaches_the_injected_preamble_once() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("main.tex"),
            "\\documentclass{article}\n\\begin{document}\n\
             \\begin{code}[lang=rust]\nfn main() {}\n\\end{code}\n\\end{document}\n",
        )
        .unwrap();

        // With Inconsolata font, the load line and its guard must appear once.
        let cfg = Settings {
            font: ListingFont::Inconsolata,
            ..Settings::default()
        };
        run(dir.path(), "main.tex", cfg).unwrap();
        let out = std::fs::read_to_string(dir.path().join("main.tex")).unwrap();
        let guard = r"\@ifpackageloaded{inconsolata}{}{\usepackage[varqu,varl]{inconsolata}}%";
        assert_eq!(out.matches(guard).count(), 1, "guard appears once:\n{out}");
        assert_eq!(
            out.matches(r"\usepackage[varqu,varl]{inconsolata}").count(),
            1,
            "load line appears once:\n{out}"
        );

        // With default Document font, no font line is injected (existing
        // snapshots pin byte-identity, so this is a sanity check).
        let dir2 = tempfile::tempdir().unwrap();
        std::fs::write(
            dir2.path().join("main.tex"),
            "\\documentclass{article}\n\\begin{document}\n\
             \\begin{code}[lang=rust]\nfn main() {}\n\\end{code}\n\\end{document}\n",
        )
        .unwrap();
        run(dir2.path(), "main.tex", Settings::default()).unwrap();
        let out2 = std::fs::read_to_string(dir2.path().join("main.tex")).unwrap();
        assert!(
            !out2.contains("inconsolata"),
            "Document font injects nothing:\n{out2}"
        );
        assert!(
            !out2.contains("sourcecodepro"),
            "Document font injects nothing:\n{out2}"
        );
        assert!(
            !out2.contains("DejaVuSansMono"),
            "Document font injects nothing:\n{out2}"
        );
        assert!(
            !out2.contains("plex-mono"),
            "Document font injects nothing:\n{out2}"
        );
        assert!(
            !out2.contains("FiraMono"),
            "Document font injects nothing:\n{out2}"
        );
    }

    /// The explicit `font = "document"` value and an absent key produce
    /// byte-identical rewrites — the same guarantee the golden snapshot pins
    /// for the default, here compared directly against each other.
    #[test]
    fn explicit_document_font_is_byte_identical_to_the_default() {
        let doc = "\\documentclass{article}\n\\begin{document}\n\
                   \\section{Demo}\n\
                   \\begin{code}[lang=python, numbers=true]\n\
                   def fib(n):\n    return n\n\
                   \\end{code}\n\
                   Prose after the block.\n\\end{document}\n";

        let absent = tempfile::tempdir().unwrap();
        std::fs::write(absent.path().join("main.tex"), doc).unwrap();
        run(absent.path(), "main.tex", Settings::default()).unwrap();

        let explicit = tempfile::tempdir().unwrap();
        std::fs::write(explicit.path().join("main.tex"), doc).unwrap();
        let cfg = Settings {
            font: ListingFont::Document,
            ..Settings::default()
        };
        run(explicit.path(), "main.tex", cfg).unwrap();

        let absent_bytes = std::fs::read(absent.path().join("main.tex")).unwrap();
        let explicit_bytes = std::fs::read(explicit.path().join("main.tex")).unwrap();
        // Sanity first: the pass really rewrote and injected, so the two
        // files compared below are not both untouched copies of the source.
        let absent_text = String::from_utf8(absent_bytes.clone()).unwrap();
        assert!(
            absent_text.contains("texforge code listings"),
            "fixture must contain a rewritten block:\n{absent_text}"
        );
        assert_eq!(
            absent_bytes, explicit_bytes,
            "font = \"document\" must be byte-identical to an absent key"
        );
    }
}
