# Installway

Local, single-file `.exe` installer in the style of InstallShield / MSI - but
written in Rust and built around a BLAKE3 + HDiffPatch manifest format.

Each output `.exe` carries its own payload: the file zip is appended as a PE
**overlay** (no size ceiling, streamed on at build, mmap-read at install), while
the signed manifest, the uninstaller and the payload length ride as small
`RT_RCDATA` resources. No network, no admin elevation, no MSI runtime.

## Workspace

| Crate | Type | Purpose |
|---|---|---|
| `common` | lib | Manifest types (`Manifest`, `FileEntry`, `PatchInfo`, `InstallerPayload`, `SignedPayload`), BLAKE3 hashing, file scan, HDiffPatch generation wrapper. |
| `installer_builder` | bin | Offline build tool. Generates Ed25519 keypairs and packs a directory (or a `from_dir`/`to_dir` pair) into a self-contained installer `.exe`. |
| `installer` | bin | The installer stub. Loads its own RCDATA payload, verifies the Ed25519 signature against a public key compiled in at build time, checks per-file BLAKE3 hashes, and either fresh-extracts the zip or applies HDiffPatch deltas in place. Modernized Win32 UI (Segoe UI, license page, progress, run-now checkbox). |
| `uninstaller` | bin | Built once by the builder and embedded as the 3rd RCDATA resource. The installer writes it to `%LOCALAPPDATA%\<publisher>\Uninstall\<product>\uninstall.exe` (outside the app folder) and registers it under HKCU `Software\Microsoft\Windows\CurrentVersion\Uninstall`, so the product shows up in Windows Apps. Reads the manifest, removes every tracked file + shortcuts + associations + registry entry, then a `%TEMP%` stage-2 copy deletes the app dir, the data dir, and itself. |

## Security model

The installer carries four overlapping guarantees:

1. **Ed25519 signature** over the exact JSON bytes that describe the payload. The
   public key is **compiled** into the installer stub via the `INSTALLER_PUB_KEY`
   build-time env var - never embedded as a resource, so an attacker cannot swap
   key + payload together.
2. **BLAKE3 of the zip payload** is recorded in the signed manifest and
   re-verified at runtime before any byte is extracted.
3. **BLAKE3 per file** is recorded in `Manifest.files[...].hash` and checked
   after each write (full extract) or patch apply.
4. **Anti-rollback** via `min_installer_version`: if the running stub is older
   than the payload's minimum, install is refused.
5. **Patch from-version pinning**: a patch installer carries `from_version` and
   refuses to run unless the target directory's `version.json` matches.

Authenticode is **not** handled in code - sign the final `.exe` with
`signtool.exe sign /fd SHA256 /tr http://timestamp.digicert.com setup-...exe`
as a separate post-build step (the builder prints the exact command).

## Build

```pwsh
cargo build --release -p installer_builder
```

The installer stub is built on demand by `installer_builder pack` (with the
public key threaded through as an env var). You do not build `installer`
yourself.

