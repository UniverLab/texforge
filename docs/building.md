---
title: Building
description: Compile to PDF, watch mode, and the texforge runtime directory.
order: 4
---

# Building

## `texforge build`

Compiles the project to `<title>.pdf` in the project root. The file name is the
`[document] title` in lowercase with every run of characters other than letters
and digits replaced by one hyphen, and no leading or trailing hyphen, so
`My Thesis: Final Version` becomes `my-thesis-final-version.pdf`:

1. Copies sources into a temporary build directory (originals are never
   modified).
2. Renders embedded [diagram environments](diagrams.md) to PNG.
3. Invokes Tectonic on the entry point declared in `project.toml` and
   places the resulting PDF (named after the document title) in the
   project root.

On the first run texforge downloads the Tectonic binary into
`~/.texforge/bin/` automatically. On the next build (or `texforge doctor`)
an outdated managed copy is updated to the supported version automatically;
when the Tectonic that runs is older than the supported version — for
example one earlier in `PATH` — the build warns about it and continues.

## Building a template folder

A folder carrying a `template.toml` — with or without a `project.toml` —
builds straight to a filled PDF, without creating a project first:

```bash
texforge build --values client.toml
texforge build --values client.toml --out ../nda-acme.pdf
```

Placeholders are rendered into a temporary copy outside the folder, then the
normal pipeline runs on that copy: the folder itself is never modified and
nothing is ever written inside it. Only `.tex` files are substituted, with
the same rules as `texforge new`; every other file is copied byte-for-byte,
except `template.toml`, `.git/`, `build/`, `.texforge/`, the values file and
`*.pdf`, which are skipped. A missing required placeholder fails the run
naming each one and the values file, before anything compiles. When the
folder also has a `project.toml`, its title, entry, bibliography, highlight,
diagrams and reproducible settings apply; with only a `template.toml`, the
entry is `main.tex` and the title is the manifest's display name. The PDF
lands at `--out`, or at `<folder-name>.pdf` in the parent directory when
`--out` is omitted, and the run ends with `wrote <path>` — `--out` pointing
inside the folder is an error, as is `--values` on a folder with no
`template.toml`. A relative `--out` resolves against the current directory,
so a bare file name would land inside the folder and is rejected: write
`../name.pdf` to place it next to the folder. Without `--values`, a folder
that also has a `project.toml` builds as a plain project; a folder with only
a `template.toml` fails telling you `--values` is needed. `--values` cannot
be combined with `--watch`.

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
last build. Press `Ctrl+C` to stop. The header banner follows the terminal
width — the large font at 82 columns or wider, the compact one below that.
When stdout is not a TTY no banner is printed. See the
[CLI reference](cli-reference.md#startup-banner).

## Cleaning

```bash
texforge clean   # remove build artifacts
```

## Preview

`texforge preview` rasterizes the compiled PDF to PNG pages for quick visual review or for embedding in documentation:

```bash
# Rasterize all pages (writes to ./preview/)
texforge preview

# Single page
texforge preview --page 1

# Custom output directory
texforge preview --out ./previews

# Scale up for higher resolution (default: 1.0 pixel per PDF point)
texforge preview --scale 2.0
```

## PDF Inspection

Once built, the PDF can be inspected without re-compiling:

```bash
# Extract text as readers and accessibility tools see it
texforge pdf text

# Report pages, fonts, whether fonts are embedded, and metadata
texforge pdf info

# List which section opens each page (for diffing in version control)
texforge pdf pages

# Verify significant source words survived into the PDF
# (catches missing fonts, encoding issues, rendering bugs)
texforge pdf check
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
