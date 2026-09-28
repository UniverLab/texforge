---
title: Code listings
description: Syntax-highlighted code with 'code' blocks rendered by texforge — no listings.sty, no shell-escape.
order: 6
---

# Code listings

Write a `code` environment and texforge highlights it at build time:

```latex
\begin{code}[lang=python, numbers=true]
def fib(n):
    return n if n < 2 else fib(n - 1) + fib(n - 2)
\end{code}
```

Highlighting is resolved **outside LaTeX**, in Rust (via syntect): the build
rewrites each block into plain LaTeX that needs only `color.sty` — no
`listings.sty`, no `minted`, no `-shell-escape`, no Python. Your sources are
never modified; the rewrite happens on the build copy only, and the required
`\usepackage{color}` guard plus colour definitions are injected into the
build copy's preamble automatically (skipped when you already load `color`
or `xcolor` yourself, and safe when a class such as beamer loads `xcolor`
behind the scenes).

Without a `code` block — and without the `lstlisting` opt-in below — the
pass writes nothing and documents compile exactly as before.

## Options

| Option | Values | Default |
|---|---|---|
| `lang` | language name or alias (table below); empty means a deliberate plain monospace block | plain block, no warning |
| `numbers` | `true` shows a line-number gutter | `false`, unless `[highlight] numbers = true` sets the document default (the block option wins) |

An unknown `lang` never fails the build: the block renders monochrome and
`texforge build` warns, naming the language and pointing at
`docs/listings.md`. An unknown option warns and is ignored.

Project-wide defaults live in `project.toml`:

```toml
[highlight]
theme = "github"     # "github" (default) or "one-light"
lstlisting = true    # also rewrite \begin{lstlisting} blocks (default false)
numbers = true       # number every block unless it says otherwise
```

## Languages

Names match by syntax name or file extension (case-insensitive); the aliases
are conveniences for the spellings authors actually type.

| `lang=` | Highlights as | Aliases |
|---|---|---|
| `python` | Python | `py` |
| `rust` | Rust | `rs` |
| `javascript` | JavaScript | `js` |
| `typescript` | TypeScript | `ts` |
| `bash` | Bash | `sh`, `zsh`, `shell` |
| `c`, `c++` | C / C++ | `cpp`, `cc` |
| `json` | JSON | — |
| `yaml` | YAML | `yml` |
| `toml` | TOML | — |
| `markdown` | Markdown | `md` |
| `latex` | LaTeX | `tex` |
| `r` | R | — |
| `sql` | SQL (also `MySQL`, `PostgreSQL` by name) | — |
| `clojure`, `css`, `diff`, `erlang`, `go`, `haskell`, `java`, `lua`, `matlab`, `ocaml`, `php`, `ruby`, `xml` | as named | `rb` (ruby) |
| `make` | Makefile | `mk` |
| `dot` | Graphviz DOT | `graphviz`, `gv` |
| `git` | Git diff output | — |
| `jsx`, `tsx` | JSX / TSX | — |
| `text`, `txt`, `plaintext`, *(empty)* | deliberate plain block — no warning | — |

## Themes

Two light palettes: `github` (default) and `one-light`, set with
`[highlight] theme`. Both are light-only by design: no block background is
ever painted, so a dark theme's white foreground text would disappear on
paper. An unknown theme name fails the build and lists the valid names.

## Behaviour notes

- Every source line becomes its own paragraph, so TeX may break the page
  between any two lines — a 60-line block flows across pages with no extra
  markup.
- Lines never wrap: spaces become non-breaking (`~`) and tabs expand to 4,
  so indentation survives. A line wider than ~90 columns warns
  (`code line is {n} chars wide …`) — split the line; TeX still reports its
  own overfull boxes as well.
- Special characters (`\ { } $ & # _ % ~ ^ < >`) are escaped at the
  character level, so code can never be misread as LaTeX. `|` prints as-is.
- Warnings name **build-copy** line numbers — the same coordinates
  Tectonic's own errors use (the diagram pass shifts lines first).
- The `code` environment name and the `\tfx` command prefix are reserved:
  defining them yourself fails the build with a file:line error telling you
  to rename.

## `lstlisting` opt-in

Existing `listings.sty` documents are left alone unless you ask:

```toml
[highlight]
lstlisting = true
```

Then `\begin{lstlisting}[language=Python, numbers=left]…\end{lstlisting}`
blocks are rewritten the same way: `language=` (case-insensitive, `[ISO]`
dialect prefixes stripped) carries over, `numbers=left|right` turns the
gutter on, and every other `listings` option is dropped with the usual
unknown-option warning.

## Why not minted

`minted` shells out to Pygments, which needs `-shell-escape` and a Python
installation — both outside what texforge enables. That is why
`texforge check` warns on `\usepackage{minted}` and suggests the `code`
environment instead.

## Worked example

```latex
\documentclass{article}
\begin{document}

\section{Setup}

\begin{code}[lang=bash]
# install dependencies
cargo build --release
\end{code}

\begin{code}[lang=rust, numbers=true]
fn main() {
    println!("fib(10) = {}", fib(10));
}
\end{code}

\end{document}
```

With `[highlight] theme = "one-light"` in `project.toml`, both blocks render
in the One Light palette; the Rust block gains line numbers.
