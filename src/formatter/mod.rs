//! LaTeX + BibTeX code formatter.
//!
//! Opinionated, deterministic formatter inspired by `rustfmt` — one canonical
//! output regardless of input style. It re-indents by nesting level
//! (environments *and* unbalanced braces), trims trailing whitespace, and
//! collapses runs of blank lines. It never reflows paragraph text or touches
//! the contents of verbatim-like environments.

const INDENT: &str = "  ";

/// Environments whose content must be passed through untouched.
const VERBATIM_ENVS: &[&str] = &["verbatim", "lstlisting", "minted", "Verbatim"];

/// Format LaTeX source code with consistent style.
pub fn format(source: &str) -> String {
    let mut output: Vec<String> = Vec::new();
    let mut depth: usize = 0;
    let mut prev_blank = false;
    let mut verbatim: Option<String> = None;

    for line in source.lines() {
        // Inside a verbatim environment: emit raw until the matching \end.
        if let Some(ref env) = verbatim {
            let end_tag = format!("\\end{{{}}}", env);
            if line.trim_start().starts_with(&end_tag) {
                depth = depth.saturating_sub(1);
                output.push(indent_line(depth, line.trim()));
                verbatim = None;
            } else {
                output.push(line.trim_end().to_string());
            }
            continue;
        }

        let trimmed = line.trim();

        // Collapse runs of blank lines into a single blank line.
        if trimmed.is_empty() {
            if !prev_blank && !output.is_empty() {
                output.push(String::new());
            }
            prev_blank = true;
            continue;
        }
        prev_blank = false;

        // Lines that *open* with a closer dedent before being placed, so the
        // closer aligns with whatever opened it.
        let display_depth = depth.saturating_sub(leading_dedent(trimmed));
        output.push(indent_line(display_depth, trimmed));

        // A verbatim opener freezes formatting until its \end.
        if let Some(env) = verbatim_opener(trimmed) {
            depth += 1;
            verbatim = Some(env);
            continue;
        }

        // Apply this line's net nesting change to the running depth.
        let delta = nesting_delta(trimmed);
        depth = (depth as i32 + delta).max(0) as usize;
    }

    // Drop trailing blank lines, guarantee a single final newline.
    while output.last().is_some_and(|l| l.is_empty()) {
        output.pop();
    }
    let mut result = output.join("\n");
    result.push('\n');
    result
}

fn indent_line(depth: usize, content: &str) -> String {
    if content.is_empty() {
        String::new()
    } else {
        format!("{}{}", INDENT.repeat(depth), content)
    }
}

/// How many levels a line should dedent *before* being placed, based on the
/// closers it opens with (`\end{...}` or a run of leading `}` / `]`).
fn leading_dedent(line: &str) -> usize {
    if line.starts_with("\\end{") {
        return 1;
    }
    line.chars().take_while(|&c| c == '}' || c == ']').count()
}

/// Net nesting change a line contributes: `\begin`/`\end` plus the balance of
/// unescaped braces and brackets (comments and escaped delimiters ignored).
fn nesting_delta(line: &str) -> i32 {
    let mut delta = count_occurrences(line, "\\begin{") as i32;
    delta -= count_occurrences(line, "\\end{") as i32;

    let mut escaped = false;
    for ch in line.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        match ch {
            '\\' => escaped = true,
            '%' => break, // rest of the line is a comment
            '{' | '[' => delta += 1,
            '}' | ']' => delta -= 1,
            _ => {}
        }
    }
    delta
}

/// If the line opens a verbatim-like environment, return its name.
fn verbatim_opener(line: &str) -> Option<String> {
    if !line.starts_with("\\begin{") {
        return None;
    }
    let env = extract_env_name(line)?;
    VERBATIM_ENVS.contains(&env.as_str()).then_some(env)
}

fn count_occurrences(haystack: &str, needle: &str) -> usize {
    haystack.matches(needle).count()
}

/// Extract environment name from `\begin{envname}`.
fn extract_env_name(line: &str) -> Option<String> {
    let start = line.find("\\begin{")? + 7;
    let end = line[start..].find('}')?;
    Some(line[start..start + end].to_string())
}

// ---------------------------------------------------------------------------
// BibTeX formatter
// ---------------------------------------------------------------------------

/// Format a `.bib` file: one field per line, two-space indent, aligned `=`,
/// lowercased entry types and field names, trailing comma after every field.
///
/// Conservative by design — if the input can't be parsed cleanly (junk outside
/// entries, unbalanced delimiters, …) the original source is returned untouched
/// rather than risk corrupting references.
pub fn format_bib(source: &str) -> String {
    match try_format_bib(source) {
        Some(formatted) => formatted,
        None => source.to_string(),
    }
}

