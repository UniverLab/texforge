---
title: Configuration
description: Global user configuration and the project.toml manifest.
order: 7
---

# Configuration

Texforge has two configuration layers: a **global user config** (who you
are, your defaults) and a **per-project manifest** (`project.toml`).

## Global configuration

Stored in `~/.texforge/config.toml` (or
`$XDG_CONFIG_HOME/texforge/config.toml`). These values are used as
replaceable placeholders in templates.

**Interactive setup:**

```bash
texforge config
```

The wizard asks for:

- **Name** — your full name
- **Email** — your email address
- **Institution** — your institution/organization
- **Language** — document language (default: `english`)

**Command-line interface:**

```bash
texforge config list                      # view all settings
texforge config name                      # get a value
texforge config name "Ada Lovelace"       # set a value
texforge config email "ada@example.com"
texforge config institution "University of Tech"
texforge config language "spanish"
```

## Project manifest — `project.toml`

Every texforge project has a `project.toml` at its root. It is generated
by `texforge new` / `texforge init`:

```toml
[document]
title = "My Thesis"
author = "Ada Lovelace"
template = "general"

[build]
entry = "main.tex"
# bibliography = "references.bib"   # optional
# reproducible = true               # optional: reproducible builds by default
```

| Key | Description |
|---|---|
| `document.title` | Document title |
| `document.author` | Document author |
| `document.template` | Template the project was created from |
| `build.entry` | Entry `.tex` file passed to the engine |
| `build.bibliography` | Optional `.bib` file used by the linter |
| `build.reproducible` | Optional: pin `SOURCE_DATE_EPOCH` so identical source plus the same Tectonic version yields an identical PDF. `true` uses a fixed default epoch; a number pins an explicit epoch (`reproducible = 1700000000`); `false` or absent keeps the default behaviour. Overridden by `texforge build --reproducible` when that flag is present. |
