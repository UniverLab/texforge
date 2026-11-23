---
title: CLI Reference
description: Every texforge command and flag.
order: 10
---

# CLI Reference

```
texforge <command> [options]
```

This page documents the 1.0 command surface exactly as the binary
implements it: every command and subcommand with its flags, defaults, one
example, and its exit codes. When this page and the binary disagree, the
binary wins. The frozen surface is guarded by `tests/cli_surface.rs` and
`tests/cli_surface.txt`: any change to a command, flag, `--json` shape, or
exit code is deliberate and updates this page in the same change.

## Exit codes

The canonical table. Every section below ends with "Exit codes: see the
table" because the codes are shared, not per-command.

| Code | Meaning |
|---|---|
| `0` | Success. The command did what it says. For `update --check`, up to date. |
| `1` | Lint finding or build failure. `check` found an error (or any finding with `--deny-warnings`); `fmt --check` found a file that needs formatting; `build` failed (missing entry, engine error); `update --check` found an available update; an update install-phase failure after a successful check. |
| `2` | Usage or network. A clap usage error (unknown command, bad flag value, missing argument); or `update` / `update --check` could not complete the release check (network, DNS, TLS, HTTP ≥ 400, unparsable response — the cause is the single line on stderr). |

When the reader of a command's output goes away while it is still printing —
`texforge skill | head`, `texforge outline | grep -q chapter`, a closed
terminal — the command stops quietly and exits `0`, with nothing on stderr.
This applies to every command that prints; a command that fails on its own
still prints `Error: ...` and exits `1`.

## Startup banner

`texforge init`, the `texforge config` wizard and `texforge build --watch`
draw the texforge banner above their prompt or header. The font follows the
terminal width at the moment it is printed: the large font when the terminal
is at least 82 columns wide — the width of its widest line — and a compact
four-line font otherwise. A banner line is never wrapped or cut.

Nothing is printed when stdout is not a TTY (redirected, piped or captured),
and the `--json` commands (`stats`, `outline`) never draw the banner.

## Project lifecycle

### texforge new

Create a new project from a template.

```
texforge new <name> [-t <template>] [--values <file>]
```

| Flag | Value | Default |
|---|---|---|
| `--template`, `-t` | template name | `general` |
| `--values` | path to a `.toml`, `.yaml` or `.yml` file | none |

The `--values` file fills template placeholders; its keys are echoed into
the project as `values.toml`. A `title` key also sets `[document] title`
in the generated `project.toml`, so the PDF is named after it.

Example:

```
texforge new mi-tesis -t apa-general --values cliente.yaml
```

Exit codes: see the table.

### texforge init

Interactive wizard — new project or migrate an existing one.

```
texforge init
```

No flags.

Example:

```
cd mi-tesis-existente/ && texforge init
```

Exit codes: see the table.

### texforge build

Compile the project to PDF.

```
texforge build [--watch] [--delay <s>] [--verbose] [--reproducible[=<epoch>]]
  [--preview] [--preview-page <n>] [--preview-out <path>]
```

| Flag | Value | Default |
|---|---|---|
| `--watch` | none (switch) | off |
| `--delay` | debounce seconds before rebuilding | `2` |
| `--verbose` | none (switch): print every engine warning instead of a per-file summary | off |
| `--reproducible` | optional epoch, seconds since the Unix epoch; bare flag pins a fixed default epoch | off |
| `--preview` | none (switch): write a PNG of one page after each rebuild (requires `--watch`) | off |
| `--preview-page` | page to rasterize, 1-based (requires `--preview`) | `1` |
| `--preview-out` | fixed path for the preview PNG (requires `--preview`) | `preview.png` |

Example:

```
texforge build --watch --delay 5 --preview
```

Exit codes: see the table.

### texforge clean

Remove build artifacts.

```
texforge clean
```

No flags.

Example:

```
texforge clean
```

Exit codes: see the table.

## Quality

### texforge check

Lint the project without compiling (includes spell-check).

```
texforge check [--deny-warnings]
```

| Flag | Value | Default |
|---|---|---|
| `--deny-warnings` | none (switch): treat warnings as errors | off |

Example:

```
texforge check --deny-warnings
```

Exit codes: see the table.

### texforge fmt

Format `.tex` files in place.

```
texforge fmt [--check]
```

| Flag | Value | Default |
|---|---|---|
| `--check` | none (switch): check formatting without modifying files (CI-friendly) | off |

