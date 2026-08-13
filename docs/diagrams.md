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
| `style` | `default` | Editorial style preset — see below |

When a `caption` is given the diagram is wrapped in a `figure` environment
at the requested position; without it the image is embedded inline.

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
look like they came from different tools. `style=` applies one of four
named presets consistently across all three renderers:

```latex
\begin{mermaid}[style=editorial, caption=System flow]
flowchart LR
  A --> B --> C
\end{mermaid}
```

| Preset | Effect |
|---|---|
| `default` | Each renderer's own untouched default. Omitting `style=` always renders this — no existing document changes appearance. |
| `editorial` | Restrained palette, one accent colour, no drop shadows, thin strokes, generous whitespace. |
| `monochrome` | Greyscale only — for documents printed in black and white. |
| `technical` | Drafting register: uniform stroke weight, no fills, labels in a monospaced face where the renderer allows it. |

An unrecognised style name (e.g. `style=editoral`) fails the build with a
message naming the value and listing the valid ones — it never silently
falls back to `default`.

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
  spacing), so every preset is expressed almost exactly.
- **D2** has native theming (a named colour palette), so presets map onto
  it with good fidelity. `technical`'s monospaced labels come from D2's own
  `mono` theme rule, which also adds minor drafting-style ornamentation
  (double borders, container dots) to nested/grouped diagrams as a side
  effect — not a colour or fidelity gap, just a quirk of the one lever D2
  exposes for a monospaced face.
- **Graphviz** has no theme concept at all — styling is per-node and
  per-edge only. Node fill/border colour and edge colour carry over, but a
  document-wide background is not expressible, and neither is a
  monospaced label face (`technical`'s font rule has no effect here).
