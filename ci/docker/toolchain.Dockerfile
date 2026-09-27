# escape=`
# Installway, toolchain: Rust, MSVC and the Installway sources in a Windows
# container. Each run compiles the installer stub with YOUR public key, then
# packs the project mounted at C:\work (installway.toml) and signs it with
# your private key:
#
#   docker run --rm -v "${PWD}:C:\work" `
#       -e INSTALLWAY_PRIV_KEY=<hex> -e INSTALLWAY_PUB_KEY=<hex> `
#       ghcr.io/tooltip-focus/installway:toolchain
#
# Build context: the repository root.
FROM mcr.microsoft.com/windows/servercore:ltsc2022

SHELL ["cmd", "/S", "/C"]

# MSVC, the Windows SDK (import libraries, delayimp.lib) and CMake (PakLib,
# found by common/build.rs through vswhere).
RUN curl -fSL -o vs_buildtools.exe https://aka.ms/vs/17/release/vs_buildtools.exe `
    && (start /w vs_buildtools.exe --quiet --wait --norestart --nocache `
        --installPath "%ProgramFiles(x86)%\Microsoft Visual Studio\2022\BuildTools" `
        --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended `
        --add Microsoft.VisualStudio.Component.VC.CMake.Project `
        || IF "%ERRORLEVEL%"=="3010" EXIT 0) `
    && del /q vs_buildtools.exe

# Git, for vcpkg: PakLib pins its zstd/xxHash with a builtin-baseline, which
# needs a full vcpkg clone.
ARG MINGIT_VERSION=2.47.1
RUN curl -fSL -o mingit.zip https://github.com/git-for-windows/git/releases/download/v%MINGIT_VERSION%.windows.1/MinGit-%MINGIT_VERSION%-64-bit.zip `
    && mkdir C:\mingit && tar -xf mingit.zip -C C:\mingit && del /q mingit.zip

ENV RUSTUP_HOME=C:\rustup `
    CARGO_HOME=C:\cargo `
    VCPKG_ROOT=C:\vcpkg
RUN curl -fSL -o rustup-init.exe https://static.rust-lang.org/rustup/dist/x86_64-pc-windows-msvc/rustup-init.exe `
    && rustup-init.exe -y --no-modify-path --profile minimal --default-toolchain stable `
    && del /q rustup-init.exe

SHELL ["powershell", "-NoProfile", "-Command", "$ErrorActionPreference = 'Stop';"]
RUN [Environment]::SetEnvironmentVariable('Path', $env:Path + ';C:\cargo\bin;C:\mingit\cmd', 'Machine')

RUN git clone --quiet https://github.com/microsoft/vcpkg C:\vcpkg; `
    if ($LASTEXITCODE) { exit $LASTEXITCODE }; `
    & C:\vcpkg\bootstrap-vcpkg.bat -disableMetrics; `
    if ($LASTEXITCODE) { exit $LASTEXITCODE }

COPY . C:/src
WORKDIR C:/src

# Compile everything once, both stub variants, with a throwaway key: the
# dependencies are then cached and a run only rebuilds the stub, offline.
RUN $env:INSTALLER_PUB_KEY = '11' * 32; `
    cargo build --release --locked -p installer_builder -p installer -p uninstaller; `
    if ($LASTEXITCODE) { exit $LASTEXITCODE }; `
    cargo build --release --locked -p installer -p uninstaller --features installer/hintway,uninstaller/hintway; `
    if ($LASTEXITCODE) { exit $LASTEXITCODE }

COPY ci/docker/entrypoint.ps1 C:/installway/entrypoint.ps1

WORKDIR C:/work
ENTRYPOINT ["powershell", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", "C:\\installway\\entrypoint.ps1"]