struct BibEntry {
    kind: String,
    key: Option<String>,
    fields: Vec<(String, String)>,
}

fn try_format_bib(source: &str) -> Option<String> {
    let chars: Vec<char> = source.chars().collect();
    let n = chars.len();
    let mut i = 0;
    let mut entries = Vec::new();

    while i < n {
        while i < n && chars[i].is_whitespace() {
            i += 1;
        }
        if i >= n {
            break;
        }
        // Any non-whitespace outside an entry is content we don't understand.
        if chars[i] != '@' {
            return None;
        }
        i += 1;

        let kind_start = i;
        while i < n && chars[i].is_alphanumeric() {
            i += 1;
        }
        let kind: String = chars[kind_start..i].iter().collect();
        if kind.is_empty() {
            return None;
        }

        while i < n && chars[i].is_whitespace() {
            i += 1;
        }
        if i >= n {
            return None;
        }
        let open = match chars[i] {
            '{' => '{',
            '(' => '(',
            _ => return None,
        };
        i += 1;

        let body_start = i;
        let mut brace_depth = 0i32;
        let mut in_quote = false;
        loop {
            if i >= n {
                return None; // unterminated entry
            }
            let c = chars[i];
            if in_quote {
                if c == '"' {
                    in_quote = false;
                }
            } else {
                match c {
                    '"' => in_quote = true,
                    '{' => brace_depth += 1,
                    '}' => {
                        if open == '{' && brace_depth == 0 {
                            break;
                        }
                        brace_depth -= 1;
                    }
                    ')' if open == '(' && brace_depth == 0 => break,
                    _ => {}
                }
            }
            i += 1;
        }
        let body: String = chars[body_start..i].iter().collect();
        i += 1; // consume closing delimiter

        entries.push(parse_bib_body(&kind, &body)?);
    }

    if entries.is_empty() {
        return None;
    }

    let mut blocks: Vec<String> = Vec::new();
    for entry in &entries {
        blocks.push(render_bib_entry(entry));
    }
    let mut out = blocks.join("\n\n");
    out.push('\n');
    Some(out)
}

fn parse_bib_body(kind: &str, body: &str) -> Option<BibEntry> {
    let segments = split_top_level(body);
    let kind_l = kind.to_lowercase();

    // @string / @preamble have no citation key; everything is a field/value.
    let key_less = matches!(kind_l.as_str(), "string" | "preamble");

    let mut iter = segments.into_iter();
    let mut key = None;
    if !key_less {
        let first = iter.next()?.trim().to_string();
        if first.is_empty() || first.contains('=') {
            return None;
        }
        key = Some(first);
    }

    let mut fields = Vec::new();
    for seg in iter {
        let seg = seg.trim();
        if seg.is_empty() {
            continue; // tolerate trailing comma
        }
        let eq = top_level_eq(seg)?;
        let name = seg[..eq].trim().to_lowercase();
        let value = collapse_ws(seg[eq + 1..].trim());
        if name.is_empty() {
            return None;
        }
        fields.push((name, value));
    }

    Some(BibEntry {
        kind: kind_l,
        key,
        fields,
    })
}

fn render_bib_entry(entry: &BibEntry) -> String {
    let mut s = String::new();
    s.push('@');
    s.push_str(&entry.kind);
    s.push('{');

    let width = entry.fields.iter().map(|(n, _)| n.len()).max().unwrap_or(0);

    match &entry.key {
        Some(key) => s.push_str(key),
        None => {
            // key-less (@string/@preamble): keep the opening brace tight.
            if let Some((name, value)) = entry.fields.first() {
                s.push_str(&format!("{} = {}", name, value));
            }
            s.push('}');
            return s;
        }
    }

    for (name, value) in &entry.fields {
        s.push_str(",\n");
        s.push_str(INDENT);
        s.push_str(&format!("{:width$} = {}", name, value, width = width));
    }
    if !entry.fields.is_empty() {
        s.push(',');
    }
    s.push('\n');
    s.push('}');
    s
}

