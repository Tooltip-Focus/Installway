# Build a runnable test installer with the "uninstall complete" message box
# ENABLED, so you can install then uninstall and verify the popup behavior.
# Output: test-build\setup.exe (kept, not cleaned up).
$ErrorActionPreference = 'Stop'

function Assert-Exit($desc) {
    if ($LASTEXITCODE -ne 0) { throw "$desc failed (exit $LASTEXITCODE)" }
}

$root = Resolve-Path (Join-Path $PSScriptRoot '..')
Set-Location $root

$out = Join-Path $root 'test-build'
New-Item -ItemType Directory -Force $out | Out-Null

Write-Host '== build installer_builder =='
cargo build --release -p installer_builder
Assert-Exit 'cargo build installer_builder'
$bld = Join-Path $root 'target\release\installer_builder.exe'

Write-Host '== keygen =='
& $bld keygen --out (Join-Path $out 'keys')
Assert-Exit 'keygen'

Write-Host '== build stub + uninstaller WITH the key =='
$env:INSTALLER_PUB_KEY = (Get-Content (Join-Path $out 'keys\pub.key')).Trim()
cargo build --release -p installer -p uninstaller
$buildCode = $LASTEXITCODE
Remove-Item Env:\INSTALLER_PUB_KEY
if ($buildCode -ne 0) { throw "cargo build installer/uninstaller failed (exit $buildCode)" }

Write-Host '== stage a tiny payload =='
$src = Join-Path $out 'src'
New-Item -ItemType Directory -Force (Join-Path $src 'bin') | Out-Null
'hello' | Set-Content (Join-Path $src 'readme.txt')
Copy-Item $bld (Join-Path $src 'bin\app.exe')   # any PE works as the packaged "app"

Write-Host '== pack (show-uninstall-complete ENABLED) =='
$setup = Join-Path $out 'setup.exe'
& $bld pack `
    --product 'Test App' --product-id testapp --publisher 'Test' --to-version 1.0.0 `
    --input $src --exe 'bin/app.exe' `
    --show-uninstall-complete `
    --installer-stub (Join-Path $root 'target\release\installer.exe') `
    --uninstaller    (Join-Path $root 'target\release\uninstall.exe') `
    --priv-key       (Join-Path $out 'keys\priv.key') `
    --out $setup
Assert-Exit 'pack'

Write-Host ''
Write-Host "INSTALLER READY: $setup"
Write-Host 'Run it to install, then uninstall via Add/Remove Programs (or the'
Write-Host 'install dir uninstall.exe) to see the "uninstall complete" message box.'