Example:

```
texforge fmt --check
```

Exit codes: see the table.

## Templates

### texforge template

Manage templates. Every action is a subcommand below.

```
texforge template <list|add|remove|validate|refresh|source>
```

Example:

```
texforge template list --local
```

Exit codes: see the table.

### texforge template list

List installed templates, plus every configured remote source by default.
Each line shows its source in parentheses: `  - <name> (<source>)` (`official`
for the official registry; the built-in template keeps `general (built-in)`).
A source that cannot be fetched warns once as
`Warning: source '<name>': <reason>` and never hides the other sources.

```
texforge template list [--local] [--json]
```

| Flag | Value | Default |
|---|---|---|
| `--local` | none (switch): only locally installed templates, skip the remote sources | off |
| `--json` | none (switch): print one JSON array (`name`, `source`, `installed`) and nothing else | off |

Example:

```
texforge template list --local
```

Exit codes: see the table.

#### `texforge template list --json` fields

Top-level array, one object per template (installed, plus available without
`--local`), sorted by `name`, then `source`:

| Field | Type | Meaning |
|---|---|---|
| `name` | string | template name within its source |
| `source` | string | source name (`official`, or `built-in` for the embedded template) |
| `installed` | boolean | true for installed templates, false for available ones |

### texforge template add

Add a template from a registry source or from a local folder.

```
texforge template add [--force] <source>
```

`<source>` is a template name, `source/name`, or a path to a template folder.
A bare name offered by more than one source fails listing every `source/name`
candidate — pick one explicitly. There is no carve-out for the official
source: two sources offering the same name always clash.

| Flag | Value | Default |
|---|---|---|
| `--force` | none (switch): replace an already installed template | off |

Example:

```
texforge template add ./my-template --force
```

Exit codes: see the table.

### texforge template remove

Remove an installed template.

```
texforge template remove <name>
```

No flags. `<name>` is the installed template name, or `source/name` when
several installed sources offer it.

Example:

```
texforge template remove apa-general
```

Exit codes: see the table.

### texforge template validate

Verify template compatibility.

```
texforge template validate <name>
```

No flags. `<name>` is the installed template name.

Example:

```
texforge template validate general
```

Exit codes: see the table.

### texforge template refresh

Refresh cached templates, bypassing the TTL — all of them, or one by name.

```
texforge template refresh [name]
```

No flags. Omit `name` to refresh every cached template. `name` accepts the
`source/name` form, like `add` and `remove`.

Example:

```
texforge template refresh general
```

Exit codes: see the table.

### texforge template source

Manage extra template sources (registries). The official registry is always
present and default; sources added here are stored as `[[template_sources]]`
tables in the user config file.

```
texforge template source <add|list|remove>
```

### texforge template source add

Add a template source from a GitHub repository. The reachability check runs
unless `--no-check` is given; the config file is written only after it passes.

```
texforge template source add [--ref <ref>] [--no-check] <name> <repo-or-url>
```

`<repo-or-url>` is `owner/name`, `https://github.com/owner/name` or
`https://github.com/owner/name.git` (all three become `owner/name`).
`<name>` must match `[a-z0-9][a-z0-9-]*`; `official` is reserved.

| Flag | Meaning | Default |
|------|---------|---------|
| `--ref` | git ref to use | `main` |
| `--no-check` | skip the reachability check (for offline use) | off |

Example:

```
texforge template source add acme https://github.com/acme/texforge-templates.git --ref v2
```

### texforge template source list

List the official source first (marked as default, never removable), then the
configured ones: name, `owner/name` and ref.

```
texforge template source list [--json]
```

| Flag | Meaning | Default |
|------|---------|---------|
| `--json` | print one JSON array (`name`, `repo`, `ref`, `official`) and nothing else | off |

Example:

```
texforge template source list --json
```

### texforge template source remove

Remove a template source from the config file. Installed templates are left
untouched.

```
texforge template source remove <name>
```

No flags. Removing `official` or an unknown name is an error.

Example:

```
texforge template source remove acme
```

Exit codes: see the table.

## Spell-check

### texforge spell

Manage the personal spell-check whitelist (dictionary). Every action is a
subcommand below. Default scope is global (`~/.texforge/spell-words`); both
scopes are unioned at check time.

```
texforge spell <add|list|remove>
```

Example:

```
texforge spell list --local
```

Exit codes: see the table.