If you need HDiffPatch deltas in patch installers, drop `hdiffz.exe` next to
`installer_builder.exe` (in `target\release\`). Without it the builder falls
back to shipping changed files in full and prints a warning.

## Workflow

### 1. Generate a keypair (once per product)

```pwsh
.\target\release\installer_builder.exe keygen --out .\keys
```

Produces `keys\priv.key` (keep secret) and `keys\pub.key`. Every installer you
ship must be signed with the same `priv.key`; the matching `pub.key` is
compiled into every stub.

### 2. Build a fresh installer

```pwsh
.\target\release\installer_builder.exe pack `
    --product myapp `
    --publisher "My Company" `
    --to-version 1.0 `
    --input .\build\myapp-1.0 `
    --exe myapp.exe `
    --priv-key .\keys\priv.key `
    --pub-key  .\keys\pub.key `
    --out .\dist\setup-myapp-1.0.exe
```

`--publisher` is mandatory. It sets the Add/Remove Programs "Publisher" field
and the per-user uninstall data folder (see Uninstall below).

### 3. Build a patch installer

```pwsh
.\target\release\installer_builder.exe pack `
    --product myapp `
    --publisher "My Company" `
    --from-version 1.0 --from-dir .\build\myapp-1.0 `
    --to-version   1.1 --input    .\build\myapp-1.1 `
    --exe myapp.exe `
    --priv-key .\keys\priv.key `
    --pub-key  .\keys\pub.key `
    --out .\dist\patch-myapp-1.0-to-1.1.exe
```

A patch installer carries only the deltas (or the full bytes for files where
the delta would be bigger) plus the list of files to delete. Unchanged files
have no payload entry at all.

### Config file (`--config`)

`pack` takes a long command line. Put any options in a TOML file instead and
pass `--config`. **CLI args override the file**; anything absent from both uses
the built-in default (only `min_installer_version` has one: `1.0.0`).

Flat keys, snake_case, matching the CLI long names. Unknown keys are rejected
(typo guard). Booleans (`force_reinstall`, `reuse_stub`) are `true`/`false`;
`assoc` is an array. A CLI `--assoc` list *replaces* the file's list when given.

```toml
# pack.toml
product    = "myapp"
publisher  = "My Company"
to_version = "1.0"
input      = "build/myapp-1.0"
exe        = "myapp.exe"
license    = "legal/EULA-myapp-en.txt"
assoc      = [".myx:MyApp Document", ".myz:MyApp Archive"]
priv_key   = "keys/priv.key"
pub_key    = "keys/pub.key"
out        = "dist/setup-myapp-1.0.exe"

# default install dir the UI proposes (per-app); %VAR% tokens are expanded.
default_install_dir = "%LOCALAPPDATA%\\Programs\\MyApp"

# trim the wizard (optional)
# skip_license = true   # hide the License page
# skip_path    = true   # hide the Choose-location page, install to default_install_dir

# patch mode (optional)
# from_version = "0.9"
# from_dir     = "build/myapp-0.9"

# dev (optional)
# force_reinstall = true
```

```pwsh
# everything from the file
.\target\release\installer_builder.exe pack --config .\pack.toml

# file as a base, override one value on the CLI
.\target\release\installer_builder.exe pack --config .\pack.toml --to-version 1.1
```

Required (via CLI or file): `product`, `publisher`, `to-version`, `input`,
`exe`, `priv-key`, `out`. A missing one fails with a message naming it.

### Dev: reinstall from scratch

Add `--force-reinstall` to any build (full or patch):

```pwsh
.\target\release\installer_builder.exe pack `
    --product myapp --publisher "My Company" `
    --to-version 1.0 --input .\build\myapp-1.0 --exe myapp.exe `
    --force-reinstall `
    --priv-key .\keys\priv.key --pub-key .\keys\pub.key `
    --out .\dist\setup-myapp-dev.exe
```

The resulting installer:
- **skips the patch from-version check** (installs over any/no existing version);
- **rewrites every file** (no hash-skip — guarantees fresh bytes);
- **removes orphans** (any existing file not part of this build), so the install
  matches the build exactly.

Still fully transactional: orphan removals are backed up and rolled back if the
install fails. Intended for development; ship normal installers to users.

### Toolchain-free packaging (prebuilt kit)

By default `pack` runs `cargo build` to produce the stub + uninstaller, so it
needs the Rust toolchain and this source tree. To let someone package versions
**without Rust**, build the binaries once and hand them a kit.

1. **Once, with the toolchain** (you, the vendor):

   ```pwsh
   $env:INSTALLER_PUB_KEY = Get-Content .\keys\pub.key
   cargo build --release -p installer -p uninstaller -p installer_builder
   ```

   This bakes your public key into `installer.exe`. Collect into a kit folder:
   `installer_builder.exe`, `installer.exe`, `uninstall.exe`, and `priv.key`
   (keep `priv.key` secret - whoever holds it can sign installers).

2. **Anytime, no toolchain** (the packager, on any Windows box):

   ```pwsh
   .\installer_builder.exe pack `
       --product myapp --publisher "My Company" --to-version 1.0 `
       --input .\files --exe myapp.exe `
       --installer-stub .\installer.exe `
       --uninstaller   .\uninstall.exe `
       --priv-key      .\priv.key `
       --out .\setup-myapp-1.0.exe
   ```

   `--pub-key` is **not** needed (the stub already has the key compiled in). The
   `--priv-key` must match that baked public key, or the produced installer will
   reject its own payload at runtime. Works for patch builds too (add
   `--from-version`/`--from-dir`). Icon, version-info, overlay and signing all
   behave identically.

## Installation

### Interactive

Double-click the `.exe`. The wizard walks through:

1. **License** - lorem-ipsum EULA (placeholder), "I accept" checkbox gates the Next button.
2. **Choose install location** - default `%LOCALAPPDATA%\Programs\<product>`, with a native `IFileOpenDialog` folder picker.
3. **Progress** - Win11 progress bar + per-file status. Cancel-safe.
4. **Done** - "Run program now" checkbox (defaults to checked when `manifest.exe` is set). Finish launches the installed program via `ShellExecuteW`.

No admin elevation (manifest declares `asInvoker`). Segoe UI font, Common
Controls v6 visual styles, DPI-aware (`PerMonitorV2`).

**Trimming the wizard.** `--skip-license` hides step 1; `--skip-path` hides
step 2 (installs to the default location, no picker). With both, the wizard goes
straight to Progress on launch. With `--skip-path` only, the License page's
button becomes "Install". `--default-install-dir` / `--skip-license` /
`--skip-path` are all `pack` options (CLI or `--config`).

**Already installed?** When a prior install of this product is found (its
`installer_info.json` in the per-user data dir), the installer treats this as a
reinstall/upgrade and **skips the Choose-location page entirely** - for both
full and patch installers - installing into the existing folder. A patch *must*
go there (it `hdiff`s the files already on disk), and a full reinstall there
avoids an accidental second copy in another folder. The build-time `--skip-path`
only matters for *first* installs.

**Proposed location, in priority order:**

1. An explicit path argument (`--silent`/`--minimal "<dir>"` or `INSTALLWAY_PATH`).
2. **The folder the product was last installed to** (reinstall/upgrade lands in
   place; the Choose page is skipped as above).
3. The build's `--default-install-dir` (per-app, `%VAR%` tokens expanded, e.g.
   `%LOCALAPPDATA%\Programs\MyApp`).
