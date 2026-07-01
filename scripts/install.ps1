# install.ps1 — download and install texforge on Windows
# Usage: irm https://raw.githubusercontent.com/UniverLab/texforge/main/scripts/install.ps1 | iex
#
# Options (set as env vars before running):
#   $env:VERSION    = "0.1.0"           # pin a specific version
#   $env:INSTALL_DIR = "C:\\my\\bin"      # custom install directory

$ErrorActionPreference = "Stop"

$Repo       = "UniverLab/texforge"
$Binary     = "texforge.exe"
$Target     = "x86_64-pc-windows-msvc"
$InstallDir = if ($env:INSTALL_DIR) { $env:INSTALL_DIR } else { "$env:USERPROFILE\.local\bin" }

function Info($label, $msg) {
    Write-Host "  " -NoNewline
    Write-Host $label -ForegroundColor Blue -NoNewline
    Write-Host " $msg"
}

function Fail($msg) {
    Write-Host "  error: $msg" -ForegroundColor Red
    exit 1
}

# --- resolve version ---
if ($env:VERSION) {
    $Tag = "v$($env:VERSION)"
    Info "version" "$Tag (pinned)"
} else {
    # Get latest stable release (exclude prerelease)
    $releases = Invoke-RestMethod "https://api.github.com/repos/$Repo/releases"
    $stable = $releases | Where-Object { -not $_.prerelease } | Select-Object -First 1
    if ($stable) {
        $Tag = $stable.tag_name
    } else {
        # Fallback to latest if no stable found
        $latest = Invoke-RestMethod "https://api.github.com/repos/$Repo/releases/latest"
        $Tag = $latest.tag_name
    }
    if (-not $Tag) { Fail "Could not resolve latest stable release" }
    Info "version" "$Tag (latest stable)"
}

# --- download ---
$Archive = "texforge-$Tag-$Target.zip"
$Url     = "https://github.com/$Repo/releases/download/$Tag/$Archive"
$Tmp     = Join-Path $env:TEMP "texforge-install"
New-Item -ItemType Directory -Force -Path $Tmp | Out-Null

Info "download" $Url
try {
    Invoke-WebRequest -Uri $Url -OutFile "$Tmp\$Archive" -UseBasicParsing
} catch {
    Fail "Download failed: $_`nURL: $Url"
}

# --- verify checksum ---
# Match against SHA256SUMS.txt from the same release. Missing sums file (older
# releases) -> skip; a present-but-mismatched checksum is fatal.
$SumsUrl = "https://github.com/$Repo/releases/download/$Tag/SHA256SUMS.txt"
$SumsOk  = $false
try {
    Invoke-WebRequest -Uri $SumsUrl -OutFile "$Tmp\SHA256SUMS.txt" -UseBasicParsing
    $SumsOk = $true
} catch {
    Info "checksum" "SHA256SUMS.txt not found for $Tag - skipping verification"
}
if ($SumsOk) {
    $line = Select-String -Path "$Tmp\SHA256SUMS.txt" -Pattern ([regex]::Escape($Archive)) | Select-Object -First 1
    if (-not $line) { Fail "No checksum listed for $Archive in SHA256SUMS.txt" }
    $expected = ($line.Line -split '\s+')[0].ToLower()
    $actual   = (Get-FileHash "$Tmp\$Archive" -Algorithm SHA256).Hash.ToLower()
    if ($actual -ne $expected) { Fail "Checksum mismatch for $Archive (expected $expected, got $actual)" }
    Info "checksum" "verified"
}

# --- extract ---
Expand-Archive -Path "$Tmp\$Archive" -DestinationPath $Tmp -Force
$extracted = Join-Path $Tmp "texforge.exe"
if (-not (Test-Path $extracted)) { Fail "Binary not found in archive" }

# --- install ---
New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
Copy-Item $extracted "$InstallDir\texforge.exe" -Force
Info "installed" "$InstallDir\texforge.exe"

# --- ensure PATH ---
$userPath = [Environment]::GetEnvironmentVariable("PATH", "User")
if ($userPath -notlike "*$InstallDir*") {
    [Environment]::SetEnvironmentVariable("PATH", "$InstallDir;$userPath", "User")
    $env:PATH = "$InstallDir;$env:PATH"
    Info "updated" "User PATH"
}

# --- cleanup ---
Remove-Item $Tmp -Recurse -Force

# --- install the texforge agent skill (optional) ---
# Teaches AI agents how to drive texforge. Skipped when npx is unavailable or
# $env:SKIP_SKILL is set; a failure here never fails the binary install above.
$Skill      = "texforge"
$SkillsRepo = "https://github.com/UniverLab/skills"
if ($env:SKIP_SKILL) {
    Info "skill" "skipped (SKIP_SKILL set)"
} elseif (Get-Command npx -ErrorAction SilentlyContinue) {
    Info "skill" "adding '$Skill' (npx skills add)"
    try {
        & npx -y skills add $SkillsRepo --skill $Skill
        if ($LASTEXITCODE -eq 0) {
            Info "skill" "installed"
        } else {
            Info "skill" "skipped - add later with: npx skills add $SkillsRepo --skill $Skill"
        }
    } catch {
        Info "skill" "skipped - add later with: npx skills add $SkillsRepo --skill $Skill"
    }
} else {
    Info "skill" "npx not found - add later with: npx skills add $SkillsRepo --skill $Skill"
}

# --- verify ---
$ver = & "$InstallDir\texforge.exe" --version 2>$null
Info "done" $ver
Write-Host ""
Info "ready" "Run 'texforge --help' to get started!"
