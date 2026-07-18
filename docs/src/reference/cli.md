# Builder CLI

Complete reference for `installer_builder`. For the flags of the built
installer itself, see [Installer runtime flags](installer-cli.md).

```text
installer_builder <COMMAND>

Commands:
  keygen   Generate an Ed25519 signing keypair
  pack     Build an installer .exe with an embedded payload
```

## keygen

| Option | Required | Description |
|---|---|---|
| `-o, --out <DIR>` | Yes | Output directory for `priv.key` and `pub.key` (hex-encoded). |

See [Signing keys](../getting-started/signing-keys.md).

## pack

Every value may come from the CLI or from a `--config` TOML file; the CLI
wins. Required fields are checked after merging. See
[The config file](../building/config.md) for the file format and merge rules.

### Identity and content

| Option | Required | Description |
|---|---|---|
| `-p, --product <NAME>` | Yes | Display name: Windows Apps, version info, wizard UI, shortcut labels. |
| `--product-id <ID>` | Yes | Registry-safe internal id: Uninstall key, ProgIDs, data folder, upgrade detection. Must match `^[A-Za-z][A-Za-z0-9._-]{0,49}$`; keep it stable across versions. |
| `--publisher <NAME>` | Yes | Vendor name: Apps "Publisher" field and the uninstall data folder. Must not be empty. |
| `--to-version <VER>` | Yes | New version. Also parsed as `a.b.c.d` for the version-info resource. |
| `--input <DIR>` | Yes | Source directory of the new version's files. |
| `-e, --exe <REL>` | No | Main executable, relative to `--input`. Omit only if the product has no executable; see [Full installers](../building/full.md#the-main-executable). |
| `-o, --out <FILE>` | Yes | Output installer `.exe` path. |

### Signing and stub

| Option | Required | Description |
|---|---|---|
| `--priv-key <FILE>` | One of the two | Ed25519 private key file that signs the payload. |
| `--priv-key-literal <HEX>` | One of the two | The private key as 64 hex chars, for CI pipelines. Mutually exclusive with `--priv-key`. |
| `--pub-key <FILE>` | Toolchain mode: one of the two | Public key file compiled into the stub. Ignored with `--installer-stub`. |
| `--pub-key-literal <HEX>` | Toolchain mode: one of the two | The public key as 64 hex chars. Mutually exclusive with `--pub-key`. |
| `--installer-stub <FILE>` | With `--uninstaller` | Prebuilt stub. Switches to [toolchain-free mode](../building/toolchain.md). |
| `--uninstaller <FILE>` | With `--installer-stub` | Prebuilt uninstaller. |
| `--reuse-stub` | No | Skip rebuilding the stub and uninstaller when they already exist (toolchain mode). |

### Patch mode

| Option | Required | Description |
|---|---|---|
| `--from-version <VER>` | With `--from-dir` | Previous version string. Pins the target install. |
| `--from-dir <DIR>` | With `--from-version` | Previous version's files, for delta generation. |

### Packaging and behavior

| Option | Default | Description |
|---|---|---|
| `--license <FILE>` | Built-in placeholder | UTF-8 EULA shown on the License page. |
| `--banner <FILE.png>` | Flat gray header | PNG painted across the wizard header. Author at 1400 x 144 px; see [Branding](../packaging/branding.md#header-banner). |
| `--assoc ".ext:Description"` | None | File association. Repeatable; a CLI list replaces the config file's list. |
| `--default-install-dir <DIR>` | `%LOCALAPPDATA%\Programs\<product>` | Proposed install path. `%VAR%` tokens are expanded. |
| `--skip-license` | Off | Hide the License page. |
| `--skip-path` | Off | Hide the Choose-location page. |
| `--install-dir-restriction <enforce\|default-dir-only\|bypass>` | `enforce` | Non-empty-folder guard for fresh interactive installs. See [Wizard pages and install location](../packaging/wizard.md#the-non-empty-folder-guard). |
| `--launch-option <checked\|unchecked\|hidden>` | `checked` | State of the final-page "launch now" checkbox. |
| `--upgrade-minimal-ui` | Off | Upgrades use the compact minimal UI; a first install still gets the wizard. |
| `--show-uninstall-complete` | Off | Show a confirmation message box at the end of an interactive uninstall. |
| `--min-installer-version <VER>` | `1.0.0` | Minimum installer stub version allowed to run this payload. |
| `--purge-unknown-files` | Off | Full installs: remove unknown or leftover files on an upgrade or reinstall. Known files are still hash-skipped. Ignored for patches. |
| `--force-reinstall` | Off | Dev: rewrite all files, remove orphans, skip the from-version check. |
| `--config <FILE.toml>` | None | Read any of the above from a TOML file. |

Shortcuts, registry entries, plugins, feature packs, and `feature_mode` are
config-file only. See [The config file](../building/config.md#tables).
