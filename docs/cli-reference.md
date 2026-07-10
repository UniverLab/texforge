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
| `texforge clean` | Remove build artifacts |

## Quality

| Command | Description |
|---|---|
| `texforge check` | Lint without compiling |
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
