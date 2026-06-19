# Build a real installer that exercises plugin-contributed wizard pages.
# Uses the committed test key pair (test/keys). Output:
#   test/out/setup-plugin-pages.exe
# Run that, pick a folder, choose a country on the plugin page, finish; then check
# <install_dir>/selected-country.txt for the recorded choice.
$ErrorActionPreference = 'Stop'

function Assert-Exit($desc) {
    if ($LASTEXITCODE -ne 0) { throw "$desc failed (exit $LASTEXITCODE)" }
}

$root = Resolve-Path (Join-Path $PSScriptRoot '..')
Set-Location $root

$demo    = Join-Path $root 'test\plugin-pages'
$plugins = Join-Path $demo 'plugins'
$src     = Join-Path $demo 'src'
New-Item -ItemType Directory -Force $plugins | Out-Null
New-Item -ItemType Directory -Force (Join-Path $src 'bin') | Out-Null
New-Item -ItemType Directory -Force (Join-Path $root 'test\out') | Out-Null

Write-Host '== build installer_builder =='
cargo build --release -p installer_builder
Assert-Exit 'cargo build installer_builder'
$bld = Join-Path $root 'target\release\installer_builder.exe'

Write-Host '== build country_picker plugin =='
cargo build --release --manifest-path (Join-Path $root 'sdk\examples\country_picker\Cargo.toml')
Assert-Exit 'cargo build country_picker'
Copy-Item (Join-Path $root 'sdk\examples\country_picker\target\release\country_picker.dll') `
    (Join-Path $plugins 'country_picker.dll') -Force

Write-Host '== build stub + uninstaller WITH the committed test key =='
$env:INSTALLER_PUB_KEY = (Get-Content (Join-Path $root 'test\keys\pub.key')).Trim()
cargo build --release -p installer -p uninstaller
$buildCode = $LASTEXITCODE
Remove-Item Env:\INSTALLER_PUB_KEY
if ($buildCode -ne 0) { throw "cargo build installer/uninstaller failed (exit $buildCode)" }

Write-Host '== stage a tiny payload =='
'hello from the plugin-pages demo' | Set-Content (Join-Path $src 'readme.txt')
Copy-Item $bld (Join-Path $src 'bin\app.exe') -Force   # any PE works as the packaged "app"

Write-Host '== pack (config carries the [[plugin]] ui = true) =='
& $bld pack --config (Join-Path $demo 'pack.toml')
Assert-Exit 'pack'

$setup = Join-Path $root 'test\out\setup-plugin-pages.exe'
Write-Host ''
Write-Host "INSTALLER READY: $setup"
Write-Host 'Run it: pick a folder, choose a country on the plugin page, finish.'
Write-Host 'Then check <install_dir>\selected-country.txt for the recorded choice.'
