---
title: Quick Start
description: From zero to PDF in two commands.
order: 3
---

# Quick Start

## New project

```bash
texforge new my-thesis
cd my-thesis
texforge build
```

That's it: `build/main.pdf` is ready. On the very first build texforge
downloads Tectonic (the LaTeX engine) automatically — no TeX distribution
needed.

## Interactive wizard

If you prefer a guided flow, `texforge init` auto-detects the context:

- If a `.tex` file with `\documentclass` is found in the current
  directory, it **migrates the existing project**: asks for title and
  author and generates `project.toml`.
- Otherwise it **guides the creation of a new project**: asks for name
  and template.

```bash
# Existing LaTeX project
cd my-existing-thesis/
texforge init

# Empty directory
mkdir my-new-doc && cd my-new-doc
texforge init
```

## The everyday loop

```bash
texforge check          # lint: missing files, broken refs, unclosed envs
texforge fmt            # normalize formatting
texforge build          # compile to build/main.pdf
texforge build --watch  # rebuild automatically while you edit
```

## Project layout

A texforge project is a regular LaTeX project plus a `project.toml`
manifest:

```
my-thesis/
├── project.toml     # document metadata and build entry point
├── main.tex
├── src/             # chapters, sections (depends on template)
└── build/           # generated — PDF and intermediate files
```

See [Configuration](configuration.md) for the `project.toml` format and
[Templates](templates.md) for the available starting points.
