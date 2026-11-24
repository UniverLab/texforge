---
title: Diagrams
description: Embed Mermaid and Graphviz diagrams directly in your .tex files.
order: 5
---

# Diagrams

`texforge build` intercepts embedded diagram environments before
compilation and replaces them with rendered figures. Your original `.tex`
files are never modified — rendering happens in the `build/` copies.
If your preamble doesn't load them already, texforge adds `graphicx`
(and `float` for `pos=H`) to the build copy whenever a document uses
diagrams.

All three renderers are pure Rust: no browser, no Node.js, no `dot` binary
required.

Diagrams are embedded as vector PDF, so they stay sharp at any zoom level
and their label text remains selectable and searchable in the final PDF.
If a diagram's SVG can't be converted to PDF, texforge falls back to a
rasterized PNG for that one diagram, prints a warning naming it, and the
build still succeeds.

## Including SVG files

Your own SVG artwork needs no hand conversion: include it directly and the
build converts it to vector PDF (with the same PNG fallback and warning as
diagrams) before compilation. Your `.tex` files are never modified — the
conversion happens in the `build/` copies.

```latex
\usepackage{graphicx}
% ...
\includegraphics[width=\linewidth]{figs/logo.svg}
```

An include counts as SVG when the path ends in `.svg`, or when it has no
extension and `<project>/<path>.svg` exists while none of `<project>/<path>.pdf`,
`.png`, `.jpg`, `.jpeg` or `.eps` exists (a bare `{figs/logo}` beside a
`logo.pdf` keeps resolving to the PDF, exactly as LaTeX would). Other
extensions (`.png`, `.jpg`, …) are never touched.

Only files reachable from the entry through `\input` are rewritten;
`\include` is not followed. Paths resolve against the project root —
`\graphicspath` is not supported. SVG text is rendered with the same shared
fonts as diagram figures, so labels stay selectable in the final PDF; a
missing SVG file fails the build with `<file>:<line>: SVG not found: <path>`.

## Mermaid

```latex
% Default: width=\linewidth, floats like any figure, no caption
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

## Accents and special characters

Mermaid, D2 and Graphviz do not read LaTeX, but the block sits in a `.tex`
file, so the usual LaTeX accents are still accepted: texforge translates
them to the corresponding UTF-8 character before rendering, in all three
environments.

```latex
\begin{mermaid}
flowchart LR
  A["M\'etodos"] --> B["Evaluaci\'on"] --> C["\~nand\'u"]
\end{mermaid}
```

`\'e`, `\'{e}` and `{\'e}` all render as `é` and share a single cached image,
so whichever escape style the document already uses works unchanged. Acute,
grave, circumflex, diaeresis, tilde, macron and dot accents work with or
without braces; the letter forms `\c{c}` (cedilla), `\v{s}` (caron),
`\u{a}`, `\H{o}`, `\r{a}` (ring) and `\k{a}` need the braced argument.
`\-` is dropped, and `\&`, `\%` and `\_` become the literal characters.
Any other backslash is left untouched — a Graphviz `\n` line break still
starts a new line. `texforge check` does not flag these escapes: they are
normal in the surrounding LaTeX.

## Options

| Option | Default | Description |
|---|---|---|
| `width` | `\linewidth` | Image width (ignored when `scale` is set); when given without `height`, the figure keeps its natural aspect ratio |
| `height` | _(none)_ | Image height (ignored when `scale` is set) |
| `scale` | _(none)_ | Scale factor — overrides `width`/`height` when set |
| `keepaspectratio` | _(auto with width only)_ | Keep the aspect ratio. Added automatically when `width` is set without `height`; set it explicitly to keep the proportions when you give both `width` and `height` |
| `pos` | _(none — floats)_ | Figure placement (`H`, `t`, `b`, `h`, `p`); omit it to let LaTeX place the figure like any other image, `H` keeps it exactly where it is written |
| `caption` | _(none)_ | Figure caption |
| `label` | _(none)_ | `\label{…}` placed after the caption, for `\ref` |
| `style` | `default` | Editorial style preset — see below |

A figure given only `width=` keeps the rendered artefact's natural
proportions: texforge adds `keepaspectratio` for you, so the image is never
stretched or squashed. Give both `width` and `height` to fix both dimensions,
and add `keepaspectratio` yourself if you want the image scaled to fit that
box without distortion; `scale=` overrides both dimensions.

