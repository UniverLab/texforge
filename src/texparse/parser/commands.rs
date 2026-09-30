//! Command dispatch: `\verb`, `\begin`/`\end`, `\href`, prose and plain commands.

use super::super::{is_command_char, offset_tokens, tokenize_with_spans};
use super::super::{
    SpannedToken, Token, MATH_ENVIRONMENTS, TABLE_PREAMBLE_ENVIRONMENTS, VERBATIM_ENVIRONMENTS,
};
use super::Parser;

impl<'a> Parser<'a> {
    pub(super) fn read_command_name(&mut self) -> String {
        let start = self.pos;
        while let Some(c) = self.peek() {
            if is_command_char(c) {
                self.bump();
            } else {
                break;
            }
        }
        self.src[start..self.pos].to_string()
    }

    /// `\verb[char][...]` and `\lstinline[opts][char][...]`: the delimiter is
    /// the first non-letter after the name (and any options); everything up to
    /// the next occurrence of that delimiter is verbatim and never prose.
    pub(super) fn handle_verb_command(
        &mut self,
        name: &str,
        start: usize,
        tokens: &mut Vec<SpannedToken>,
    ) {
        let mut args = Vec::new();
        if name == "lstinline" {
            while let Some(optional) = self.read_bracket_group() {
                args.push(optional);
            }
        }
        if self.peek() == Some('*') {
            self.bump();
        }
        let Some(delim) = self.bump() else {
            self.push(
                tokens,
                Token::Command {
                    name: name.to_string(),
                    args,
                },
                start,
                self.pos,
            );
            return;
        };
        let mut content = String::new();
        while let Some(c) = self.peek() {
            if c == delim || c == '\n' {
                break;
            }
            content.push(c);
            self.bump();
        }
        args.push(content);
        self.push(
            tokens,
            Token::Command {
                name: name.to_string(),
                args,
            },
            start,
            self.pos,
        );
    }

    pub(super) fn handle_begin(&mut self, start: usize, tokens: &mut Vec<SpannedToken>) {
        let env = self.read_braced_group().unwrap_or_default();
        // Discard float placement etc.: `\begin{figure}[htbp]`.
        let _ = self.read_bracket_group();
        let mid = self.pos;

        if env == "document" {
            self.push(tokens, Token::BeginDocument, start, mid);
            return;
        }
        if MATH_ENVIRONMENTS.contains(&env.as_str()) {
            self.push(tokens, Token::BeginMath, start, mid);
            self.skip_to_env_end(&env);
            self.push(tokens, Token::EndMath, mid, self.pos);
            return;
        }
        if VERBATIM_ENVIRONMENTS.contains(&env.as_str()) {
            self.push(
                tokens,
                Token::BeginVerbatim { env: env.clone() },
                start,
                mid,
            );
            self.skip_to_env_end(&env);
            self.push(tokens, Token::EndVerbatim { env }, mid, self.pos);
            return;
        }
        if TABLE_PREAMBLE_ENVIRONMENTS.contains(&env.as_str()) {
            self.consume_table_preamble(&env);
        }
        self.push(tokens, Token::Environment { name: env }, start, self.pos);
    }

    /// Consume the non-prose arguments that follow `\begin{tabular}` and its
    /// relatives. The `[pos]` shared by all of them was already discarded by
    /// the caller as "float placement"; this reads what plain discard leaves
    /// behind: `tabular*`/`tabularx`/`tabulary` take `{width}` before the
    /// column spec, all of them take a mandatory `{cols}`.
    pub(super) fn consume_table_preamble(&mut self, env: &str) {
        if matches!(env, "tabular*" | "tabularx" | "tabulary") {
            let _ = self.read_braced_group(); // {width}
            let _ = self.read_bracket_group(); // [pos] (tabular* only)
        }
        let _ = self.read_braced_group(); // {cols}
    }

    pub(super) fn handle_end(&mut self, start: usize, tokens: &mut Vec<SpannedToken>) {
        let env = self.read_braced_group().unwrap_or_default();
        if env == "document" {
            self.push(tokens, Token::EndDocument, start, self.pos);
        } else if MATH_ENVIRONMENTS.contains(&env.as_str()) {
            self.push(tokens, Token::EndMath, start, self.pos);
        } else if VERBATIM_ENVIRONMENTS.contains(&env.as_str()) {
            self.push(tokens, Token::EndVerbatim { env }, start, self.pos);
        } else {
            self.push(tokens, Token::Environment { name: env }, start, self.pos);
        }
    }

    /// `\href{url}{text}`: the URL is a non-prose argument, the link text is prose.
    pub(super) fn handle_href(&mut self, start: usize, tokens: &mut Vec<SpannedToken>) {
        let mut args = Vec::new();
        while let Some(optional) = self.read_bracket_group() {
            args.push(optional);
        }
        if let Some(url) = self.read_braced_group() {
            args.push(url);
        }
        self.push(
            tokens,
            Token::Command {
                name: "href".to_string(),
                args,
            },
            start,
            self.pos,
        );
        if let Some((text, text_start, _text_end)) = self.read_braced_group_spanned() {
            let sub = tokenize_with_spans(&text);
            self.unclosed_math
                .extend(sub.unclosed_math.iter().map(|offset| offset + text_start));
            tokens.extend(offset_tokens(sub.tokens, text_start));
        }
    }

    /// A command whose braced argument is prose, emitted as [`Token::Text`].
    pub(super) fn handle_prose_command(
        &mut self,
        name: &str,
        start: usize,
        tokens: &mut Vec<SpannedToken>,
    ) {
        let mut args = Vec::new();
        while let Some(optional) = self.read_bracket_group() {
            args.push(optional);
        }
        self.push(
            tokens,
            Token::Command {
                name: name.to_string(),
                args,
            },
            start,
            self.pos,
        );
        if let Some((prose, prose_start, _prose_end)) = self.read_braced_group_spanned() {
            let sub = tokenize_with_spans(&prose);
            self.unclosed_math
                .extend(sub.unclosed_math.iter().map(|offset| offset + prose_start));
            tokens.extend(offset_tokens(sub.tokens, prose_start));
        }
    }

    /// Any other command: greedily collect optional and braced arguments.
    pub(super) fn handle_plain_command(
        &mut self,
        name: &str,
        start: usize,
        tokens: &mut Vec<SpannedToken>,
    ) {
        let mut args = Vec::new();
        loop {
            if let Some(optional) = self.read_bracket_group() {
                args.push(optional);
                continue;
            }
            if let Some(braced) = self.read_braced_group() {
                args.push(braced);
                continue;
            }
            break;
        }
        self.push(
            tokens,
            Token::Command {
                name: name.to_string(),
                args,
            },
            start,
            self.pos,
        );
    }
}
