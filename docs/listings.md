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
| `caption` | caption text (LaTeX; wrap in braces if it contains a comma) | _(none)_ — without it the block has no number, no label and no list entry |
| `label` | label for `\ref`/`\pageref` (e.g. `lst:fib`); only meaningful with `caption` | _(none)_ — without a caption it warns (`label without caption is ignored`) |
| `pos` | `H` (inline, the default), `h`, `t`, `b`, `p` | `H` — the listing stays where it is written and may break across pages |
| `size` | `scriptsize`, `footnotesize`, `small`, `normalsize` | `small` (today's size); any other value warns and uses `small` |
| `style` | `light`, `light-mono`, `dark`, `dark-mono` (see [Styles](#styles)) | `[highlight.by_lang]` for that language, else `[highlight] style`, else `light`; any other value fails the build |

An unknown `lang` never fails the build: the block renders monochrome and
`texforge build` warns, naming the language and pointing at
`docs/listings.md`. An unknown option warns and is ignored.

Project-wide defaults live in `project.toml`:

```toml
[highlight]
theme = "github"     # "github" (default) or "one-light"
style = "light"      # "light" (default), "light-mono", "dark", "dark-mono"
lstlisting = true    # also rewrite \begin{lstlisting} blocks (default false)
numbers = true       # number every block unless it says otherwise
font = "inconsolata" # typewriter family for code blocks (default "document")
# caption_name = "Listing"        # override the language's listing name
# list_name = "List of Listings"  # override the list-of-listings heading

[highlight.by_lang]
bash = "dark"        # shell commands in a terminal-dark frame
```

## Font

The `[highlight] font` key sets the document's **typewriter family** (`\ttfamily`) so
inline `\texttt{}` matches the code blocks. The value must be one of the monospace
families shipped in the Tectonic bundle:

| value | family |
|---|---|
| `document` (default) | whatever the preamble sets — Latin Modern Mono in the bundled templates |
| `inconsolata` | Inconsolata (with `varqu`/`varl` for straight quotes and distinguishable `l`) |
| `source-code-pro` | Source Code Pro |
| `dejavu-sans-mono` | DejaVu Sans Mono |
| `plex-mono` | IBM Plex Mono |
| `fira-mono` | Fira Mono |

The chosen package is loaded once, guarded by `\@ifpackageloaded`, so a document
that already loads the same package is not loaded twice. Because the injection
happens **after** the author's preamble, the chosen family wins over a mono
package the author loaded earlier. Unknown values fail the build, naming the
value and listing the six valid ones.

## Captions, labels and the list of listings

A block with `caption=` gains a numbered caption line directly above the
frame — kept on the same page as the frame's first line — reading
"**Listing** N: …" (bold name, counter `tfxlisting`, numbered per chapter
as `chapter.N` when the class defines `\chapter`, else plain N):

```latex
\begin{code}[lang=rust, numbers=true, caption={Fibonacci}, label={lst:fib}, pos=t, size=footnotesize]
fn fib(n: u64) -> u64 { if n < 2 { n } else { fib(n-1) + fib(n-2) } }
\end{code}
```

With `label=` (placed right after the counter step), reference it from the
text with `\ref{lst:fib}` (prints N) or `\pageref{lst:fib}`. Every
captioned listing adds an entry to the list of listings; print it with
`\listoflistings` (a `\chapter*`/`\section*` heading plus the entries). A
document that never calls `\listoflistings` is unaffected.

The name follows the document language — resolved exactly as the
spell checker resolves it (`babel`/`polyglossia` option, else the
configured fallback language): english → "Listing" / "List of Listings";
spanish → "Listado" / "Índice de listados"; any other language → the
English names. `project.toml` `[highlight] caption_name` and `list_name`
override both.

Behaviour notes:

- By default a listing is **not** a float: it stays where it is written
  and may break across pages. With `pos=` other than `H` the block is
  wrapped in a `figure` of that placement (non-breaking), caption or not —
  `H` is never emitted as `figure[H]`, so no `float` package is needed.
  `pos=H` means inline (the default), accepted for symmetry with diagrams.
- A floated block longer than the text height cannot fit: the build warns
  (`floated listing is too tall …`) and renders it inline instead.
- Line numbers scale with `size=`. The caption machinery (counter, names,
  `\listoflistings`) is injected only when some block actually carries a
  `caption`, so a document without captions compiles exactly as before.

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

## Styles

Four styles, and you can mix them in one document — Python in the light
palette, shell commands in a terminal-dark one, a monochrome block for the
print edition:

| Style | Palette | Frame background |
|---|---|---|
| `light` (default) | the theme's own (`github` or `one-light`) | `github` `#f6f8fa`, `one-light` `#fafafa` |
| `light-mono` | greys only — keywords bold, comments italic | `#f6f6f6` |
| `dark` | the dark twin of the theme: GitHub Dark Dimmed or One Dark | `#22272e`, `#282c34` |
| `dark-mono` | greys only, on a dark frame | `#2b2b2b` |

`theme` still picks the *palette family* (`github` / `one-light`) and `style`
picks light vs dark vs monochrome; a `dark` block takes its colours from the
dark twin of the theme. The two `-mono` styles ignore `theme`: with hue gone
there is nothing left for the family to decide. An unknown style name fails
the build, naming the value and listing the valid ones.

The style of a block is resolved most-specific-first:

1. the block's own `style=` option
2. the `[highlight.by_lang]` table, keyed by `lang=` name **or alias**
   (`bash`, `sh`, `shell` and `zsh` are one entry)
3. `[highlight] style`
4. `light`

```toml
[highlight]
style = "light"

[highlight.by_lang]
bash = "dark"        # every shell block, whatever spelling its lang= uses
```

```latex
\begin{code}[lang=python, style=light-mono]  % …or one block on its own
def fib(n):
    return n
\end{code}
```

`lstlisting` blocks take their style from `by_lang` or the document default —
they have no `style=` option, so one there warns like any other unknown
`listings` option.

**Printing.** The dark styles lay down a good deal more ink and are meant for
screen and for colour printers; the `-mono` styles are the black-and-white
choice, where weight and slant carry the structure instead of hue. Every
token in the `dark`, `light-mono` and `dark-mono` styles meets WCAG AA
contrast (4.5:1) against its own background; `light` is the long-standing
palette and is byte-identical by design, so its contrast is unchanged.

## The frame

Every rewritten block (`code`, or an opted-in `lstlisting`) renders inside
a frame: a background tint from its style (`light` uses the theme's `#f6f8fa`
/ `#fafafa`, every other style its own), a 0.4pt hairline border in the
palette's comment colour, 4pt inner padding left and right, 3pt top and
bottom, full text width — plain LaTeX (`color.sty` rules only, no
`tcolorbox`/`mdframed`). The colours travel with the block, which is what
lets one document mix styles.

The frame is painted per line with zero-size overlays, so page breaking
works exactly as without it and a block split across pages stays open at
the break: the first fragment has no bottom border, the second no top
border, and both keep their side borders and tint.

With `numbers`, the gutter sits inside the frame: numbers right-aligned in
the palette's comment colour, separated from the code by a 0.3pt rule in the
border colour.

Vertical rhythm is `\medskip` before and after every block. The paragraph
right after a block is not indented — unless you left a blank line after
`\end{code}`, in which case the normal paragraph indent applies. A block
never starts at the very bottom of a page with fewer than two lines on it:
the first two and the last two lines are glued together.

## Behaviour notes

- Every source line becomes its own paragraph, so TeX may break the page
  between any two lines — a 60-line block flows across pages with no extra
  markup.
- Lines never wrap: spaces become non-breaking (`\tfxsp{}`, a
  `\nobreakspace` — never a bare `~`, which misfires under spanish `babel`
  before `}` or `-`) and tabs expand to 4,
  so indentation survives. A line wider than ~90 columns warns
  (`code line is {n} chars wide …`) — split the line; TeX still reports its
  own overfull boxes as well.
- Special characters (`` \ { } $ & # _ % ~ ^ < > " ' ` ``) are escaped at the
  character level, so code can never be misread as LaTeX (`"` uses
  `\char34{}`, which stays safe under `babel` shorthands such as spanish;
  `%` uses `\char37{}` so spanish `babel` cannot drop the preceding space;
  `<`/`>` use `\char60{}`/`\char62{}` so they print one exact monospace
  cell instead of a wider math glyph;
  `'`/`` ` `` use `\textquotesingle{}`/`\textasciigrave{}` so the listing
  keeps straight, copy-pasteable quotes).
  `|` prints as-is.
- The overfull-line and unknown-language warnings name the line numbers of
  the build copy **as the listing pass received it** — before the blocks
  are rewritten and before the preamble is injected. Without a diagram
  block in front of them those are exactly your source file's line
  numbers; the diagram pass (and, later, the rewrite and injection here)
  move the finished build copy. Engine (Tectonic) warnings and errors in
  a file the pass rewrote — or in the entry, where the preamble is
  injected even when its own blocks live in an `\input` — are remapped
  back through the build-copy line map, so `texforge build` points them
  at those same pass-input coordinates (same caveat: with a diagram block
  in front they are build-copy lines, not source lines; files reached via
  `\input` may be reported with or without `.tex`, and the map normalises
  both). (An unknown *option* warns without a line: the option parser is
  shared with the diagram blocks.)
- Only a real block counts: a `\begin{code}` behind a `%`, or quoted
  inside `verbatim`/`lstlisting`, is text — the pass ignores it and the
  document stays byte-identical.
- `code` is a verbatim environment for `texforge check`, `fmt` and `wc`
  (like `lstlisting` and `minted`): code never triggers prose linter rules,
  never counts as words, and `texforge fmt` passes its body through
  untouched.
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
gutter on (and `numbers=none` turns it off, even against a document-wide
`[highlight] numbers = true`), and every other `listings` option is dropped
with the usual unknown-option warning — except the caption vocabulary,
which maps onto the same behaviour: `caption=`, `label=`,
`float=`/`placement=` (first present wins) and a
`basicstyle=\footnotesize`-style font size (`\scriptsize`, `\footnotesize`,
`small`, `\normalsize`; `\smallskip` is not mistaken for `\small`).

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
in the One Light palette; the Rust block gains line numbers. Add
`[highlight.by_lang] bash = "dark"` and every shell block moves to a
terminal-dark frame while the rest of the document is untouched.