/// Split on top-level commas (ignoring commas inside braces or quotes).
fn split_top_level(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut depth = 0i32;
    let mut in_quote = false;
    for c in body.chars() {
        match c {
            '"' if depth == 0 => {
                in_quote = !in_quote;
                cur.push(c);
            }
            '{' if !in_quote => {
                depth += 1;
                cur.push(c);
            }
            '}' if !in_quote => {
                depth -= 1;
                cur.push(c);
            }
            ',' if depth == 0 && !in_quote => {
                out.push(std::mem::take(&mut cur));
            }
            _ => cur.push(c),
        }
    }
    out.push(cur);
    out
}

/// Index of the first top-level `=` in a `name = value` segment.
fn top_level_eq(seg: &str) -> Option<usize> {
    let mut depth = 0i32;
    let mut in_quote = false;
    for (idx, c) in seg.char_indices() {
        match c {
            '"' if depth == 0 => in_quote = !in_quote,
            '{' if !in_quote => depth += 1,
            '}' if !in_quote => depth -= 1,
            '=' if depth == 0 && !in_quote => return Some(idx),
            _ => {}
        }
    }
    None
}

/// Collapse internal whitespace runs (including newlines) into single spaces.
fn collapse_ws(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_input_returns_single_newline() {
        assert_eq!(format(""), "\n");
    }

    #[test]
    fn trailing_newline_always_present() {
        let out = format("hello");
        assert!(out.ends_with('\n'));
    }

    #[test]
    fn indentation_inside_environment() {
        let src = "\\begin{document}\nhello\n\\end{document}";
        let out = format(src);
        assert_eq!(out, "\\begin{document}\n  hello\n\\end{document}\n");
    }

    #[test]
    fn nested_environments_indent_begin_and_end() {
        let src = "\\begin{a}\n\\begin{b}\nx\n\\end{b}\n\\end{a}";
        let out = format(src);
        assert_eq!(
            out,
            "\\begin{a}\n  \\begin{b}\n    x\n  \\end{b}\n\\end{a}\n"
        );
    }

    #[test]
    fn multiline_brace_argument_indents() {
        let src = "\\hypersetup{\npdftitle={X},\ncolorlinks\n}";
        let out = format(src);
        assert_eq!(out, "\\hypersetup{\n  pdftitle={X},\n  colorlinks\n}\n");
    }

    #[test]
    fn escaped_braces_do_not_affect_depth() {
        let src = "\\begin{document}\na \\{ b \\} c\nd\n\\end{document}";
        let out = format(src);
        assert_eq!(
            out,
            "\\begin{document}\n  a \\{ b \\} c\n  d\n\\end{document}\n"
        );
    }

    #[test]
    fn comment_braces_ignored() {
        let src = "\\begin{document}\nx % this { is not counted\ny\n\\end{document}";
        let out = format(src);
        assert_eq!(
            out,
            "\\begin{document}\n  x % this { is not counted\n  y\n\\end{document}\n"
        );
    }

    #[test]
    fn multiple_blank_lines_collapsed() {
        let src = "a\n\n\n\nb";
        let out = format(src);
        assert_eq!(out, "a\n\nb\n");
    }

    #[test]
    fn verbatim_content_preserved() {
        let src = "\\begin{verbatim}\n  raw   content\n\\end{verbatim}";
        let out = format(src);
        assert_eq!(out, "\\begin{verbatim}\n  raw   content\n\\end{verbatim}\n");
    }

    #[test]
    fn bib_entry_formatted_and_aligned() {
        let src = "@Article{key, author={A. B.},title = {T},year=2020}";
        let out = format_bib(src);
        assert_eq!(
            out,
            "@article{key,\n  author = {A. B.},\n  title  = {T},\n  year   = 2020,\n}\n"
        );
    }

    #[test]
    fn bib_multiline_value_collapsed() {
        let src = "@book{k,\n  title = {A\n     long    title},\n}";
        let out = format_bib(src);
        assert_eq!(out, "@book{k,\n  title = {A long title},\n}\n");
    }

    #[test]
    fn bib_invalid_left_untouched() {
        let src = "not a bib file at all";
        assert_eq!(format_bib(src), src);
    }

    #[test]
    fn bib_braced_comma_not_split() {
        let src = "@misc{k, title={Hello, World}}";
        let out = format_bib(src);
        assert_eq!(out, "@misc{k,\n  title = {Hello, World},\n}\n");
    }

    #[test]
    fn lstlisting_content_preserved() {
        let src = "\\begin{lstlisting}\n  code here\n\\end{lstlisting}";
        let out = format(src);
        assert_eq!(out, "\\begin{lstlisting}\n  code here\n\\end{lstlisting}\n");
    }

    #[test]
    fn minted_content_preserved() {
        let src = "\\begin{minted}\n  raw\n\\end{minted}";
        let out = format(src);
        assert_eq!(out, "\\begin{minted}\n  raw\n\\end{minted}\n");
    }

    #[test]
    fn leading_dedent_end() {
        assert_eq!(leading_dedent("\\end{doc}"), 1);
    }

    #[test]
    fn leading_dedent_braces() {
        assert_eq!(leading_dedent("}}"), 2);
    }

    #[test]
    fn leading_dedent_none() {
        assert_eq!(leading_dedent("hello"), 0);
    }

    #[test]
    fn nesting_delta_begin() {
        assert_eq!(nesting_delta("\\begin{doc}"), 1);
    }

    #[test]
    fn nesting_delta_end() {
        assert_eq!(nesting_delta("\\end{doc}"), -1);
    }

    #[test]
    fn nesting_delta_braces() {
        assert_eq!(nesting_delta("{ a }"), 0);
    }

    #[test]
    fn nesting_delta_comment_ignored() {
        assert_eq!(nesting_delta("x % { ignore"), 0);
    }

    #[test]
    fn nesting_delta_escaped_braces() {
        assert_eq!(nesting_delta("\\{ \\}"), 0);
    }

    #[test]
    fn extract_env_name_simple() {
        assert_eq!(
            extract_env_name("\\begin{figure}"),
            Some("figure".to_string())
        );
    }

    #[test]
    fn extract_env_name_no_begin() {
        assert_eq!(extract_env_name("no begin here"), None);
    }

    #[test]
    fn count_occurrences_basic() {
        assert_eq!(count_occurrences("abcabc", "abc"), 2);
    }

    #[test]
    fn count_occurrences_none() {
        assert_eq!(count_occurrences("hello", "xyz"), 0);
    }

    #[test]
    fn split_top_level_simple() {
        let result = split_top_level("a, b, c");
        assert_eq!(result, vec!["a", " b", " c"]);
    }

    #[test]
    fn split_top_level_braced_comma() {
        let result = split_top_level("a={x,y}, b");
        assert_eq!(result, vec!["a={x,y}", " b"]);
    }

    #[test]
    fn split_top_level_quoted_comma() {
        let result = split_top_level("\"a,b\", c");
        assert_eq!(result, vec!["\"a,b\"", " c"]);
    }

    #[test]
    fn top_level_eq_found() {
        assert_eq!(top_level_eq("name = value"), Some(5));
    }

    #[test]
    fn top_level_eq_in_braces() {
        assert_eq!(top_level_eq("{name = value}"), None);
    }

    #[test]
    fn collapse_ws_basic() {
        assert_eq!(collapse_ws("  hello   world  "), "hello world");
    }

    #[test]
    fn collapse_ws_newlines() {
        assert_eq!(collapse_ws("a\n\nb"), "a b");
    }

    #[test]
    fn format_empty_lines_only() {
        let out = format("\n\n\n");
        assert_eq!(out, "\n");
    }

    #[test]
    fn format_single_line() {
        let out = format("hello");
        assert_eq!(out, "hello\n");
    }

    #[test]
    fn format_mixed_content() {
        let src = "\\begin{document}\n\\begin{figure}\nimg\n\\end{figure}\n\\end{document}";
        let out = format(src);
        assert!(out.contains("  \\begin{figure}"));
        assert!(out.contains("    img"));
        assert!(out.contains("  \\end{figure}"));
    }

    #[test]
    fn bib_string_entry() {
        let src = "@string{jabref = {Journal of Things}}";
        let out = format_bib(src);
        assert!(out.contains("@string"));
    }

    #[test]
    fn bib_paren_delimiters() {
        let src = "@article(key, author={A. B.})";
        let out = format_bib(src);
        assert!(out.contains("@article"));
    }

    #[test]
    fn bib_unterminated_returns_original() {
        let src = "@article{key, author={A. B.}";
        assert_eq!(format_bib(src), src);
    }

    #[test]
    fn bib_empty_returns_original() {
        assert_eq!(format_bib(""), "");
    }

    #[test]
    fn format_trailing_whitespace_trimmed() {
        let src = "hello   \nworld";
        let out = format(src);
        assert!(!out.contains("   \n"));
    }

    #[test]
    fn format_comment_not_counted() {
        let src = "\\begin{doc}\nx % { unmatched\n\\end{doc}";
        let out = format(src);
        assert_eq!(out, "\\begin{doc}\n  x % { unmatched\n\\end{doc}\n");
    }
}
