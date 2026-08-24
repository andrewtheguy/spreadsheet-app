<#
.SYNOPSIS
Builds the Windows MSI from packaging/windows/main.wxs.

.DESCRIPTION
Tauri used to produce the installer; with the Slint rewrite this script does it. It downloads
the WiX v3 binaries on demand rather than relying on them being present, so it behaves the same
on a CI runner and on a developer box.

Expects the release binary to already exist (cargo build --release).

.EXAMPLE
pwsh scripts/bundle-windows.ps1 -ExePath target/release/spreadsheet-app.exe -OutDir target/windows
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string] $ExePath,
    [string] $OutDir = "target/windows",
    # Pinned so a WiX release can't silently change the installer we ship.
    [string] $WixVersion = "3.14.1",
    [string] $WixUrl = "https://github.com/wixtoolset/wix3/releases/download/wix3141rtm/wix314-binaries.zip",
    # SHA-256 of wix314-binaries.zip, so a substituted or truncated download can't be unpacked
    # and run as part of the installer build. Update it whenever $WixUrl changes.
    [string] $WixSha256 = "6AC824E1642D6F7277D0ED7EA09411A508F6116BA6FAE0AA5F2C7DAA2FF43D31"
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$repoRoot = Split-Path -Parent $PSScriptRoot

if (-not (Test-Path $ExePath)) {
    throw "binary not found: $ExePath (run cargo build --release first)"
}

# Take the version from Cargo.toml so the MSI can't drift from the crate.
$cargoToml = Join-Path $repoRoot "Cargo.toml"
$version = (Select-String -Path $cargoToml -Pattern '^version = "(.*)"' | Select-Object -First 1).Matches[0].Groups[1].Value
if (-not $version) { throw "could not read version from $cargoToml" }

# --- WiX toolset ------------------------------------------------------------------------
$wixDir = Join-Path $repoRoot ".wix/$WixVersion"
if (-not (Test-Path (Join-Path $wixDir "candle.exe"))) {
    Write-Host "Downloading WiX $WixVersion"
    New-Item -ItemType Directory -Force -Path $wixDir | Out-Null
    $zip = Join-Path ([System.IO.Path]::GetTempPath()) "wix-$WixVersion.zip"
    Invoke-WebRequest -UseBasicParsing -Uri $WixUrl -OutFile $zip
    $actual = (Get-FileHash -Algorithm SHA256 -Path $zip).Hash
    if ($actual -ne $WixSha256) {
        Remove-Item $zip -ErrorAction SilentlyContinue
        throw "WiX download hash mismatch: expected $WixSha256, got $actual"
    }
    Expand-Archive -Force -Path $zip -DestinationPath $wixDir
    Remove-Item $zip
}

# --- Build ------------------------------------------------------------------------------
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
$msi = Join-Path $OutDir "spreadsheet-app-$version-x86_64.msi"
$wixobj = Join-Path $OutDir "main.wixobj"
$icon = Join-Path $repoRoot "icons/icon.ico"
$wxs = Join-Path $repoRoot "packaging/windows/main.wxs"

& (Join-Path $wixDir "candle.exe") `
    -nologo -arch x64 `
    -dVersion="$version" `
    -dExePath="$((Resolve-Path $ExePath).Path)" `
    -dIconPath="$icon" `
    -out $wixobj $wxs
if ($LASTEXITCODE -ne 0) { throw "candle failed with exit code $LASTEXITCODE" }

# WixUIExtension supplies the WixUI_InstallDir dialog set referenced by main.wxs. ICE61 is
# suppressed because MajorUpgrade with `AllowSameVersionUpgrades` unset trips it on a
# same-version rebuild, which is exactly what CI does.
& (Join-Path $wixDir "light.exe") `
    -nologo -ext WixUIExtension -sice:ICE61 `
    -out $msi $wixobj
if ($LASTEXITCODE -ne 0) { throw "light failed with exit code $LASTEXITCODE" }

Remove-Item $wixobj -ErrorAction SilentlyContinue
Remove-Item (Join-Path $OutDir "*.wixpdb") -ErrorAction SilentlyContinue

Write-Host "built $msi (version $version)"