### texforge spell add

Add one or more words to the whitelist.

```
texforge spell add [--local|--global] <words>...
```

| Flag | Value | Default |
|---|---|---|
| `--local` | none (switch): target the project whitelist | off (global) |
| `--global` | none (switch): target the global personal dictionary | off (the default scope is global) |

`--local` and `--global` conflict; passing both is a usage error (exit 2).

Example:

```
texforge spell add LaTeX RGB --local
```

Exit codes: see the table.

### texforge spell list

List the effective whitelist words for the scope.

```
texforge spell list [--local|--global]
```

| Flag | Value | Default |
|---|---|---|
| `--local` | none (switch): list the project whitelist | off (global) |
| `--global` | none (switch): list the global personal dictionary | off (the default scope is global) |

Example:

```
texforge spell list --global
```

Exit codes: see the table.

### texforge spell remove

Remove one or more words from the whitelist.

```
texforge spell remove [--local|--global] <words>...
```

| Flag | Value | Default |
|---|---|---|
| `--local` | none (switch): target the project whitelist | off (global) |
| `--global` | none (switch): target the global personal dictionary | off (the default scope is global) |

Example:

```
texforge spell remove RGB
```

Exit codes: see the table.

## PDF inspection

### texforge pdf

Inspect the compiled PDF. Every action is a subcommand below.

```
texforge pdf <text|info|pages|check>
```

Example:

```
texforge pdf info
```

Exit codes: see the table.

### texforge pdf text

Print the PDF text as a reader or accessibility tool would see it.

```
texforge pdf text [--raw]
```

| Flag | Value | Default |
|---|---|---|
| `--raw` | none (switch): keep ligature codepoints unnormalized | off |

Example:

```
texforge pdf text --raw
```

Exit codes: see the table.

### texforge pdf info

Report pages, fonts (embedded?), and metadata.

```
texforge pdf info
```

No flags.

Example:

```
texforge pdf info
```

Exit codes: see the table.

### texforge pdf pages

Report which section opens each page (diff-friendly, one line per page).

```
texforge pdf pages
```

No flags.

Example:

```
texforge pdf pages
```

Exit codes: see the table.

### texforge pdf check

Check significant source words appear in the PDF text.

```
texforge pdf check
```

No flags.

Example:

```
texforge pdf check
```

Exit codes: see the table.

## Document analysis

### texforge outline

Print the document's section tree.

```
texforge outline [--json]
```

| Flag | Value | Default |
|---|---|---|
| `--json` | none (switch): output JSON instead of a human-readable tree | off |

Example:

```
texforge outline --json
```

Starred sectioning commands (`\chapter*`, `\section*`, ...) are listed unnumbered: they keep their level and title but consume no number.

Exit codes: see the table.

#### `texforge outline --json` fields

Top-level object:

| Field | Type | Meaning |
|---|---|---|
| `document` | string | document name (the project title) |
| `sections` | array | sections in `\input` traversal order |

Each entry of `sections`:

| Field | Type | Meaning |
|---|---|---|
| `level` | integer | section level (`0` = part, …, `4` = subsubsection) |
| `number` | string | dotted section number (`1`, `1.1`, `2.1.1`); empty for starred sections |
| `title` | string | resolved section title (LaTeX markup stripped to text; a title that is exactly one standard macro such as `\abstractname` resolves to its English text) |
| `file` | string | `.tex` file containing the section, relative to the project root |
| `line` | integer | 1-based line of the section command within `file` |

### texforge stats

Count words per section (default) or per `.tex` file.

```
texforge stats [--by <section|file>] [--json]
```

| Flag | Value | Default |
|---|---|---|
| `--by` | `section` or `file`: break down by section or by `.tex` file | `section` |
| `--json` | none (switch): output JSON instead of a human-readable breakdown | off |

In section mode a starred sectioning command (`\chapter*`, `\section*`, ...) has
an empty section number: its words are reported under its title, with no number
before it, and it consumes no number.

Example:

```
texforge stats --by file --json
```

Exit codes: see the table.

#### `texforge stats --json` fields

Top-level object:

| Field | Type | Meaning |
|---|---|---|
| `document` | string | document name (the project title) |
| `total_words` | integer | total counted words across all files |
| `file_count` | integer | number of `.tex` files scanned |
| `preamble_words` | integer | words between `\begin{document}` and the first section |
| `sections` | array | per-section (or per-file with `--by file`) breakdown |

