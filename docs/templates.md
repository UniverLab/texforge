---
title: Templates
description: Install, manage and validate LaTeX templates from the registry or a local folder.
order: 7
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
texforge template add <folder>      # install a template from a local folder
texforge template remove <name>     # remove an installed template
texforge template validate <name>   # verify template compatibility
```

Installed templates are cached under `~/.texforge/templates/`.

## Using a template

```bash
texforge new my-doc -t apa-general
```

Or pick one interactively with `texforge init`.

## Local templates

To try a template you are still writing, point `template add` at its folder:

```bash
texforge template add ./my-template
```

The folder needs a `template.toml` manifest and a `main.tex`. The install
name is the manifest's `id`, and the folder is copied into
`~/.texforge/templates/<id>` as it is. If a template with that `id` is already
installed, the command fails; pass `--force` to replace it:

```bash
texforge template add ./my-template --force
```

A template installed this way is marked as local and **is never refreshed from
the network** — `texforge template refresh` skips it, and `texforge new` uses
its files exactly as they are on disk. Install it again with `--force` after
editing the folder.

## Rate limits

Downloads come from GitHub. If the registry refuses a request — usually
because an anonymous caller hit the hourly rate limit — texforge prints why,
including any `Retry-After` / `x-ratelimit-reset` values the server sent, and
suggests installing a template from a local folder instead. The underlying
reason is also shown by `texforge new -t <name>` when it cannot download a
missing template.

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
repository, then install it locally with `texforge template add <folder>`
while you refine it.
