---
title: Linting & Formatting
description: Static analysis with texforge check and canonical style with texforge fmt.
order: 8
---

# Linting & Formatting

## Linter — `texforge check`

Runs static analysis without compiling, including spell-check:

| Check | What it verifies |
|---|---|
| `\input{file}` | referenced file exists |
| `\includegraphics{img}` | image exists |
| `\cite{key}` | key exists in the `.bib` file |
| `\ref{label}` / `\label{label}` | cross-reference consistency |
| `\begin{env}` / `\end{env}` | no unclosed environments |
| Spelling | prose against language-specific dictionaries |

Errors come with file, line and a suggestion:

```
ERROR [main.tex:47]
  \includegraphics{missing.png} — file not found

ERROR [main.tex:12]
  \cite{smith2020} — key not found in .bib

ERROR [main.tex:23]
  \begin{figure} never closed
  suggestion: Add \end{figure}

ERROR [main.tex:18]
  mispeled — not in dictionary
```

### Spell-Check

Language is detected from the document:

- `\usepackage[spanish]{babel}` — uses Spanish Hunspell dictionary
- `\usepackage[polyglossia]{...}` — language extracted from polyglossia declaration
- No declaration — falls back to `texforge config language` (default: `english`)

Dictionaries download automatically into `~/.texforge/dicts/` on first use.

Manage custom words with `texforge spell`:

```bash
# Add to global dictionary (all projects)
texforge spell add "LaTeX" "UUID"

# Add to current project only
texforge spell add "ProjectCodename" --local

# List and remove
texforge spell list [--local]
texforge spell remove "LaTeX" [--local]
```

Both scopes are unioned at check time — project-local and global words are both respected.

## Formatter — `texforge fmt`

Applies opinionated formatting inspired by `rustfmt`:

- Consistent indentation (2 spaces) inside environments
- Collapsed multiple blank lines
- Aligned `\begin{}` / `\end{}` blocks

One canonical output regardless of input style — git diffs stay clean.

```bash
texforge fmt           # format in place
texforge fmt --check   # check without modifying (CI-friendly)
```

`fmt --check` exits non-zero when files would change, which makes it easy
to enforce formatting in CI.
