# Pack a tiny sample project with the portable kit or with a Docker image,
# once without and once with `hintway_tenant_id`, and check that the matching
# stub variant was picked and that each installer passes its own `--verify`.
# Used by build-kit.ps1 and the release workflow; runnable locally.
#
#   ./scripts/test-packaging.ps1 -KitDir dist\kit
#   ./scripts/test-packaging.ps1 -Image installway:dev
#   ./scripts/test-packaging.ps1 -Image installway:dev-toolchain -CustomKey
#
# -CustomKey generates a fresh key pair and passes it to the (toolchain) image
# through INSTALLWAY_PRIV_KEY / INSTALLWAY_PUB_KEY.
param(
    [string]$KitDir,
    [string]$Image,
    [switch]$CustomKey
)
$ErrorActionPreference = 'Stop'

function Assert-Exit($desc) {
    if ($LASTEXITCODE -ne 0) { throw "$desc failed (exit $LASTEXITCODE)" }
}

# Run a command, echo its output and return it as strings. Native stderr is
# merged in, so relax the error preference while it runs.
function Invoke-Logged([scriptblock]$Command) {
    $ErrorActionPreference = 'Continue'
    $log = & $Command 2>&1 | ForEach-Object { "$_" }
    $log | Write-Host
    $log
}

if ([bool]$KitDir -eq [bool]$Image) { throw 'Pass exactly one of -KitDir or -Image' }
if ($CustomKey -and -not $Image) { throw '-CustomKey needs -Image' }
if ($KitDir) { $KitDir = (Resolve-Path $KitDir).Path }

$root = Resolve-Path (Join-Path $PSScriptRoot '..')
# RUNNER_TEMP on CI: GetTempPath() can be an 8.3 short path, which Docker mounts reject.
$tempBase = if ($env:RUNNER_TEMP) { $env:RUNNER_TEMP } else { [System.IO.Path]::GetTempPath() }
$work = Join-Path $tempBase "installway-pkgtest-$PID"
New-Item -ItemType Directory -Force $work | Out-Null
try {
    $dockerEnv = @()
    if ($CustomKey) {
        Write-Host '== keygen (custom key) =='
        docker run --rm -v "${work}:C:\work" --entrypoint 'C:\src\target\release\installer_builder.exe' $Image keygen --out 'C:\work\keys'
        Assert-Exit 'keygen'
        $priv = (Get-Content (Join-Path $work 'keys\priv.key')).Trim()
        $pub = (Get-Content (Join-Path $work 'keys\pub.key')).Trim()
        $dockerEnv = @('-e', "INSTALLWAY_PRIV_KEY=$priv", '-e', "INSTALLWAY_PUB_KEY=$pub")
    }

    foreach ($hintway in $false, $true) {
        $variant = if ($hintway) { 'on' } else { 'off' }
        Write-Host "== pack (hintway $variant) =="

        $project = Join-Path $work "hintway-$variant"
        New-Item -ItemType Directory -Force (Join-Path $project 'app\bin') | Out-Null
        Copy-Item (Join-Path $root 'test\v1.0\bin\app.exe') (Join-Path $project 'app\bin\app.exe')
        'hello' | Set-Content (Join-Path $project 'app\readme.txt')
        $toml = @(
            'product      = "Packaging Test"'
            'product_id   = "pkgtest"'
            'publisher    = "Installway"'
            'to_version   = "1.0.0"'
            'input        = "app"'
            'exe          = "bin/app.exe"'
            'out          = "out/setup.exe"'
            'skip_license = true'
        )
        if ($hintway) { $toml += 'hintway_tenant_id = "00000000-0000-4000-8000-000000000000"' }
        $toml | Set-Content (Join-Path $project 'installway.toml')

        if ($KitDir) {
            Push-Location $project
            try { $log = Invoke-Logged { & (Join-Path $KitDir 'pack.ps1') } }
            finally { Pop-Location }
        } else {
            $log = Invoke-Logged { docker run --rm -v "${project}:C:\work" @dockerEnv $Image }
        }
        Assert-Exit "pack (hintway $variant)"
        if (-not ($log -match "^Hintway: $variant\b")) {
            throw "expected 'Hintway: $variant' in the pack output"
        }

        & (Join-Path $project 'out\setup.exe') --verify
        Assert-Exit "setup.exe --verify (hintway $variant)"
    }

    Write-Host 'PACKAGING OK'
}
finally {
    Remove-Item -Recurse -Force $work -ErrorAction Ignore
}
