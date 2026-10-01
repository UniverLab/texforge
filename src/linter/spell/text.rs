//! Token stream -> the flat, accent-resolved text the spell checker sees,
//! plus the (offset, line) chunk map that puts findings back in the source.

use crate::texparse::{SpannedToken, Token};
use crate::texutil::strip_empty_groups;

use super::accents::{
    is_accent_command, is_transparent_command, try_resolve_accent, AccentBaseSource,
};

/// 1-based line number of a byte offset.
pub(super) fn line_of(source: &str, offset: usize) -> usize {
    let offset = offset.min(source.len());
    1 + source[..offset].matches('\n').count()
}

pub(super) fn build_spell_text(
    tokens: &[SpannedToken],
    source: &str,
) -> (String, Vec<(usize, usize)>) {
    let mut out = String::new();
    let mut line_chunks: Vec<(usize, usize)> = Vec::new();
    let mut pending_text_skip: usize = 0;
    let mut tokens_to_skip: usize = 0;

    // The cursor is the iterator itself: accent macros that must swallow the
    // following tokens express that as a bounded skip counter instead of
    // arithmetic on an index.
    for (i, spanned) in tokens.iter().enumerate() {
        if tokens_to_skip > 0 {
            tokens_to_skip -= 1;
            continue;
        }
        match &spanned.token {
            Token::Text(t) => {
                let skip = pending_text_skip;
                pending_text_skip = 0;
                let line = line_of(source, spanned.start);
                let chunk = strip_empty_groups(&t[skip..]);
                if !chunk.is_empty() {
                    let base = out.len();
                    line_chunks.push((base, line));
                    out.push_str(&chunk);
                    push_line_breaks(&mut line_chunks, base, &chunk, line);
                }
            }
            Token::Command { name, args } if is_accent_command(name) => {
                match try_resolve_accent(name, args, tokens, i) {
                    Some((composed, source_kind)) => {
                        let line = line_of(source, spanned.start);
                        line_chunks.push((out.len(), line));
                        out.push(composed);
                        match source_kind {
                            AccentBaseSource::FromArgs => {}
                            AccentBaseSource::FromNextText { chars_to_skip } => {
                                pending_text_skip = chars_to_skip;
                            }
                            AccentBaseSource::FromDotlessIJ {
                                extra_tokens_to_skip,
                                chars_to_skip_in_last,
                            } => {
                                // The accent token itself is consumed by this
                                // iteration; swallow the brace, the dotless
                                // command and its closing brace after it.
                                tokens_to_skip = extra_tokens_to_skip - 1;
                                pending_text_skip = chars_to_skip_in_last;
                            }
                        }
                    }
                    None => {
                        out.push(' ');
                        pending_text_skip = 0;
                    }
                }
            }
            Token::Command { name, .. } if is_transparent_command(name) => {
                // Emit nothing and do NOT push a separator: the characters on
                // either side belong to the same word.
                pending_text_skip = 0;
            }
            Token::Command { name, .. } if name == "i" || name == "j" => {
                let line = line_of(source, spanned.start);
                line_chunks.push((out.len(), line));
                out.push(if name == "i" { 'i' } else { 'j' });
                pending_text_skip = 0;
            }
            _ => {
                out.push(' ');
                pending_text_skip = 0;
            }
        }
    }

    (out, line_chunks)
}

/// Record the line each embedded newline starts inside one appended text
/// chunk. The chunk begins at `base` on `line`, so the n-th newline (1-based)
/// is followed by the first byte of line `line + n`.
fn push_line_breaks(line_chunks: &mut Vec<(usize, usize)>, base: usize, chunk: &str, line: usize) {
    for (n, (byte, _)) in chunk.match_indices('\n').enumerate() {
        line_chunks.push((base + byte + 1, line + n + 1));
    }
}