Every diagram becomes a `figure`. Without `pos` it floats like any other
image; `pos=H` keeps it exactly where it is written; `caption` adds a
numbered caption and `label` a `\ref` target.

`texforge check` resolves a `\ref` to a diagram `label=`, with or without a `caption`.

If an option value contains a comma, wrap it in braces — the same
convention LaTeX packages already use for this:

```latex
\begin{mermaid}[caption={Preset \texttt{editorial}: paleta restringida, un solo acento}]
flowchart LR
  A --> B
\end{mermaid}
```

Without the braces, the diagram intercepts commas as option separators,
so `un solo acento` would be parsed as a second, unrecognized option and
the caption would end at `restringida`. An unrecognized option prints a
warning naming it and the environment, but the build continues; an
unterminated `{` fails the build.

## Style presets

By default, each renderer draws diagrams with whatever it ships as its own
look — which means a Mermaid figure and a D2 figure in the same document can
look like they came from different tools. `style=` applies one of five
named presets consistently across all three renderers:

```latex
\begin{mermaid}[style=editorial, caption=System flow]
flowchart LR
  A --> B --> C
\end{mermaid}
```

| Preset | Effect | Suits |
|---|---|---|
| `default` | Each renderer's own untouched default. Omitting `style=` always renders this — no existing document changes appearance. | Any document that wants the renderers' own look. |
| `editorial` | Restrained palette (ink `#1D1510`, garnet accent `#87222C`, muted brown `#6B5A43`), one accent colour, no drop shadows, thin strokes, generous whitespace, EB Garamond type. | The `article` and `workshop` templates. |
| `monochrome` | Greyscale only — for documents printed in black and white. | The `apa` templates. |
| `technical` | Drafting register: uniform stroke weight, no fills, labels in a monospaced face where the renderer allows it. | The `ieee` template. |
| `diagram-design` | Warm paper-tint nodes (`#F1ECE4`), muted brown lines and arrows (`#6B5A43`), ink text (`#1D1510`), sans-serif face, generous spacing; no failure accent. | Documents that want a warm hand-drawn register. |

An unrecognised style name (e.g. `style=editoral`) fails the build with a
message naming the value and listing the valid ones — it never silently
falls back to `default`.

### `diagram-design`

```latex
\begin{mermaid}[style=diagram-design, caption=System flow]
flowchart LR
  A --> B --> C
\end{mermaid}
```

What it expresses: tinted nodes, muted lines and arrows, ink text, a
sans-serif face and extra spacing — the look of warm hand-drawn
figures. What it does not: stroke width, corner radius and arrowhead
shape are not controllable through the engines, and there is no failure
accent — the garnet `#87222C` of the report's figures is deliberately
not part of the preset, so a node that must stand out gets its colour
per node in the diagram source.

Inspired by ["diagram-design"](https://github.com/cathrynlavery/diagram-design)
by Cathryn Lavery (MIT).

Presets are named, not parameterised: there is no `style=editorial,accent=#ff0000`.
Opening them to arbitrary colours would make them themes, a different and
larger feature.

### Project-wide default

Set a document-wide default in `project.toml`:

```toml
[diagrams]
style = "editorial"
```

The environment's own `style=` attribute overrides this; omitting both
falls back to `default`.

### Fidelity varies by renderer

The three renderers expose very different styling surfaces, so the same
preset degrades differently depending on which one draws it — a diagram
still renders and the build still succeeds even where a renderer can't
express part of a preset.

- **Mermaid** has the richest surface (a full theme: colours, font, layout
  spacing), so every preset is expressed almost exactly — colour, font and
  spacing all carry over.
- **D2** has native theming (a named colour palette), so presets map onto
  it with good fidelity. `technical`'s monospaced labels come from D2's own
  `mono` theme rule, which also adds minor drafting-style ornamentation
  (double borders, container dots) to nested/grouped diagrams as a side
  effect — not a colour or fidelity gap, just a quirk of the one lever D2
  exposes for a monospaced face.
- **Graphviz** has no theme concept at all — styling is per-node and
  per-edge only. Node fill/border colour and edge colour carry over, but a
  document-wide background is not expressible, and neither is a
  font family (`layout-rs` never reads `fontname`, so for Graphviz — as
  for D2, which fixes its own faces — only colours change).

See also: highlight source code with the [`code` environment](listings.md) — native syntax highlighting with no `listings`/`minted` setup.
