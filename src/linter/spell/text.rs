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
    let mut i = 0;
    let mut pending_text_skip: usize = 0;

    while i < tokens.len() {
        match &tokens[i].token {
            Token::Text(t) => {
                let skip = pending_text_skip;
                pending_text_skip = 0;
                let line = line_of(source, tokens[i].start);
                let chunk = strip_empty_groups(&t[skip..]);
                if !chunk.is_empty() {
                    line_chunks.push((out.len(), line));
                    out.push_str(&chunk);
                }
                i += 1;
            }
            Token::Command { name, args } if is_accent_command(name) => {
                match try_resolve_accent(name, args, tokens, i) {
                    Some((composed, source_kind)) => {
                        let line = line_of(source, tokens[i].start);
                        line_chunks.push((out.len(), line));
                        out.push(composed);
                        i += 1;
                        match source_kind {
                            AccentBaseSource::FromArgs => {}
                            AccentBaseSource::FromNextText { chars_to_skip } => {
                                pending_text_skip = chars_to_skip;
                            }
                            AccentBaseSource::FromDotlessIJ {
                                extra_tokens_to_skip,
                                chars_to_skip_in_last,
                            } => {
                                i += extra_tokens_to_skip - 1;
                                pending_text_skip = chars_to_skip_in_last;
                            }
                        }
                    }
                    None => {
                        out.push(' ');
                        i += 1;
                        pending_text_skip = 0;
                    }
                }
            }
            Token::Command { name, .. } if is_transparent_command(name) => {
                // Emit nothing and do NOT push a separator: the characters on
                // either side belong to the same word.
                i += 1;
                pending_text_skip = 0;
            }
            Token::Command { name, .. } if name == "i" || name == "j" => {
                let line = line_of(source, tokens[i].start);
                line_chunks.push((out.len(), line));
                out.push(if name == "i" { 'i' } else { 'j' });
                i += 1;
                pending_text_skip = 0;
            }
            _ => {
                out.push(' ');
                i += 1;
                pending_text_skip = 0;
            }
        }
    }

    (out, line_chunks)
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
    use super::super::test_support::ENV_MUTEX;

    // --- TE12: ligature-workaround empty groups must not split words ---

    /// The four real tokens from the reported document must each be checked
    /// as their joined form, not fragmented on the `{}` empty-group ligature
    /// workaround.
    #[test]
    fn ligature_workaround_empty_groups_are_checked_as_joined_words() {
        let _lock = ENV_MUTEX.lock().unwrap();
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
        let _lock = ENV_MUTEX.lock().unwrap();
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
        let _lock = ENV_MUTEX.lock().unwrap();
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
}
