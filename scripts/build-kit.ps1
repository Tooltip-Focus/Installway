# Build the portable Installway kit: the builder, plus the installer stub and
# the uninstaller in two variants (plain and Hintway) compiled with the given
# public key, hdiffz, the key pair and the wrappers from ci/kit. Then packs a
# sample project with the kit and runs `--verify` (skip with -SkipTest).
# Used by the release workflow; runnable locally.
#
#   ./scripts/build-kit.ps1 -PrivKeyHex <hex> -PubKeyHex <hex> [-OutDir dist\kit] [-SkipTest]
param(
    [Parameter(Mandatory)][string]$PrivKeyHex,
    [Parameter(Mandatory)][string]$PubKeyHex,
    [string]$OutDir = 'dist\kit',
    [switch]$SkipTest
)
$ErrorActionPreference = 'Stop'

function Assert-Exit($desc) {
    if ($LASTEXITCODE -ne 0) { throw "$desc failed (exit $LASTEXITCODE)" }
}

function Assert-KeyHex($name, $value) {
    if ($value -notmatch '^[0-9a-fA-F]{64}$') { throw "$name must be 64 hex chars (32 bytes)" }
}

$root = Resolve-Path (Join-Path $PSScriptRoot '..')
Set-Location $root

$PrivKeyHex = $PrivKeyHex.Trim().ToLowerInvariant()
$PubKeyHex = $PubKeyHex.Trim().ToLowerInvariant()
Assert-KeyHex 'PrivKeyHex' $PrivKeyHex
Assert-KeyHex 'PubKeyHex' $PubKeyHex

if (-not [System.IO.Path]::IsPathRooted($OutDir)) { $OutDir = Join-Path $root $OutDir }
if ((Test-Path $OutDir) -and (Get-ChildItem $OutDir -Force | Select-Object -First 1)) {
    throw "$OutDir is not empty"
}
New-Item -ItemType Directory -Force $OutDir | Out-Null

$release = Join-Path $root 'target\release'
$env:INSTALLER_PUB_KEY = $PubKeyHex
try {
    Write-Host '== build builder + stub + uninstaller (plain) =='
    cargo build --release --locked -p installer_builder -p installer -p uninstaller
    Assert-Exit 'cargo build (plain)'
    foreach ($exe in 'installer_builder.exe', 'installer.exe', 'uninstall.exe') {
        Copy-Item (Join-Path $release $exe) $OutDir
    }

    # Same target dir: copy the plain pair out before this build overwrites it.
    Write-Host '== build stub + uninstaller (hintway) =='
    cargo build --release --locked -p installer -p uninstaller --features installer/hintway,uninstaller/hintway
    Assert-Exit 'cargo build (hintway)'
    Copy-Item (Join-Path $release 'installer.exe') (Join-Path $OutDir 'installer-hintway.exe')
    Copy-Item (Join-Path $release 'uninstall.exe') (Join-Path $OutDir 'uninstall-hintway.exe')
}
finally {
    Remove-Item Env:\INSTALLER_PUB_KEY -ErrorAction Ignore
}

Copy-Item (Join-Path $root 'vendor\hdiffpatch\hdiffz.exe') $OutDir
Set-Content -NoNewline -Encoding ascii (Join-Path $OutDir 'priv.key') $PrivKeyHex
Set-Content -NoNewline -Encoding ascii (Join-Path $OutDir 'pub.key') $PubKeyHex
Copy-Item (Join-Path $root 'ci\kit\*') $OutDir
Write-Host "Kit written to $OutDir"

if (-not $SkipTest) {
    & (Join-Path $PSScriptRoot 'test-packaging.ps1') -KitDir $OutDir
    Assert-Exit 'kit test'
}
