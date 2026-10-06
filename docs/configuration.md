---
title: Configuration
description: Global user configuration and the project.toml manifest.
order: 8
---

# Configuration

Texforge has two configuration layers: a **global user config** (who you
are, your defaults) and a **per-project manifest** (`project.toml`).

## Global configuration

Stored in `~/.texforge/config.toml` (or
`$XDG_CONFIG_HOME/texforge/config.toml`). These values are used as
replaceable placeholders in templates.

**Interactive setup:**

```bash
texforge config
```

The wizard asks for:

- **Name** — your full name
- **Email** — your email address
- **Institution** — your institution/organization
- **Language** — fallback language for documents without an explicit `\usepackage[lang]{babel}` or `polyglossia` declaration (default: `english`). If a document declares its language in the preamble, that takes precedence.

**Command-line interface:**

```bash
texforge config list                      # view all settings
texforge config name                      # get a value
texforge config name "Ada Lovelace"       # set a value
texforge config email "ada@example.com"
texforge config institution "University of Tech"
texforge config language "spanish"
```

## Project manifest — `project.toml`

Every texforge project has a `project.toml` at its root. It is generated
by `texforge new` / `texforge init`:

```toml
[document]
title = "My Thesis"
author = "Ada Lovelace"
template = "general"

[build]
entry = "main.tex"
# bibliography = "references.bib"   # optional
# reproducible = true               # optional: reproducible builds by default

# [diagrams]
# style = "editorial"               # optional: document-wide diagram style default

# [highlight]
# theme = "github"                  # optional: code-listing palette ("github", "one-light")
# style = "light"                   # optional: document-wide listing style ("light", "light-mono", "dark", "dark-mono")
# lstlisting = true                 # optional: also rewrite \begin{lstlisting} blocks
# numbers = true                    # optional: number every code block
# font = "inconsolata"              # optional: typewriter family for code blocks ("document", "inconsolata", "source-code-pro", "dejavu-sans-mono", "plex-mono", "fira-mono")
# caption_name = "Listing"          # optional: override the listing name
# list_name = "List of Listings"    # optional: override the list heading

# [highlight.by_lang]               # optional: per-language styles, by name or alias
# bash = "dark"
```

| Key | Description |
|---|---|
| `document.title` | Document title |
| `document.author` | Document author |
| `document.template` | Template the project was created from |
| `build.entry` | Entry `.tex` file passed to the engine |
| `build.bibliography` | Optional `.bib` file used by the linter |
| `build.reproducible` | Optional: pin `SOURCE_DATE_EPOCH` so identical source plus the same Tectonic version yields an identical PDF. `true` uses a fixed default epoch; a number pins an explicit epoch (`reproducible = 1700000000`); `false` or absent keeps the default behaviour. Overridden by `texforge build --reproducible` when that flag is present. |
| `diagrams.style` | Optional: document-wide default diagram style preset (`default`, `editorial`, `monochrome`, `technical`; see [Diagrams](diagrams.md)). A `style=` on the diagram environment itself overrides this. |
| `highlight.theme` | Optional: code-listing palette family (`github`, `one-light`; see [Code listings](listings.md)). |
| `highlight.style` | Optional: document-wide listing style (`light`, `light-mono`, `dark`, `dark-mono`). A block's `style=` option wins over it. |
| `highlight.by_lang` | Optional: per-language styles, keyed by language name or alias (`bash = "dark"`); wins over `highlight.style` for blocks in that language. |
| `highlight.lstlisting` | Optional: also rewrite `\begin{lstlisting}` blocks with native highlighting (default `false` — without it, `listings` users keep real `listings.sty` behaviour). |
| `highlight.numbers` | Optional: number every line of every code block unless the block sets `numbers=` itself. |
| `highlight.font` | Optional: typewriter family for code blocks (`document`, `inconsolata`, `source-code-pro`, `dejavu-sans-mono`, `plex-mono`, `fira-mono`; see [Code listings](listings.md)). |
| `highlight.caption_name` | Optional: override the language-resolved listing name (`Listing` / `Listado`). |
| `highlight.list_name` | Optional: override the language-resolved list-of-listings heading. |
