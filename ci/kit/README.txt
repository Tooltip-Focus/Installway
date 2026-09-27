Installway portable kit
=======================

Build Windows installers without Rust or the Installway sources.
Documentation: https://tooltip-focus.github.io/Installway/building/ci.html


Quick start
-----------

1. Unzip the kit. If Windows blocks the files (downloaded from the internet):

       Get-ChildItem <kit folder> | Unblock-File

2. Copy installway.example.toml into your project folder as installway.toml
   and adapt it (product, version, input folder, output path...).

3. From your project folder, run:

       <kit folder>\pack.cmd

   or, from PowerShell:

       <kit folder>\pack.ps1
       <kit folder>\pack.ps1 --config other.toml --to-version 1.2.3

   Extra arguments are passed to `installer_builder pack` and override the
   config file.

4. Sign the produced .exe with your Authenticode certificate (signtool), so
   SmartScreen and antivirus engines trust it.


Contents
--------

    installer_builder.exe   the packer
    installer.exe           installer stub
    uninstall.exe           uninstaller
    installer-hintway.exe   installer stub with Hintway install analytics
    uninstall-hintway.exe   uninstaller with Hintway install analytics
    hdiffz.exe              binary diff tool, for patch installers
    priv.key, pub.key       the kit's shared signing key pair
    pack.ps1, pack.cmd      wrappers around `installer_builder pack`

pack.ps1 picks the Hintway pair when the config sets `hintway_tenant_id`,
the plain pair otherwise.


Security: shared key
--------------------

Every copy of this kit carries the same signing key, and it is public. The
Ed25519 signature of installers built with the kit therefore proves nothing
about who built them: anyone with the kit can produce an installer these
stubs accept. File integrity (BLAKE3) is still checked, and your Authenticode
signature is what identifies you as the publisher.

For your own key, use the `-toolchain` Docker image, or build the stubs from
source (see "Packaging without the Rust toolchain" in the documentation).
