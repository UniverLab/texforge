---
title: CLI Reference
description: Every texforge command and flag.
order: 10
---

# CLI Reference

```
texforge <command> [options]
```

## Project lifecycle

| Command | Description |
|---|---|
| `texforge new <name>` | Create a new project from the default template |
| `texforge new <name> -t <template>` | Create with a specific template |
| `texforge init` | Interactive wizard — new project or migrate an existing one |
| `texforge build` | Compile to PDF |
| `texforge build --watch` | Watch for changes and rebuild automatically |
| `texforge build --watch --delay <s>` | Custom debounce delay (default: 2s) |
| `texforge build --reproducible` | Pin `SOURCE_DATE_EPOCH` to a fixed epoch so identical source yields an identical PDF |
| `texforge build --reproducible=<epoch>` | Reproducible build with an explicit epoch (seconds since the Unix epoch) |
| `texforge clean` | Remove build artifacts |

## Quality

| Command | Description |
|---|---|
| `texforge check` | Lint without compiling (includes spell-check) |
| `texforge check --deny-warnings` | Treat warnings as errors |
| `texforge fmt` | Format `.tex` files in place |
| `texforge fmt --check` | Check formatting without modifying (CI-friendly) |

## Templates

| Command | Description |
|---|---|
| `texforge template list` | List installed + available in the registry |
| `texforge template list --installed` | List only locally installed templates |
| `texforge template add <name>` | Download a template from the registry |
| `texforge template remove <name>` | Remove an installed template |
| `texforge template validate <name>` | Verify template compatibility |
| `texforge template refresh` | Refresh all cached templates (bypass TTL) |
| `texforge template refresh <name>` | Refresh one cached template (bypass TTL) |

## Spell-Check

| Command | Description |
|---|---|
| `texforge spell add <words>...` | Add word(s) to personal dictionary |
| `texforge spell add <words>... --local` | Add to project-local dictionary instead of global |
| `texforge spell list` | List all words in personal dictionary |
| `texforge spell list --local` | List project-local dictionary |
| `texforge spell remove <words>...` | Remove word(s) from personal dictionary |
| `texforge spell remove <words>... --local` | Remove from project-local dictionary |

Default scope is global (`~/.texforge/spell-words`). Both scopes are unioned at check time.

## PDF Inspection

| Command | Description |
|---|---|
| `texforge pdf text` | Extract text as seen by readers and accessibility tools |
| `texforge pdf text --raw` | Keep ligature codepoints as separate characters |
| `texforge pdf info` | Report pages, fonts, embedding status, metadata |
| `texforge pdf pages` | List which section opens each page (diff-friendly) |
| `texforge pdf check` | Verify significant source words appear in the PDF text |

## Document Analysis

| Command | Description |
|---|---|
| `texforge outline` | Print the section tree |
| `texforge outline --json` | Output as JSON |
| `texforge stats` | Count words by section (default) |
| `texforge stats --by file` | Count words by `.tex` file |
| `texforge stats --json` | Output as JSON |

## Preview

| Command | Description |
|---|---|
| `texforge preview` | Rasterize all PDF pages to PNG (writes to `./preview/`) |
| `texforge preview --page <N>` | Rasterize page N only (1-based) |
| `texforge preview --scale <SCALE>` | Scale factor for rasterization (default: 1.0) |
| `texforge preview --out <DIR>` | Output directory (default: `./preview/`) |

## Diagnostics

| Command | Description |
|---|---|
| `texforge doctor` | Diagnose Tectonic, cache, fonts, dictionaries, and project |

## Maintenance

| Command | Description |
|---|---|
| `texforge update` | Update to the latest stable release (always asks first; the prompt's default is **NO**) |
| `texforge update --check` | Read-only check: exit `1` if an update is available, `0` if up to date, `2` if the check could not be completed |
| `texforge update --yes` | Install the update without asking |

`texforge update` fetches the latest stable GitHub release (drafts and prereleases excluded), downloads the asset for your platform, verifies its SHA256 checksum when the release ships one, and replaces the binary that is running — atomically, in place. It never updates silently.

- A `cargo install` binary is refused: cargo owns that file, so run `cargo install --force texforge` instead (exits `0`), printing `installed with cargo — run: cargo install --force texforge`.
- A network, API, or parse failure prints its cause as one line on stderr and exits `2` — for `--check` and plain `update` alike — so a script never reads an outage as a new release. Failures after a successful check (download, checksum, permissions) exit `1`.
- `--check` never prompts, never downloads, and never touches local paths.

Exit codes of `texforge update [--check]`:

| Code | Meaning |
|---|---|
| `0` | Up to date (or update installed / declined / cargo-managed refusal) |
| `1` | `--check`: an update **is available**; in plain `update`, a failure after a successful check (download, checksum, permissions) |
| `2` | The release check **could not be completed** (network, DNS, TLS, HTTP ≥ 400, unparsable response); the cause is the single line printed on stderr. Applies to `--check` and plain `update` alike. |

The version arrow line prints versions without the `v` prefix: `texforge 0.0.1 → 0.9.0`.

## Uninstall

| Command | Description |
|---|---|
| `texforge uninstall` | Remove everything texforge manages under `~/.texforge` |
| `texforge uninstall --yes` | Skip the confirmation prompt |
| `texforge uninstall --dry-run` | Print the plan without removing anything |
| `texforge uninstall --include-spell-words` | Also remove the personal spell dictionary (preserved by default) |

The texforge binary itself is never removed by this command. The personal spell dictionary (`~/.texforge/spell-words`) contains your own writing and is preserved unless `--include-spell-words` is passed.

## Configuration

| Command | Description |
|---|---|
| `texforge config` | Interactive wizard (name, email, institution, language) |
| `texforge config list` | Show all configured values |
| `texforge config <key>` | Show value for a key |
| `texforge config <key> <value>` | Set a value |

Valid keys: `name`, `email`, `institution`, `language`.

## Global flags

| Flag | Description |
|---|---|
| `--help` | Show help for any command |
| `--version` | Show texforge version |
