---
title: Texforge
description: A unified LaTeX workspace — write, render diagrams, and build PDFs with one self-contained tool.
order: 1
---

# Texforge

Texforge is a unified LaTeX workspace: one tool for writing, rendering
diagrams (Mermaid, Graphviz) and building PDFs. Set it up once and stay
focused on your document.

It is a single binary written in Rust. You do **not** need TeX Live, MiKTeX,
Node.js, a browser, or Graphviz installed — the LaTeX engine
([Tectonic](https://tectonic-typesetting.github.io/)) is downloaded
automatically on first build, and diagrams are rendered in pure Rust.

## Why texforge

A classic LaTeX setup means installing a multi-gigabyte distribution,
wiring up `latexmk`, and gluing external renderers for diagrams. Texforge
collapses that into a single workflow:

- **One-command setup** — install once; engine, templates and diagram
  renderers are included or fetched on demand.
- **Diagrams as first-class citizens** — write Mermaid or Graphviz blocks
  directly in your `.tex` files; they render and embed during build.
- **Guided workflows** — start a new project or migrate an existing one
  with `texforge init`.
- **Template registry** — install, manage and validate templates, with a
  built-in offline fallback.
- **Build and live edit** — compile once or use watch mode.
- **Smart linting** — catch missing files, broken references, bibliography
  keys and unclosed environments before compiling.
- **Opinionated formatter** — one canonical `.tex` style, clean git diffs.

## How the documentation is organized

- [Installation](installation.md) — install, update and uninstall.
- [Quick Start](quickstart.md) — from zero to PDF in two commands.
- [Building](building.md) — `build`, watch mode and the runtime directory.
- [Diagrams](diagrams.md) — Mermaid and Graphviz environments.
- [Templates](templates.md) — using the template registry.
- [Configuration](configuration.md) — global config and `project.toml`.
- [Linting & Formatting](linting-and-formatting.md) — `check` and `fmt`.
- [CLI Reference](cli-reference.md) — every command and flag.

## Part of UniverLab

Texforge is an experiment of [UniverLab](https://github.com/UniverLab),
an open computational laboratory. It follows the lab's engineering
principles: one tool one job, reproducibility first, offline-friendly
design.
