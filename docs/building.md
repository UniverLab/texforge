---
title: Building
description: Compile to PDF, watch mode, and the texforge runtime directory.
order: 4
---

# Building

## `texforge build`

Compiles the project to `build/main.pdf`:

1. Copies sources into `build/` (originals are never modified).
2. Renders embedded [diagram environments](diagrams.md) to PNG.
3. Invokes Tectonic on the entry point declared in `project.toml`.

On the first run texforge downloads the Tectonic binary into
`~/.texforge/bin/` automatically.

## Watch mode

`texforge build --watch` watches `.tex` files and rebuilds automatically:

```bash
texforge build --watch            # rebuild after 2s of inactivity (default)
texforge build --watch --delay 5  # custom debounce delay in seconds
```

The terminal shows a live session timer, build count and the result of the
last build. Press `Ctrl+C` to stop.

## Cleaning

```bash
texforge clean   # remove build artifacts
```

## Runtime directory

Texforge keeps its engine and template cache under `~/.texforge/`:

```
~/.texforge/
  bin/
    tectonic            # LaTeX engine (auto-installed on first build)
  templates/
    general/            # cached templates
    apa-general/
    ...
```

Deleting this directory is safe: everything is re-downloaded on demand.