Each entry of `sections`:

| Field | Type | Meaning |
|---|---|---|
| `path` | string | dotted section number, or the `.tex` file name with `--by file`; empty for a starred section |
| `level` | integer or absent | section level (`0` = part, …, `6` = subparagraph); absent with `--by file` |
| `words` | integer | word count for this section or file |

## Preview

### texforge preview

Rasterize the compiled PDF to PNG pages.

```
texforge preview [--page <n>] [--scale <s>] [--out <dir>]
```

| Flag | Value | Default |
|---|---|---|
| `--page` | page number to rasterize, 1-based | all pages |
| `--scale` | rasterization scale in pixels per PDF point | `1.0` |
| `--out` | directory to write PNGs to | `<project>/preview` |

Example:

```
texforge preview --page 1 --scale 2.0 --out ./previews
```

Exit codes: see the table.

## Diagnostics

### texforge doctor

Diagnose the managed environment (Tectonic, cache, fonts, dictionaries, and
project state). Like a build, `doctor` updates an outdated managed Tectonic
to the supported version, and warns when the Tectonic in `PATH` is older
than the supported one.

```
texforge doctor
```

No flags.

Example:

```
texforge doctor
```

Exit codes: see the table.

## Agent skill

### texforge skill

Print the agent skill embedded in this binary.

```
texforge skill [--references|--list]
```

| Flag | Value | Default |
|---|---|---|
| `--references` | none (switch): also print the bundled reference files, each preceded by `=== references/<name> ===` | off |
| `--list` | none (switch): list the bundled reference file names, one per line | off |

`--references` and `--list` conflict; passing both is a usage error
(exit 2). The skill is embedded at build time from `skill/` in the
repository, so the text always matches the installed version. It names the
version that printed it, so a stale copy can be spotted.

Example:

```
texforge skill --references
```

Exit codes: see the table.

## Maintenance

### texforge update

Update texforge to the latest stable release (always asks first; the prompt
defaults to NO).

```
texforge update [--check] [--yes]
```

| Flag | Value | Default |
|---|---|---|
| `--check` | none (switch): only report whether an update is available, never download | off |
| `--yes` | none (switch): install the update without asking | off (asks first) |

`texforge update` fetches the latest stable GitHub release (drafts and
prereleases excluded), downloads the asset for your platform, verifies the
minisign signature of the release's `SHA256SUMS.txt` (public key
`RWRDOZx1ameWKtWCHDVp3YmIsBiJszM2U4SJbCMkZ8/iSkW9Pi1yWzDD`) before trusting
its checksums, and replaces the running binary
— atomically, in place. A 1.x release with no sums file, no signature file,
or a bad signature is refused before anything is installed; 0.x releases
predate signing and stay lenient. It never updates silently. `--check` never
prompts, never downloads, and never touches local paths.

- A `cargo install` binary is refused: cargo owns that file, so run `cargo install --force texforge` instead (exits `0`).
- `--check` exits `1` when an update is available, `0` when up to date, `2` when the check could not be completed.

Example:

```
texforge update --check
```

Exit codes: see the table.

### texforge uninstall

Remove everything texforge manages under `~/.texforge`.

```
texforge uninstall [--yes] [--dry-run] [--include-spell-words]
```

| Flag | Value | Default |
|---|---|---|
| `--yes` | none (switch): skip the confirmation prompt | off |
| `--dry-run` | none (switch): print the plan without removing anything | off |
| `--include-spell-words` | none (switch): also remove the personal spell dictionary (your own writing) | off (preserved) |

The texforge binary itself is never removed by this command.

Example:

```
texforge uninstall --dry-run
```

Exit codes: see the table.

## Configuration

### texforge config

Manage global configuration.

```
texforge config [list|<key> [<value>]]
```

No flags. With no arguments, opens the interactive wizard (name, email,
institution, language). `list` shows all configured values; a lone key
shows its value; key plus value sets it. Valid keys: `name`, `email`,
`institution`, `language`.

Example:

```
texforge config language spanish
```

Exit codes: see the table.

## Code listings

The `code` environment option `file=` is not a command: it reads a source
file into a listing at build time (for example
`\begin{code}[lang=python, file=scripts/demo.py]`). See
[listings](listings.md) for the full `code` environment reference.

## Global flags

| Flag | Description |
|---|---|
| `--help` | Show help for any command |
| `--version` | Show texforge version |
