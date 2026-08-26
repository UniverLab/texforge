---
title: CLI Reference
description: Every texforge command and flag.
order: 9
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