pub(super) fn line_for_offset(line_chunks: &[(usize, usize)], offset: usize) -> usize {
    match line_chunks.binary_search_by_key(&offset, |(off, _)| *off) {
        Ok(idx) => line_chunks[idx].1,
        Err(0) => line_chunks.first().map_or(1, |&(_, l)| l),
        Err(idx) => line_chunks[idx - 1].1,
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use tempfile::TempDir;

    use super::super::lint_files;
    use super::super::test_support::run_with_home;
    use crate::test_sync::ENV_LOCK;

    // --- TE12: ligature-workaround empty groups must not split words ---

    /// The four real tokens from the reported document must each be checked
    /// as their joined form, not fragmented on the `{}` empty-group ligature
    /// workaround.
    #[test]
    fn ligature_workaround_empty_groups_are_checked_as_joined_words() {
        let _lock = ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = TempDir::new().unwrap();
        let dicts_dir = home.path().join(".texforge").join("dicts");
        fs::create_dir_all(&dicts_dir).unwrap();
        fs::write(
            dicts_dir.join("english.txt"),
            "artificial\nworkflows\nmlflow\nlocal\nfirst\n",
        )
        .unwrap();

        let orig_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", home.path());

        let src = "\\begin{document}\nArtif{}icial workf{}lows MLf{}low local-f{}irst\n\
                   \\end{document}";
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
            "empty-group ligature workarounds must be checked as joined words, not fragments: {:?}",
            findings
        );
    }

    /// A genuine misspelling written with the empty-group idiom must still
    /// be reported, exactly once, as the joined word — never as fragments,
    /// since a fragment is not something the author can search for.
    #[test]
    fn misspelled_word_with_empty_group_is_reported_as_joined_word() {
        let _lock = ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = TempDir::new().unwrap();
        let dicts_dir = home.path().join(".texforge").join("dicts");
        fs::create_dir_all(&dicts_dir).unwrap();
        fs::write(dicts_dir.join("english.txt"), "hello\nworld\n").unwrap();

        let orig_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", home.path());

        let src = "\\begin{document}\nHello Wrongwo{}rd world\n\\end{document}";
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
            "expected exactly one finding for the joined misspelling: {:?}",
            findings
        );
        assert!(
            findings[0].message.contains("wrongword"),
            "must report the joined word: {:?}",
            findings
        );
        assert!(
            !findings
                .iter()
                .any(|f| f.message.contains("wrongwo'") || f.message.contains("'rd")),
            "must not report fragments of the joined word: {:?}",
            findings
        );
    }

    /// A `{}` at the start or end of a word, and two empty groups inside one
    /// word, must all be stripped correctly rather than merely the common
    /// mid-word case.
    #[test]
    fn empty_group_at_start_end_and_doubled_behave_sanely() {
        let _lock = ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = TempDir::new().unwrap();
        let dicts_dir = home.path().join(".texforge").join("dicts");
        fs::create_dir_all(&dicts_dir).unwrap();
        fs::write(dicts_dir.join("english.txt"), "begin\nend\nmiddlepoint\n").unwrap();

        let orig_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", home.path());

        let src = "\\begin{document}\n{}Begin End{} Middle{}Po{}int\n\\end{document}";
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
            "leading/trailing/doubled empty groups must all be stripped: {:?}",
            findings
        );
    }

    // --- dotless \i/\j: they render their own letter, with no separator
    // inserted between the surrounding text and them ---

    fn spell_text(src: &str) -> String {
        let tokenized = crate::texparse::tokenize_with_spans(src);
        super::build_spell_text(&tokenized.tokens, src).0
    }

    #[test]
    fn dotless_i_emits_its_letter_without_a_break() {
        assert_eq!(spell_text(r"ag\i jt"), "agi jt");
    }

    #[test]
    fn dotless_j_emits_its_letter_without_a_break() {
        assert_eq!(spell_text(r"ag\j jt"), "agj jt");
    }

    /// `\'{\i}` composes to `í` and swallows exactly the brace, the dotless
    /// command and the closing brace — the trailing text after `}` must
    /// survive untouched.
    #[test]
    fn accented_dotless_i_skips_only_the_braced_command() {
        assert_eq!(spell_text(r"\'{\i} ok"), "í ok");
    }

    // --- line accuracy: a Text token spanning several source lines must
    // report each misspelling on its own line ---

    /// A long Text token (one token, several source lines) must report each
    /// misspelling on its own line, not on the line the token began.
    #[test]
    fn word_after_newline_in_a_long_text_token_reports_its_own_line() {
        let src = "one\ntwo recieve\n\nthree teh";
        // The whole bug is that this is a single Text token.
        let text_tokens = crate::texparse::tokenize_with_spans(src)
            .tokens
            .iter()
            .filter(|t| matches!(t.token, crate::texparse::Token::Text(_)))
            .count();
        assert_eq!(text_tokens, 1, "fixture must be one Text token");

        run_with_home("", "one\ntwo\nthree\n", || {
            let files = vec![("main.tex".to_string(), src.to_string())];
            let root = TempDir::new().unwrap();
            let findings = lint_files(&files, root.path(), Some("english")).unwrap();
            assert_eq!(findings.len(), 2, "{findings:?}");
            let recieve = findings
                .iter()
                .find(|f| f.message.contains("recieve"))
                .expect("recieve must be flagged");
            assert_eq!(recieve.line, 2, "recieve is on source line 2: {findings:?}");
            let teh = findings
                .iter()
                .find(|f| f.message.contains("'teh'"))
                .expect("teh must be flagged");
            assert_eq!(teh.line, 4, "teh is on source line 4: {findings:?}");
        });
    }

    /// An accent-composed word after a newline reports the line of the word's
    /// own source line, not the line of the text chunk that precedes it.
    #[test]
    fn accent_composed_word_after_newline_reports_its_own_line() {
        let src = "x $a$ y\nA caf\\'{e} z";
        run_with_home("", "hello\nworld\n", || {
            let files = vec![("main.tex".to_string(), src.to_string())];
            let root = TempDir::new().unwrap();
            let findings = lint_files(&files, root.path(), Some("english")).unwrap();
            assert_eq!(findings.len(), 1, "{findings:?}");
            assert!(
                findings[0].message.contains("café"),
                "must report the composed word: {findings:?}"
            );
            assert_eq!(
                findings[0].line, 2,
                "café is on source line 2: {findings:?}"
            );
        });
    }
}
