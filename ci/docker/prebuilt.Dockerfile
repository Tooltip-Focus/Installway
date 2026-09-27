# escape=`
# Installway, prebuilt: the portable kit (shared signing key) in a Windows
# container. Mount a project folder containing installway.toml at C:\work:
#
#   docker run --rm -v "${PWD}:C:\work" ghcr.io/tooltip-focus/installway
#
# Build context: ci/docker, with the kit from scripts/build-kit.ps1 in ci/docker/kit.
FROM mcr.microsoft.com/windows/servercore:ltsc2022

COPY kit/ C:/installway/
COPY entrypoint.ps1 C:/installway/entrypoint.ps1

WORKDIR C:/work
ENTRYPOINT ["powershell", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", "C:\\installway\\entrypoint.ps1"]
