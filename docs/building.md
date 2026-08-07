---
title: Building
description: Compile to PDF, watch mode, and the texforge runtime directory.
order: 4
---

# Building

## `texforge build`

Compiles the project to `<title>.pdf` in the project root:

1. Copies sources into a temporary build directory (originals are never
   modified).
2. Renders embedded [diagram environments](diagrams.md) to PNG.
3. Invokes Tectonic on the entry point declared in `project.toml` and
   places the resulting PDF (named after the document title) in the
   project root.

On the first run texforge downloads the Tectonic binary into
`~/.texforge/bin/` automatically.

## Reproducible builds

By default a build embeds a current timestamp, so the same source produces a
different PDF on each run. For anything that compares outputs — visual
regression, build caching, meaningful diffs — texforge can pin the build time:

```bash
texforge build --reproducible
```

This sets `SOURCE_DATE_EPOCH` for the Tectonic invocation. With no explicit
value a fixed epoch is used (never "now"); a release can pin its own:

```bash
texforge build --reproducible=1700000000
```

The same behaviour can be made the default for a project in `project.toml`
(see [Configuration](configuration.md)); the `--reproducible` flag wins when
both are present.

**The guarantee, and its limit:** identical source plus an identical Tectonic
version yields an identical PDF. A different Tectonic version (or engine
updates within it) can still change the output, and the setting does not alter
the visible content of a document — it only pins the embedded time.

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
