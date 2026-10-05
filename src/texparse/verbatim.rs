//! Verbatim regions located in the source, for the highlighter and linter.

use std::collections::HashSet;

use super::{tokenize_with_spans, Token};

/// A verbatim region as located in the source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerbatimBlock {
    /// The environment's name (`verbatim`, `lstlisting`, `code`, …).
    pub env: String,
    /// Byte offset of the `\begin{…}` tag.
    pub begin_start: usize,
    /// Byte offset just past `\begin{…}[opts]` — where the body starts.
    pub body_start: usize,
    /// Byte offset of the matching `\end{…}` tag (end of input when the
    /// region was never closed).
    pub end_start: usize,
    /// Byte offset just past the `\end{…}` tag (end of input when unclosed).
    pub end_end: usize,
}

/// Every verbatim region of `source`, in source order.
///
/// Regions are located by [`tokenize_with_spans`], so a `\begin{…}` that only
/// appears in a comment, inside math, inside another verbatim body or inside
/// a command argument is never reported as a region of its own — which is
/// exactly what the highlight pass needs to tell a real `code` block from the
/// same text sitting inertly in a listing, and what the linter needs to keep
/// verbatim bodies opaque. An unclosed region runs to the end of input and
/// reports `end_start == end_end`.
pub fn verbatim_blocks(source: &str) -> Vec<VerbatimBlock> {
    // (env, begin_start, body_start) of regions waiting for their \end.
    // Verbatim regions cannot nest — the tokenizer skips to the first
    // matching \end — but a stray \end must not pop the wrong region, so the
    // stack is matched by name.
    let mut open: Vec<(String, usize, usize)> = Vec::new();
    let mut blocks = Vec::new();
    for spanned in tokenize_with_spans(source).tokens {
        match spanned.token {
            Token::BeginVerbatim { env } => open.push((env, spanned.start, spanned.end)),
            Token::EndVerbatim { env } => {
                let Some(index) = open.iter().rposition(|(name, _, _)| *name == env) else {
                    continue; // stray \end{…} with no matching \begin
                };
                let (_, begin_start, body_start) = open.remove(index);
                // The EndVerbatim span runs from the body start to just past
                // the \end tag; a synthesized close (unclosed region) ends at
                // input end with no tag there.
                let terminator = format!("\\end{{{env}}}");
                let end_end = spanned.end;
                let closed = end_end >= terminator.len()
                    && source.get(end_end - terminator.len()..end_end) == Some(terminator.as_str());
                let end_start = if closed {
                    end_end - terminator.len()
                } else {
                    end_end
                };
                blocks.push(VerbatimBlock {
                    env,
                    begin_start,
                    body_start,
                    end_start,
                    end_end,
                });
            }
            _ => {}
        }
    }
    blocks
}

/// 1-based numbers of the source lines that lie **wholly inside** a verbatim
/// body: neither the `\begin{…}` line (its tag and options stay visible) nor
/// the `\end{…}` line (the environment check still needs to see it close).
///
/// This is the line-shaped view of [`verbatim_blocks`] for the checks that
/// walk a file line by line: inside such a line, `\input`, `\cite`, `\label`
/// and `\begin{…}` are source-code text, not document markup.
pub fn verbatim_body_lines(source: &str) -> HashSet<usize> {
    let mut lines = HashSet::new();
    for block in verbatim_blocks(source) {
        let mut offset = 0usize;
        for (index, line) in source.split('\n').enumerate() {
            let (start, end) = (offset, offset + line.len());
            offset = end + 1;
            if start >= block.body_start && end <= block.end_start {
                lines.insert(index + 1);
            }
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regions are reported for real blocks only, in source order, with
    /// offsets a scanner can slice on.
    #[test]
    fn verbatim_blocks_locate_real_regions() {
        let src = "before\n\\begin{code}[lang=python]\nx = 1\n\\end{code}\nafter\n";
        let blocks = verbatim_blocks(src);
        assert_eq!(blocks.len(), 1, "blocks: {blocks:?}");
        let block = &blocks[0];
        assert_eq!(block.env, "code");
        assert_eq!(&src[block.begin_start..block.begin_start + 7], "\\begin{");
        assert_eq!(&src[block.body_start..block.body_start + 7], "\nx = 1\n");
        assert_eq!(&src[block.end_start..block.end_end], "\\end{code}");
    }

    /// A `\begin{code}` living in a comment, in math, or inside another
    /// verbatim body is text — it opens no region of its own.
    #[test]
    fn verbatim_blocks_ignore_commented_nested_and_math_text() {
        for src in [
            "% \\begin{code}\nx\n% \\end{code}\n",
            "$ \\begin{code} x $ \\end{code}$\n",
            "\\begin{lstlisting}\n\\begin{code}\n\\end{lstlisting}\n",
        ] {
            let blocks = verbatim_blocks(src);
            assert!(
                !blocks.iter().any(|b| b.env == "code"),
                "{src:?} must not open a code region: {blocks:?}"
            );
        }
        // The lstlisting itself is still a region (the nested \begin{code}
        // was swallowed by it).
        let src = "\\begin{lstlisting}\n\\begin{code}\n\\end{lstlisting}\n";
        let blocks = verbatim_blocks(src);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].env, "lstlisting");
    }

    /// An unclosed region runs to end of input with `end_start == end_end`.
    #[test]
    fn verbatim_blocks_report_unclosed_regions() {
        let src = "\\begin{code}\nx = 1\n";
        let blocks = verbatim_blocks(src);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].end_start, src.len());
        assert_eq!(blocks[0].end_end, src.len());
    }

    /// The line view covers body lines only: the `\begin` line (tag +
    /// options) and the `\end` line stay visible to the checks that must see
    /// the environment open and close.
    #[test]
    fn verbatim_body_lines_skip_the_begin_and_end_lines() {
        let src = "\\documentclass{article}\n\\begin{code}[lang=python]\na\nb\n\\end{code}\n\
                   \\input{still-checked.tex}\n";
        let lines = verbatim_body_lines(src);
        assert_eq!(
            lines
                .iter()
                .copied()
                .collect::<std::collections::BTreeSet<_>>(),
            std::collections::BTreeSet::from([3, 4]),
            "only the body lines: {lines:?}"
        );

        // Same line begin+body+end: nothing is "wholly inside", so no line
        // is masked and the one-liner is still checked.
        let one_liner = "\\begin{code}x\\end{code}\n";
        assert!(verbatim_body_lines(one_liner).is_empty());

        // Commented and nested occurrences mask nothing (they are not
        // regions), so a commented block never hides real prose behind it.
        let commented = "% \\begin{code}\n\\input{missing.tex}\n% \\end{code}\n";
        assert!(verbatim_body_lines(commented).is_empty());
    }
}
