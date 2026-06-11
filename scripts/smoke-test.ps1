# End-to-end smoke test: build a toolchain-free kit (stub + uninstaller with a
# real key), pack an installer with it, and run `--verify`. Asserts exit code 0.
# Runnable locally and in CI.
$ErrorActionPreference = 'Stop'

function Assert-Exit($desc) {
    if ($LASTEXITCODE -ne 0) { throw "$desc failed (exit $LASTEXITCODE)" }
}

$root = Resolve-Path (Join-Path $PSScriptRoot '..')
Set-Location $root

$work = Join-Path ([System.IO.Path]::GetTempPath()) "installway-smoke-$PID"
New-Item -ItemType Directory -Force $work | Out-Null
try {
    Write-Host '== build installer_builder =='
    cargo build --release -p installer_builder
    Assert-Exit 'cargo build installer_builder'
    $bld = Join-Path $root 'target\release\installer_builder.exe'

    Write-Host '== keygen =='
    & $bld keygen --out (Join-Path $work 'keys')
    Assert-Exit 'keygen'

    Write-Host '== build stub + uninstaller WITH the key =='
    $env:INSTALLER_PUB_KEY = (Get-Content (Join-Path $work 'keys\pub.key')).Trim()
    cargo build --release -p installer -p uninstaller
    $buildCode = $LASTEXITCODE
    Remove-Item Env:\INSTALLER_PUB_KEY
    if ($buildCode -ne 0) { throw "cargo build installer/uninstaller failed (exit $buildCode)" }

    Write-Host '== stage a tiny payload =='
    $src = Join-Path $work 'src'
    New-Item -ItemType Directory -Force (Join-Path $src 'bin') | Out-Null
    'hello' | Set-Content (Join-Path $src 'readme.txt')
    Copy-Item $bld (Join-Path $src 'bin\app.exe')   # any PE works as the packaged "app"

    Write-Host '== pack (toolchain-free: prebuilt stub + uninstaller, no --pub-key) =='
    $setup = Join-Path $work 'setup.exe'
    & $bld pack `
        --product 'Smoke App' --product-id smokeapp --publisher 'Test' --to-version 1.0.0 `
        --input $src --exe 'bin/app.exe' `
        --installer-stub (Join-Path $root 'target\release\installer.exe') `
        --uninstaller    (Join-Path $root 'target\release\uninstall.exe') `
        --priv-key       (Join-Path $work 'keys\priv.key') `
        --out $setup
    Assert-Exit 'pack'

    Write-Host '== verify =='
    & $setup --verify
    Assert-Exit 'setup.exe --verify'

    Write-Host 'SMOKE OK'
}
finally {
    Remove-Item -Recurse -Force $work -ErrorAction Ignore
}
