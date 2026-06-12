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

```bash
rm -f ~/.local/bin/texforge   # texforge binary
rm -rf ~/.texforge/           # tectonic engine + cached templates
```
