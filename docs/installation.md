---
title: Installation
description: Install texforge with the quick installer, cargo, or from source.
order: 2
---

# Installation

## Quick install (recommended)

**Linux / macOS:**

```bash
curl -fsSL https://raw.githubusercontent.com/UniverLab/texforge/main/scripts/install.sh | sh
```

**Windows (PowerShell):**

```powershell
irm https://raw.githubusercontent.com/UniverLab/texforge/main/scripts/install.ps1 | iex
```

This downloads a precompiled binary — no Rust toolchain required. Tectonic
(the LaTeX engine) is installed automatically on first build.

The installer accepts environment variables:

```bash
# Pin a specific version
VERSION=0.1.0 curl -fsSL https://raw.githubusercontent.com/UniverLab/texforge/main/scripts/install.sh | sh

# Install to a custom directory
INSTALL_DIR=/usr/local/bin curl -fsSL https://raw.githubusercontent.com/UniverLab/texforge/main/scripts/install.sh | sh
```

```powershell
# Pin a specific version (PowerShell)
$env:VERSION="0.1.0"; irm https://raw.githubusercontent.com/UniverLab/texforge/main/scripts/install.ps1 | iex
```

## Via cargo

```bash
cargo install texforge
```

Available on [crates.io](https://crates.io/crates/texforge).

## Updating texforge

The explicit command is the primary path:

```bash
texforge update            # shows the newer version and asks (default answer: NO)
texforge update --check    # exit 1 if an update is available, 0 if up to date, 2 if the check could not be completed
texforge update --yes      # install without asking
```

`texforge update` downloads the release asset for your platform, verifies its
SHA256 checksum when the release ships one, and replaces the binary that is
running (typically `~/.local/bin/texforge` for installer installs) atomically
and in place. It never updates silently: you are always asked first, and
declining leaves everything untouched. If the network or the GitHub API is
unavailable, the command exits `2` with the cause on stderr — a failed check
never reads as "you are up to date" and never reads as an update being
available.

`texforge init` still shows a passive notice (at most once a day) when a
newer release exists; that notice never downloads unless you answer yes.

**If you installed via `cargo install`:**

Self-update is deliberately disabled — cargo owns that installation path and
tracks its own versions. `texforge update` refuses and tells you to run:

```bash
cargo install --force texforge
```

**Why it matters:** Mixing install methods leaves two binaries on your system. The one on your PATH may not be the one that updated, leading to confusing version mismatches. Choose one method and stick with it.

## From source

```bash
git clone https://github.com/UniverLab/texforge.git
cd texforge
cargo build --release
# Binary at target/release/texforge
```

## GitHub Releases

Precompiled binaries for Linux x86_64, macOS x86_64/ARM64 and Windows
x86_64 are published on the
[Releases](https://github.com/UniverLab/texforge/releases) page.

## Platform support

| Platform | Architecture | Status |
|---|---|---|
| Linux | x86_64 | ✅ |
| macOS | x86_64 | ✅ |
| macOS | ARM64 (Apple Silicon) | ✅ |
| Windows | x86_64 | ✅ |

## Uninstall

To remove everything texforge manages (Tectonic engine, template cache, dictionary cache, configuration):

```bash
texforge uninstall
```

This shows what it would remove and asks for confirmation. The personal spell dictionary (`~/.texforge/spell-words`) is preserved by default — it contains your own writing. To remove it as well:

```bash
texforge uninstall --include-spell-words
```

The texforge binary itself is not removed by this command. To remove it:

- **If installed via the quick installer or a direct download:** `rm -f ~/.local/bin/texforge` (or the path shown by `texforge uninstall`).
- **If installed via cargo:** `cargo uninstall texforge`.
