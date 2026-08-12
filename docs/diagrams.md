---
title: Diagrams
description: Embed Mermaid and Graphviz diagrams directly in your .tex files.
order: 5
---

# Diagrams

`texforge build` intercepts embedded diagram environments before
compilation and replaces them with rendered figures. Your original `.tex`
files are never modified — rendering happens in the `build/` copies.

All three renderers are pure Rust: no browser, no Node.js, no `dot` binary
required.

Diagrams are embedded as vector PDF, so they stay sharp at any zoom level
and their label text remains selectable and searchable in the final PDF.
If a diagram's SVG can't be converted to PDF, texforge falls back to a
rasterized PNG for that one diagram, prints a warning naming it, and the
build still succeeds.

## Mermaid

```latex
% Default: width=\linewidth, pos=H, no caption
\begin{mermaid}
flowchart LR
  A[Input] --> B[Process] --> C[Output]
\end{mermaid}

% With options
\begin{mermaid}[width=0.6\linewidth, caption=System flow, pos=t]
flowchart TD
  X --> Y --> Z
\end{mermaid}
```

## Graphviz / DOT

```latex
\begin{graphviz}[caption=Pipeline]
digraph G {
  rankdir=LR
  A -> B -> C
  B -> D
}
\end{graphviz}
```

## Options

| Option | Default | Description |
|---|---|---|
| `width` | `\linewidth` | Image width |
| `pos` | `H` | Figure placement (`H`, `t`, `b`, `h`, `p`) |
| `caption` | _(none)_ | Figure caption |

When a `caption` is given the diagram is wrapped in a `figure` environment
at the requested position; without it the image is embedded inline.
