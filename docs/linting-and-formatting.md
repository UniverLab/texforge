---
title: Linting & Formatting
description: Static analysis with texforge check and canonical style with texforge fmt.
order: 8
---

# Linting & Formatting

## Linter — `texforge check`

Runs static analysis without compiling:

| Check | What it verifies |
|---|---|
| `\input{file}` | referenced file exists |
| `\includegraphics{img}` | image exists |
| `\cite{key}` | key exists in the `.bib` file |
| `\ref{label}` / `\label{label}` | cross-reference consistency |
| `\begin{env}` / `\end{env}` | no unclosed environments |

Errors come with file, line and a suggestion:

```
ERROR [main.tex:47]
  \includegraphics{missing.png} — file not found

ERROR [main.tex:12]
  \cite{smith2020} — key not found in .bib

ERROR [main.tex:23]
  \begin{figure} never closed
  suggestion: Add \end{figure}
```

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
