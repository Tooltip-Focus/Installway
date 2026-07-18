# Feature packs

Ship one installer that can lay down different subsets of files. Files are
tagged with a feature id at build time, in the signed manifest. At install
time, a [plugin](plugins.md) decides which features are active, and the
installer stages only those plus the always-installed base. Unselected files
are never written; they are not installed and then deleted.

There is no built-in feature UI. A plugin drives the selection, and can
contribute a checkbox page that the installer renders, so you stay in
control.

## How it fits together

1. **Build.** `[[feature]]` tables map path globs to a feature id. Matching
   files get `feature = "<id>"` in the manifest; everything else is base.
2. **Choose.** A `ui = true` plugin may show a checkbox page, pre-checked to
   the current set, via `installway_pages`. The host hands it the catalog in
   `ctx.features_json`.
3. **Resolve.** Just before staging, the host queries each plugin's
   `installway_features` with this run's page answers. Each plugin returns an
   `{ enable, disable }` delta. The active set is
   `(base + enable) - disable`, where the base depends on `feature_mode`
   (see below). Only ids the build declares are kept.
4. **Filter.** The manifest is reduced to base plus active features and used
   for the whole install: staging, verification, disk-space check, and the
   on-disk manifest the uninstaller reads.
5. **Persist.** The active set is written to `installer_info.json`. The next
   upgrade reads it to clean up any feature it deactivates and, under
   `sticky`, to seed its base.

A feature with `default = false` (the default) that no plugin enables is
never installed.

## Declaring features

Config-file only, as `[[feature]]` tables:

```toml
[[feature]]
id      = "Maps"
paths   = ["data/maps"]        # a bare folder covers its whole subtree
default = true                 # installed by default on a fresh install

[[feature]]
id    = "HiResTextures"
paths = ["textures/4k/**", "extra/*.pak"]   # default = false (opt-in)
```

| Field | Description |
|---|---|
| `id` | Feature id: ASCII letters, digits, `-`, `_`; unique. Referenced by plugins and shortcut gates. |
| `paths` | One or more path globs, relative to the input root. |
| `default` | `true` means enabled by default on a fresh install. Omitted means `false` (opt-in). A plugin can still override it at runtime. |

**Glob syntax** (paths use `/`): a plain name matches that file or the whole
folder under it (`data/maps` behaves like `data/maps/**`); `*` matches within
one path segment; `**` matches across segments; `?` matches one character. A
file may belong to at most one feature: an overlap fails the build, and a
feature matching no file fails too, as a typo guard.

The single payload zip still carries every file, under one signature and one
BLAKE3 hash. The installer just extracts the active subset, so the `.exe`
size covers all features regardless of what a given run installs.

## Activating features from a plugin

A plugin exports `installway_features`. The host queries it just before
staging, passing this run's page answers, and the plugin emits a delta over
the usual descriptor callback:

```json
{ "enable": ["Maps"], "disable": ["HiResTextures"] }
```

The active set is `(base + enable) - disable`. A plugin that does not export
the function, or emits nothing, contributes nothing, and the base then
stands. Multiple plugins are unioned.

To decide at runtime, the plugin reads `ctx.features_json`, which carries
`{ "all": [...], "active": [...] }` (the declared features and the current
base), and any signal you like:

- **A checkbox page** (`ui = true`): `installway_pages` emits a
  `multi_choice` listing `all`, pre-checked to `active`, and
  `installway_features` turns the checked set into the delta. The page shows
  on every interactive install; silent and compact installs fall back to the
  page defaults, which is the base set.
- A machine probe, a license check, an environment variable, and so on.

The host persists the resolved set itself, in `installer_info.json` and the
filtered `installer_manifest.json`, so there is no side file. The worked
example is
[`sdk/examples/feature_pack`](https://github.com/Tooltip-Focus/Installway/tree/main/sdk/examples/feature_pack),
a checkbox picker. If the selection is static, skip the plugin entirely and
just set `default = true`.

Declare the plugin like any other; `ui = true` enables its page:

```toml
[[plugin]]
name     = "feature-pack"
dll      = "plugins/feature_pack.dll"
phase    = "pre-install"
required = false
ui       = true
```

## Upgrades: sticky vs override

The top-level `feature_mode` config key decides how an upgrade seeds the
base set. That is the only thing it affects; a fresh install always seeds
from the build defaults.

| `feature_mode` | Upgrade base | Effect |
|---|---|---|
| `sticky` (default) | The previously installed set. | Features carry over from install to install. Only a plugin's delta changes them. |
| `override` | This build's `default = true` features. | The running build wins. An upgrade resets to the new build's defaults, and a feature a prior install added is dropped unless this build defaults it on or a plugin re-enables it. |

```toml
feature_mode = "override"   # omit for the default, "sticky"
```

Either way, the plugin has the final say through its `{ enable, disable }`
delta, and the previously installed set is always used to clean up the files
of a feature the upgrade drops. Use `override` when the build should dictate
the feature set. Keep `sticky` when a user's prior selection should persist.

Under `sticky`, omitting a feature from a plugin's `enable` list is not
enough to drop it. The plugin must `disable` it explicitly.

## Adding and removing features across versions

- **Adding a feature later** works on both full and patch payloads. A patch
  ships full bytes for a newly activated feature's files, since there is no
  previous version on disk to delta against; the installer handles this
  automatically.
- **Deactivating a feature** on a copy that had it installed removes its
  files. They are scheduled into the same transactional delete pass as a
  patch's removals (backed up, rollback-safe), and emptied folders are
  pruned. A feature that was never installed is simply not staged.
