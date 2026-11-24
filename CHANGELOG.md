# Changelog

All notable changes to texforge are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## Unreleased

### Fixed

- Dark (`dark`, `dark-mono`) code listings no longer show a thin light line between code lines: consecutive per-line background rules now overlap instead of abutting.
- Diagram figures given only `width=` now keep their natural aspect ratio instead of being distorted.
- `texforge check` and `texforge spell` no longer report words from `mermaid`, `d2` or `graphviz` diagram bodies as unknown prose.

### Notes

- `texforge new -t` now also takes a path to a local template folder (`./my-template`), used in place without installing; the `template` recorded in `project.toml` is the folder manifest's `id`.
- In the `ink` code-listing style, gutter line numbers now render in grey so they do not compete with the code.
- The capabilities example regained the Mermaid preset comparison and got richer Graphviz, D2 and TikZ figures.
- Code listings default to DejaVu Sans Mono unless `[highlight] font` is set or project sources load fontspec; set `font = "document"` to keep the preamble's typewriter family.
- The default DejaVu Sans Mono listing font now survives OpenType text packages (such as `ebgaramond`/`sourcesanspro`) that pull in fontspec: the injection re-selects T1 with the installed family when fontspec is loaded, so listings still embed DejaVu Sans Mono instead of falling back to Latin Modern.
- Copying a numbered code listing in viewers that honour PDF `ActualText` skips the line numbers; they still print on the page, `numbers=false` removes them everywhere, and other viewers may still copy them.
- The `ink` code-listing style now distinguishes keywords and numbers (bold), names (bold italic) and comments (italic), still black on white.
- `outline`/`stats` no longer count `\section`/`\subsection` inside `\titleformat`/`\titlespacing*` args as empty sections.
- `outline` and `stats` list starred sectioning commands (`\chapter*`, `\section*`, ...) unnumbered with an empty number, resolving title macros such as `\abstractname` to their English text.
- On release, set `date-released` and `version` in `CITATION.cff` to the release date and the released Cargo.toml version.
- Release checksums (`SHA256SUMS.txt`) are now signed with minisign.
- `texforge update` and `scripts/install.sh` verify the minisign signature of `SHA256SUMS.txt` before installing 1.x releases.
- `scripts/install.ps1` verifies the minisign signature of `SHA256SUMS.txt` on 1.x releases (`SKIP_SIGNATURE=1` skips the check).
- An outdated managed Tectonic is updated to the supported version on the next build or `doctor`, which also warns when the Tectonic in `PATH` is older than the supported one.
- `texforge template list` prints the built-in `general` template once under `Installed:`, even when a downloaded copy of `general` is cached.
- `texforge template list` shows each template's source as `  - <name> (<source>)` (plus `list --json`), and every command taking a template name accepts `source/name`, failing on clashes instead of picking silently.
- Piping output to a closed pipe (e.g. `texforge skill | head`) now exits quietly with code 0.
- `texforge new --values` now writes the file's `title` into `project.toml`, so the PDF is named after it instead of the directory.
- `texforge build --values <file> [--out <path>]` builds a folder carrying a `template.toml` straight to a filled PDF, rendering placeholders into a temporary copy without modifying any file of the folder; the PDF lands in the current directory unless `--out` says otherwise.
- `texforge new` and `texforge build` accept `--values` more than once: files merge in order with later files winning key by key, and the echoed `values.toml` lists the source files.
- `texforge check` resolves `\ref` to a `label=` option on `mermaid`, `d2` and `graphviz` blocks, with or without a `caption`.
- The example directory was renamed to `texforge-capabilities` (the old name was misspelled).
- The `texforge-capabilities` example was redone as an editorial document.
- `~/.texforge/config.toml` accepts `[[template_sources]]` tables (`name`, `repo`, optional `ref` defaulting to `main`) for extra template repositories; the official registry stays always present and default, and requests to your own sources use `GITHUB_TOKEN` (else `gh auth token`) so private repos work.
- `texforge template source add <name> <repo-or-url> [--ref <ref>] [--no-check]`, `texforge template source list [--json]` and `texforge template source remove <name>` manage those extra registries from the CLI instead of editing the config file by hand.
- The embedded skill now guides agents to suggest contributing back: reporting texforge or official-template defects as drafted issues, offering good documents as templates, and offering the user's own templates repository.

### Added

