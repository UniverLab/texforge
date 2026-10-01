# Changelog

All notable changes to texforge are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## Unreleased

### Added

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

### Changed

- The release check used by `texforge init` moved into the updater so both
  paths share one lookup, throttled to once a day for the notice; `init`
  keeps its existing prompt-and-confirm behaviour.
- A `cargo install` binary is refused with `cargo install --force texforge`
  guidance instead of being overwritten; `texforge uninstall` now reads the
  same cargo-install detection.