4. `%LOCALAPPDATA%\Programs\<product>`.

Detection is keyed by `publisher` + `product`, so it works across versions. A
fresh install (nothing recorded) shows the Choose page as normal (unless the
build set `--skip-path`).

### Minimal (app-triggered self-update)

```pwsh
.\setup-myapp-1.1.exe --minimal "C:\path\to\install"
.\setup-myapp-1.1.exe --minimal "C:\path\to\install" --launch
```

Compact windowed UI for updates the app launches itself. **No license page, no
folder picker, no Install button** - it starts the moment it opens and just
shows progress:

```text
┌────────────────────────────────────────────┐
│  ██      Applying update                    │
│  ██      MyApp 1.1                          │
│          [██████████░░░░░░░]  62%           │
│          Updating bin/app.exe               │
└────────────────────────────────────────────┘
```

App icon on the left (extracted from the installer's own embedded icon), title
+ version + progress bar + current-file status on the right. Closes itself
~0.9 s after reaching 100 %; on error it stays open with the message. Same
data-safe pre-flight as every install (closes the running app first, disk
check, etc.). Path resolves from the argument, `INSTALLWAY_PATH`, or the
default install dir. Implementation: [installer/src/ui/minimal.rs](installer/src/ui/minimal.rs).

### Silent (`/S` style, IT-friendly)

```pwsh
.\setup-myapp-1.0.exe --silent "C:\path\to\install"
.\setup-myapp-1.0.exe --silent "C:\path\to\install" --launch
```

`--launch` runs the installed `manifest.exe` after install (interactive UI
exposes this as the "Run program now" checkbox on the Done page).

Progress is printed to stderr. Exit codes:

| Code | Meaning |
|---|---|
| `0` | success |
| `10` | wrong installed version for this patch (install untouched - run the full installer) |
| `1` | any other failure (bad signature, anti-rollback, hash mismatch, disk, etc.) |

A patch run against the wrong version is a **pre-flight refusal**: nothing is
touched, the existing install keeps working, and the message tells the user to
run the full installer. A launcher can branch on exit code `10` to fetch the
full installer automatically.

### Uninstall

`uninstall.exe` and its metadata live **outside** the application folder, in a
per-user data dir (mirrors InstallShield's "Installation Information" folder):

```
%LOCALAPPDATA%\<publisher>\Uninstall\<product>\
    uninstall.exe
    installer_info.json        (holds the real install_dir)
    installer_manifest.json
```

So a user who deletes the app folder by hand still has a working uninstaller -
**no orphan Add/Remove Programs entry**, like a commercial installer.

The product appears in **Windows Settings > Apps > Installed apps** (and
classic Add/Remove Programs). Uninstalling runs that `uninstall.exe`, which:

1. Reads `installer_info.json` to find the real `install_dir`.
2. Walks `installer_manifest.json` and removes every tracked file from the app dir.
3. Removes desktop / Start Menu shortcuts and file associations.
4. Removes `version.json` + `installer_manifest.json` from the app dir, and empty subdirs.
5. Deletes the HKCU Uninstall registry entry.
6. Spawns a `%TEMP%` copy of itself (stage 2) that deletes the **app dir** and
   the **data dir** (incl. `uninstall.exe`), then schedules its own removal via
   `MoveFileExW(MOVEFILE_DELAY_UNTIL_REBOOT)`. No `cmd.exe`, no console flash.

If the app folder was already deleted, steps 1-2 no-op and the registry entry +
data dir are still cleaned. `uninstall.exe --silent` skips the confirmation
dialog (used by the registry `QuietUninstallString`).

### Inspect without installing

```pwsh
.\setup-myapp-1.0.exe --verify
```

Verifies the embedded payload and prints kind / versions / payload size.

### Verify an installed copy

```pwsh
.\setup-myapp-1.0.exe --verify-install "C:\path\to\install"
```

Re-hashes every file in the installed `installer_manifest.json` and reports
`OK` / `MISSING` / `CORRUPT` per file. Exit code `0` if clean, `1` if anything
is missing or corrupted - handy for support / scripted health checks after a
suspected disk problem.

### Dev: preview a UI view

**Debug builds only.** Render a single installer view with sample data - no
payload, no install - to iterate on layout fast:

```pwsh
cargo build -p installer
.\target\debug\installer.exe --preview <view>
```

`<view>` is one of `license`, `choose`, `progress`, `done`, `error`, or
`minimal`. Append `-patch` (e.g. `--preview choose-patch`) to preview the patch
variant, which shows the `1.1.0 → 1.2.0` subheader. The window opens straight on
that view with a sample "Sample App 1.2.0" payload and no worker thread;
`progress` shows ~62 %, `error` a sample message. `--preview` is compiled out of
release builds. Implementation:
[installer/src/ui/win32/mod.rs](installer/src/ui/win32/mod.rs),
[installer/src/ui/minimal.rs](installer/src/ui/minimal.rs).

## Runtime behavior

For every file in the manifest:

1. If the destination file already exists **and** its BLAKE3 matches the
   manifest - skip. This means a re-run of an installer is effectively
   instant.
2. Else, if this is a patch installer and the destination file exists and the
   manifest has a `PatchInfo` for it - read the patch out of the embedded zip,
   apply HDiffPatch, verify BLAKE3, atomic rename. Fall through to full
   extract if anything fails.
3. Else - read `full/<rel>` out of the embedded zip, verify BLAKE3, atomic
   rename.

Files listed in `Manifest.deleted_files` are removed afterwards.

`version.json` and `installer_manifest.json` are written to the install root -
they are the canonical record of what got installed and double as
state-required by any subsequent patch installer.

## UI

Single modernized Win32 UI - no Tauri, no WebView2, no HTML runtime. Common
Controls v6 visual styles, Segoe UI, DPI-aware `PerMonitorV2`, `asInvoker`
manifest. Deliberate choice: zero runtime dependencies, every supported
Windows version works, ~860 KB stub.

## License text

Pass `--license <path>` to `pack` and the UTF-8 text in that file becomes the
EULA shown on the installer's License page. Omitting it falls back to a
built-in lorem-ipsum placeholder. The text rides inside the signed
`InstallerPayload`, so tampering invalidates the Ed25519 signature.

```pwsh
installer_builder.exe pack `
    --product myapp --to-version 1.0 `
    --input .\build\myapp `
    --exe myapp.exe `
    --license .\legal\EULA-myapp-en.txt `
    --priv-key .\keys\priv.key --pub-key .\keys\pub.key `
    --out .\dist\setup-myapp-1.0.exe
```

`--verify` prints `License: custom (<bytes>)` or `License: built-in placeholder`.

## Version info

Pack stamps a Win32 `VS_VERSIONINFO` (RT_VERSION) resource into the setup.exe,
built from `--product` / `--publisher` / `--to-version`. Explorer's **Details**
tab then shows FileVersion, ProductVersion, ProductName, CompanyName,
FileDescription (`<product> Setup`), OriginalFilename and copyright - a complete
binary that looks finished and builds SmartScreen reputation. `--to-version`
is parsed as `a.b.c.d` (missing parts = 0). Implementation:
[installer_builder/src/version.rs](installer_builder/src/version.rs).

## Icon inheritance

At pack time the builder reads `RT_GROUP_ICON` + every referenced `RT_ICON`
from `<input>/<exe>` (the app being packaged) via
`LoadLibraryExW(LOAD_LIBRARY_AS_DATAFILE)` and stamps them into both
`setup-...exe` and the embedded `uninstall.exe` via
`BeginUpdateResourceW / UpdateResourceW / EndUpdateResourceW`. Result:
Windows Explorer shows the application's own icon on the installer and
uninstaller files, and on the Add/Remove Programs entry (the registry
`DisplayIcon` already points at `uninstall.exe`).

The uninstaller is stamped in a staging copy under `%TEMP%`, then read into
the installer payload - the cached `target/release/uninstall.exe` is left
untouched between pack runs. If the source exe has no icon resources, the
build prints a notice and falls back to the Rust default.

## File associations

Pass `--assoc ".ext:Description"` (repeatable) to register file types under
`HKCU\Software\Classes` - per-user, no admin. The shell `open` verb points at
the installed `manifest.exe` with `"%1"`.

```pwsh
installer_builder.exe pack `
    --product MyApp --to-version 1.0 `
    --input .\build\myapp --exe myapp.exe `
    --assoc ".myx:MyApp Document" `
    --assoc ".myz:MyApp Archive" `
    --priv-key .\keys\priv.key --pub-key .\keys\pub.key `
    --out .\dist\setup-myapp-1.0.exe
```

Keys written per association (ProgID = `<sanitized-product>.<ext>`):

```text
HKCU\Software\Classes\.myx                          (default) = MyApp.myx
HKCU\Software\Classes\MyApp.myx                      (default) = MyApp Document
HKCU\Software\Classes\MyApp.myx\DefaultIcon          (default) = "<exe>",0
HKCU\Software\Classes\MyApp.myx\shell\open\command   (default) = "<exe>" "%1"
```

`SHChangeNotify(SHCNE_ASSOCCHANGED)` fires so Explorer refreshes immediately.
The chosen associations are recorded in `installer_info.json`; the uninstaller
removes exactly those ProgID trees and clears each `.ext` default **only if it
still points at our ProgID** (never stomping an association the user later
re-pointed). Shared `progid_for` in `common::assoc` keeps installer and
uninstaller in lock-step. Implementation: [common/src/assoc.rs](common/src/assoc.rs).

## Shortcuts

If the payload `manifest.exe` is non-empty, the installer drops two `.lnk`
files per user (no admin needed) pointing at `<install_dir>\<exe>`:

- `%APPDATA%\Microsoft\Windows\Start Menu\Programs\<product>.lnk`
- `%USERPROFILE%\Desktop\<product>.lnk`

Same code path as the launcher (`mslnk::ShellLink`). Both are removed by the
uninstaller. Path logic lives in `common::shortcuts` so installer and
uninstaller never drift apart on file naming.

## Limitations / V1 scope

- Windows only.
- No GUI folder picker on Windows < 7 (we use modern `IFileOpenDialog`).
## Closing the running app

When installing over an existing version, the installer first makes sure no
copy of the target exe (`manifest.exe`, matched by full path inside the
install dir, file-name fallback) is still running - otherwise its files are
locked.

**Data-safe - the installer never force-kills.** It:

1. Focuses the app's main window and posts `WM_CLOSE`, so the app shows its
   own "save your work?" prompt.
2. Waits for the user to finish closing it, re-focusing + re-sending
   `WM_CLOSE` every 5 s so the prompt stays in view.
3. Proceeds the instant the process exits.

There is no timeout-then-terminate. If the user never closes the app the only
way out is the **Cancel** button (or `Ctrl+C` in silent mode), which aborts
the install with `"<app> is still running"`. Unsaved work is always the user's
to keep.

```
INFO  target app running (1 process(es)); requesting close (no force)
INFO  target app closed by user after 6s
```

Console / windowless processes have no window to message - the installer
simply waits for them to exit (or Cancel). Implementation: [installer/src/proc.rs](installer/src/proc.rs).

## Large payloads

The file zip is **not** a resource - `UpdateResource` buffers everything in RAM
and has a practical size ceiling. Instead it's appended as a PE overlay:

- **Build**: streamed onto the end of the exe after all resource passes
  (manifest/uninstaller/icon/version), before signing. No size limit.
- **Locate**: the installer finds the overlay from the PE **section table**
  (end of last section's raw data), so a trailing Authenticode certificate
  appended by `signtool` doesn't disturb it. A magic + a length stored in
  `RT_RCDATA` id=4 validate the region.
- **Read**: the installer **mmaps** the overlay (zero-copy, OS demand-paged) and
  each file is streamed from the zip to disk in ~1 MB chunks, hashed inline.

Result: multi-GB installers work with roughly constant working memory.
Measured: a 1.2 GB payload installs at ~1.1 GB peak working set - and that's
mostly the reclaimable file-cache from the BLAKE3 integrity scan, not a hard
allocation.

## Crash safety (two-phase commit)

Installs and patches are transactional. Nothing in the live install is touched
until every file is built and hash-verified.

**Phase 1 - Stage.** Each new/changed file is produced under
`.installer_tmp/staged/` (full extract, or `hdiff(existing, patch)` for
patches) and verified by BLAKE3. The existing install is untouched, so a
cancel or crash here leaves the old version fully intact.

**Phase 2 - Commit.** A `commit.journal` lists every path about to change.
Then, per file: the current version is moved to `.installer_tmp/backup/`, and
the staged file is moved into place. Each move retries for ~5 s to ride out
transient locks (AV scanner, Explorer, search indexer).

**Verify.** Every staged file is BLAKE3-checked *before* commit. After commit -
still inside the transaction, backups intact - each committed file is re-read
from its final location and re-hashed, catching any corruption introduced by
the write/rename itself (bad sector, FS glitch). A mismatch triggers rollback.

**Rollback.** If any commit or verify step fails, every already-committed file
is restored from its backup (and brand-new files removed), returning the
install to its exact pre-install state, then the error is reported.

**Power-loss recovery.** On the next launch, if a `commit.journal` is found,
the previous run was interrupted mid-commit - the installer rolls back to the
pre-install state from the backups before doing anything else.

State files (`version.json`, `installer_manifest.json`) are written
`.tmp`-then-rename so a crash can't leave corrupt JSON. Commit order ensures a
crash between "files committed" and "state written" self-heals on re-run
(everything hash-skips, state is rewritten). Implementation:
[installer/src/extract.rs](installer/src/extract.rs).

This closes the classic installer failure modes: half-written installs, no-undo
patch failures, power loss, and locked/anti-virus-held files.

**Additional hardening:**
- **Single instance per install dir.** A named mutex keyed by the (normalized)
  install path is taken for the whole install. A second installer targeting the
  same folder refuses immediately ("Another installation for this folder is
  already in progress.") instead of racing on `.installer_tmp` / journal /
  backup. Different folders are independent. The OS frees the mutex on exit or
  crash, so there's never a stale lock.
- **Cancel** is only accepted during staging (live install untouched), so it
  can never leave a partial state.
- **Disk full** mid-stage is detected (`ErrorKind::StorageFull` / Win32 112/39)
  and reported as "free up space and try again" rather than a raw IO error.
- **Path traversal**: every manifest path is validated (`safe_rel`) - only
  plain relative components, no `..`, absolute roots, or drive letters. The
  payload is Ed25519-signed already; this is defense-in-depth against a
  compromised key or builder bug.
- **Long paths**: all file operations use the `\\?\` extended-length prefix, so
  deeply nested installs (> 260 chars) work.

## Disk space pre-check

Before writing a single byte the installer queries free space on the chosen
install volume (`fs4::available_space`) and refuses to start if short.

Estimate = **total install size + 100 MB buffer**, for both full and patch.
With the two-phase commit, staging writes the *full* content of every changed
file into `.installer_tmp/staged/` and they coexist until commit; the commit
itself is rename-only (same volume) so it costs no extra space. A patch's
staged output is the reconstructed *full* file, not the small patch blob - so
patches need the same headroom as a full install (the old "patch = patch size"
estimate would under-count and is gone). The figure is conservative:
hash-skipped unchanged files are counted but never actually staged.

On failure the installer bails with a human-readable message (shown in the UI
/ printed in silent mode) and logs the figures:

```
INFO  disk space: required ~100.3 MB (full, staged worst-case), available 214.51 GB on C:\...\install_target
ERROR insufficient disk space: need 2.10 GB but only 512.0 MB free
```

## Log files

Every install and uninstall writes a timestamped UTC log so failures in the
field can be debugged without a debugger. Format: one line per event,
`YYYY-MM-DDTHH:MM:SS.mmmZ <LEVEL> <message>`. Logger flushes after every
write, so even a crashed process leaves a complete file.

| Operation | Path | Notes |
|---|---|---|
| **Install** (any mode) | `<install_dir>\install.log` | Removed by the uninstaller, so it lives exactly as long as the product. |
| **Uninstall + Finalize** | `%TEMP%\rustinst-uninstall-<uninstall-pid>.log` | Single combined file. Uninstall step's PID is the identifier; finalize step receives it as `parent_pid` and appends. Survives the `rmdir` of the install directory. |

Sample install log:
```
2026-05-30T06:40:54.599Z INFO  install start: product=testapp version=1.0 kind=Full install_dir=C:\...\install_target
2026-05-30T06:40:54.599Z INFO  payload 201745 bytes, 3 files, deleted 0
2026-05-30T06:40:54.602Z INFO  extracted: bin/app.exe (360448 bytes)
2026-05-30T06:40:54.604Z INFO  extracted: data/config.json (9 bytes)
2026-05-30T06:40:54.605Z INFO  extracted: data/readme.txt (10 bytes)
2026-05-30T06:40:54.607Z INFO  install complete in 8ms
```

Sample uninstall log:
```
2026-05-30T06:42:29.768Z INFO  uninstall start: product=testapp version=1.0 install_dir=C:\...\install_target silent=true
2026-05-30T06:42:29.770Z INFO  removed 3 payload files
2026-05-30T06:42:29.771Z INFO  removed shortcuts
2026-05-30T06:42:29.771Z INFO  removed 2 state files
2026-05-30T06:42:29.772Z INFO  unregistered HKCU\...\Uninstall\testapp
2026-05-30T06:42:29.822Z INFO  finalize start: product=testapp install_dir=... parent_pid=Some(27068)
2026-05-30T06:42:29.845Z INFO  finalize complete; self scheduled for delete-on-reboot
```

Implementation lives in [common/src/log.rs](common/src/log.rs) - global
`OnceLock<Logger>` with a `Mutex<File>`, three levels (`INFO`/`WARN`/`ERROR`),
calls before `init()` are no-ops.

## Languages

Installer + uninstaller pick the UI language in this order:

1. `--lang <code>` CLI flag (e.g. `--lang fr`)
2. `INSTALLWAY_LANG` env var
3. OS user locale via `GetUserDefaultLocaleName` (first 2 ISO-639 chars)
4. English fallback

Strings live in `common/locales/<code>.toml` and are embedded at compile time
via `include_str!`. Adding a language = drop a new TOML, add it to the
`SUPPORTED` slice in `common/src/i18n.rs`. Missing keys fall back to English
then to the key literal (never blank).

Bundled today: **en** (default), **fr**.

`Translator::detect(&args)` is called once at startup; both stages of the
uninstaller share the same lookup via a thread-local in `ui.rs`.