- The startup banner now picks a large or compact font from the terminal width, and prints nothing when stdout is not a TTY.
- `texforge new --values <FILE>` fills the placeholders a template declares
  from a flat TOML (`.toml`) or YAML (`.yaml`, `.yml`) table: strings,
  integers, floats and booleans, with file values winning over the project
  config, the user config and the template defaults. The values used are
  written to `values.toml` in the new project. A key the template does not
  declare fails the run listing the unknown and the declared names, and a
  required placeholder with no value now fails the run instead of leaving a
  raw `{{name}}` in the generated document (the directory the run created is
  removed).
- `texforge update` — the explicit self-update command. It fetches the latest
  stable GitHub release (drafts and prereleases excluded), always asks before
  replacing the running binary (default **NO**, `--yes` skips the prompt),
  verifies the asset's SHA256 checksum when the release ships a
  `SHA256SUMS.txt`, and swaps the binary atomically in place.
- `texforge update --check` — a read-only check that exits `1` when a newer
  stable release exists, `0` when up to date, and `2` when the check could
  not be completed: no prompt, no download, no local paths touched.
- Network or API failures exit `2` with a one-line cause on stderr for the
  explicit command — never exit `1`, which means "an update is available" —
  and stay silent on the passive `texforge init` notice.
- Syntax-highlighted code listings: the `code` environment (`[highlight]`
  config with `theme`, `lstlisting`, `numbers`), rendered by a pure-Rust
  syntect pass into plain LaTeX needing only `color.sty` — no
  `listings`/`minted` setup, no shell-escape. `texforge check` now warns on
  `\usepackage{minted}` and suggests `code` instead.
- Captions, labels and placement for code listings: `caption=`, `label=`,
  `pos=` and `size=` on `code` (and `caption=`, `label=`, `float=`/
  `placement=`, `basicstyle=\footnotesize` on an opted-in `lstlisting`).
  A captioned listing gets a bold "Listing N:" line above the frame, a
  `\ref`/`\pageref`-able label, and an entry in a new `\listoflistings`,
  numbered per chapter when the class defines `\chapter`; the names follow
  the document language (english / spanish) and are overridable with
  `[highlight] caption_name` / `list_name`.
- Four code-listing styles — `light`, `light-mono`, `dark`, `dark-mono` —
  selectable per block (`style=` on `code`), per language
  (`[highlight.by_lang] bash = "dark"`) or per document (`[highlight]
  style`), most specific winning. `dark` takes the dark twin of the theme
  (GitHub Dark Dimmed or One Dark) on a dark frame; the `-mono` styles are
  greyscale, using bold keywords and italic comments instead of hue, and
  are the black-and-white-printing choice. Styles can be mixed in one
  document; `light` is unchanged byte for byte.
- A monospace font for code listings: `[highlight] font = "…"`, one of
  `document` (default — the preamble's own typewriter family, byte-identical
  to before), `inconsolata`, `source-code-pro`, `dejavu-sans-mono`,
  `plex-mono` or `fira-mono`. The matching package is injected inside the
  generated preamble block, after the author's own preamble and guarded by
  `\@ifpackageloaded` so a package the document already loads is not loaded
  twice; only a build that rewrote at least one code block injects it, and an
  unknown value fails the build listing the valid ones.

- `texforge template add <folder>` installs a template from a local directory
  (`--force` replaces an existing one); a local template is never refreshed
  from the network. Registry downloads no longer count against GitHub's
  unauthenticated API limit, and a refused request now says so — including
  the `Retry-After`/`x-ratelimit-reset` values — instead of only "not found".

### Changed

- The `editorial` diagram preset now uses the refreshed template palette
  (ink `#1D1510`, garnet accent `#87222C`, muted brown `#6B5A43`) with an
  EB Garamond font stack for Mermaid, suiting the `article` and `workshop`
  templates; `monochrome` (for the `apa` templates) and `technical` (for
  `ieee`) gained matching font stacks, and D2/Graphviz render colours only.
  Cached diagrams render once more after this change, then reuse the cache.
- The release check used by `texforge init` moved into the updater so both
  paths share one lookup, throttled to once a day for the notice; `init`
  keeps its existing prompt-and-confirm behaviour.
- A `cargo install` binary is refused with `cargo install --force texforge`
  guidance instead of being overwritten; `texforge uninstall` now reads the
  same cargo-install detection.
- The pinned Tectonic used for automatic installs is now 0.17.0.
