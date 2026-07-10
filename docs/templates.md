---
title: Templates
description: Install, manage and validate LaTeX templates from the registry.
order: 6
---

# Templates

Templates are managed through the
[texforge-templates](https://github.com/UniverLab/texforge-templates)
registry. The `general` template is embedded in the binary, so creating a
project always works — even offline.

## Commands

```bash
texforge template list              # installed + available in the registry
texforge template list --installed  # only locally installed templates
texforge template add <name>        # download a template from the registry
texforge template remove <name>     # remove an installed template
texforge template validate <name>   # verify template compatibility
```

Installed templates are cached under `~/.texforge/templates/`.

## Using a template

```bash
texforge new my-doc -t apa-general
```

Or pick one interactively with `texforge init`.

## Placeholders

Templates declare placeholders (title, author, institution, …) in their
manifest. When you create a project, texforge fills them from your answers
and from your [global configuration](configuration.md) — so you don't
retype your name and institution for every document.

## Writing your own template

A template is a directory with a manifest describing its files and
placeholders:

- `id`, `version`, `display_name`, `description`
- `files` — include/exclude globs
- `placeholders` — name, type, description, required, default, choices
- `post_generate` — optional scripts to run after generation

The easiest way to start is to copy an existing template from the
[texforge-templates](https://github.com/UniverLab/texforge-templates)
repository and run `texforge template validate` against it.
