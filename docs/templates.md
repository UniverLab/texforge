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
texforge template list --local      # only locally installed templates
texforge template add <name>        # download a template from the registry
texforge template add <folder>      # install a template from a local folder
texforge template remove <name>     # remove an installed template
texforge template validate <name>   # verify template compatibility
```

Installed templates are cached under `~/.texforge/templates/`. The
built-in `general` template is listed once under `Installed:`, even when a
downloaded copy of `general` is cached.

## Template sources

Templates come from the official
[texforge-templates](https://github.com/UniverLab/texforge-templates)
registry, which is always present and used by default. You can add your own
template repositories in `~/.texforge/config.toml` (or
`$XDG_CONFIG_HOME/texforge/config.toml`) as `[[template_sources]]` tables:

```toml
[[template_sources]]
name = "acme"
repo = "acme/texforge-templates"
ref = "main"
```

`name` is local to your config (`[a-z0-9][a-z0-9-]*`, and `official` is
reserved for the official registry), `repo` is a GitHub `owner/name`, and
`ref` is optional — it defaults to `main`. A misconfigured entry (a reserved
or badly shaped name, a repo that is not `owner/name`, an empty `ref`, a
duplicate name) is an error naming the entry and the rule.

Requests to your own sources carry your GitHub credential when one exists —
`GITHUB_TOKEN` first, then the output of `gh auth token` — so private
repositories work. Without a credential the request is anonymous, and a 404
or 403 answers that the repo may be private, pointing at both ways to
authenticate. The official registry never gets a credential.

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

### Filling placeholders from a file

`texforge new` accepts `--values <FILE>`, a flat table of placeholder name to
value, so documents such as contracts, orders and audit reports are produced
from data:

```bash
texforge new contract-2026 -t contract --values client.toml
```

The format follows the extension: `.toml` parses as TOML, `.yaml` and `.yml`
as YAML. Any other extension is an error. Every value must be a string, an
integer, a float or a boolean — a nested table, an array, a null or a TOML
datetime is rejected with the offending key.

Values resolve in this order, the first one that exists winning:

1. the `--values` file
2. the project config (`./.texforge/config.toml`)
3. your user config (`~/.texforge/config.toml`)
4. the template default

The project name is the value of `title` unless the values file sets `title`
itself; that title is also written to `project.toml` as `[document] title`, so
the PDF is named after it. Every key in the file must be a placeholder the
template declares, and an unknown key fails the run naming both the unknown
keys and the declared names. Values are substituted verbatim: they are LaTeX
source, nothing is escaped, so a value may contain `\textbf{x}`, `&` or `%`.

A required placeholder left without a value fails the run, reporting every
missing placeholder and its description, and the directory the run created is
removed. Optional placeholders without a value are left as they are.

With `--values`, the values used are also written to `values.toml` in the new
project: a header comment, then one `key = "value"` line per key of that file,
sorted by key. Only the file's own keys are written — never values that came
from your configuration. Keep the file with the document and pass it back to
`texforge new --values` to regenerate from the same data.

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
