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

Texforge updates itself in place, **replacing the binary that is running**.

The check runs when you create or migrate a project with `texforge init` — not on every command, and never during a build. If a newer release exists, you are asked once, there and then. To update outside that flow, reach for the installer or `cargo` as below.

**If you used the quick installer or downloaded a binary directly:**

Accept the update prompt, and it will overwrite the binary at its current location (typically `~/.local/bin/texforge`).

**If you installed via `cargo install`:**

Self-update is deliberately disabled — cargo owns that installation path and tracks its own versions. To update, run:

```bash
cargo install --force texforge
```

**Why it matters:** Mixing install methods leaves two binaries on your system. The one on your PATH may not be the one that updated, leading to confusing version mismatches. Choose one method and stick with it.

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

```bash
rm -f ~/.local/bin/texforge   # texforge binary
rm -rf ~/.texforge/           # tectonic engine + cached templates
```
