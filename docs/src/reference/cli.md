# CLI reference

```text
installer_builder <COMMAND>

Commands:
  keygen   Generate an Ed25519 signing keypair
  pack     Build an installer .exe with an embedded payload
```

## `keygen`

| Option | Required | Meaning |
|---|---|---|
| `--out <DIR>` | yes | Output directory for `priv.key` + `pub.key` (hex-encoded). |

## `pack`

Every value may come from the CLI **or** a `--config` TOML file; the CLI wins.
Required fields are checked after merging. See [Config file](../building/config.md).

### Identity & content

| Option | Required | Meaning |
|---|---|---|
| `--product <NAME>` | yes | Display name (ARP DisplayName, version-info, UI, shortcut label). |
| `--product-id <ID>` | yes | Registry-safe internal id (HKCU Uninstall key, ProgIDs, data folder, upgrade detection). `^[A-Za-z][A-Za-z0-9._-]{0,49}$`; keep stable across versions. |
| `--publisher <NAME>` | yes | Vendor name (ARP field + uninstall data folder). Must not be empty. |
| `--to-version <VER>` | yes | New version; also parsed `a.b.c.d` for version-info. |
| `--input <DIR>` | yes | Source dir of the new version's files. |
| `--exe <REL>` | yes | Main executable, relative to `--input`. |
| `--out <FILE>` | yes | Output installer `.exe` path. |

### Signing & stub

| Option | Required | Meaning |
|---|---|---|
| `--priv-key <FILE>` | yes | Ed25519 private key that signs the payload. |
| `--pub-key <FILE>` | toolchain mode only | Public key compiled into the stub. Omit when using `--installer-stub`. |
| `--installer-stub <FILE>` | with `--uninstaller` | Prebuilt stub; switches to [toolchain-free mode](../building/toolchain.md). |
| `--uninstaller <FILE>` | with `--installer-stub` | Prebuilt uninstaller. |
| `--reuse-stub` | no | Skip rebuilding the stub/uninstaller if they already exist (toolchain mode). |

### Patch mode

| Option | Required | Meaning |
|---|---|---|
| `--from-version <VER>` | with `--from-dir` | Previous version string; pins the target. |
| `--from-dir <DIR>` | with `--from-version` | Previous version's files, for delta generation. |

### Packaging

| Option | Meaning |
|---|---|
| `--license <FILE>` | UTF-8 EULA shown on the License page. |
| `--assoc ".ext:Description"` | File association (repeatable). |
| `--default-install-dir <DIR>` | Proposed install path; `%VAR%` tokens expanded. |
| `--skip-license` | Hide the License page. |
| `--skip-path` | Hide the Choose-location page. |
| `--upgrade-minimal-ui` | Use the compact minimal UI for upgrades (full or patch); first install still uses the wizard. |
| `--min-installer-version <VER>` | Anti-rollback floor (default `1.0.0`). |
| `--force-reinstall` | Dev: rewrite all files, remove orphans, skip from-check. |
| `--purge-unknown-files` | Full installs: remove unknown/leftover files on upgrade/reinstall (known files still hash-skipped). Ignored for patches. |
| `--config <FILE.toml>` | Read any of the above from a TOML file. |

Free-form [registry keys](../packaging/registry.md) are config-file only
(`[[registry]]` tables), not CLI flags.

## Installer (`setup-*.exe`) runtime flags

The built installer, not the builder:

| Flag | Meaning |
|---|---|
| `--silent "<dir>"` | Headless install to `<dir>`; progress on stderr. |
| `--minimal "<dir>"` | Compact self-update UI. |
| `--launch` | Run the installed exe after a silent/minimal install. |
| `--verify` | Print payload kind / versions / size; don't install. |
| `--verify-install "<dir>"` | Re-hash an installed copy; report OK/MISSING/CORRUPT. |
| `--lang <code>` | Force UI language (e.g. `fr`); else `INSTALLWAY_LANG`, else OS locale, else English. |

See [Install modes](../running/install.md) and [Exit codes](exit-codes.md).
