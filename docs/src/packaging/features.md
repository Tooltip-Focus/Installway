# Feature packs

Ship one installer that can lay down different **subsets** of files — a "feature
pack" model. Files are tagged with a feature id at build time (in the signed
manifest); at install time a **plugin** decides which features are active, and
the installer stages only those (plus the always-installed base). Unselected
files are simply **never written** — not installed-then-deleted.

There is **no built-in feature UI**: a plugin drives the selection (and can
contribute a checkbox page the installer renders), so you stay in control.

## How it fits together

1. **Build** — `[[feature]]` tables map path globs to a feature id; matching
   files get `feature = "<id>"` in the manifest (everything else is *base*).
2. **Choose** — a `ui = true` plugin may show a checkbox page (pre-checked to the
   current set) via `installway_pages`; the host hands it the catalog in
   `ctx.features_json`.
3. **Resolve** — just before staging the host queries each plugin's
   `installway_features` (with this run's page answers); each returns an
   `{ enable, disable }` delta. The active set is `(base ∪ enable) \ disable`,
   where *base* is the previously-installed set on an upgrade or the build's
   **default features** (`default = true`) on a fresh install. Only ids the build
   declares are kept.
4. **Filter** — the manifest is reduced to *base + active features* and used for
   the whole install (staging, verification, disk-space, the on-disk manifest
   the uninstaller reads).
5. **Persist** — the active set is written to `installer_info.json`, so the next
   upgrade re-installs the same features by default.

Defaults seed only the **fresh** install; thereafter the set is sticky and only
the plugin changes it. A feature with `default = false` (the default) and no
plugin enabling it is never installed.

## Declaring features (build)

Config-file only — `[[feature]]` tables in `pack.toml`:

```toml
[[feature]]
id      = "Maps"
paths   = ["data/maps"]        # a bare folder covers its whole subtree
default = true                 # installed by default on a fresh install

[[feature]]
id    = "HiResTextures"
paths = ["textures/4k/**", "extra/*.pak"]   # default = false (opt-in)
```

| Field | Meaning |
|---|---|
| `id` | Feature id (ASCII letters/digits/`-`/`_`, unique). Referenced by plugins. |
| `paths` | One or more path globs (relative to the input root). |
| `default` | `true` ⇒ enabled by default on a fresh install. Omitted ⇒ `false` (opt-in). A plugin can still override it at runtime. |

**Glob syntax** (paths use `/`): a plain name matches that file or the whole
folder under it (`data/maps` ⇒ `data/maps/**`); `*` matches within one path
segment; `**` matches across segments; `?` matches one character. A file may
belong to **at most one** feature — an overlap fails the build, and a feature
matching **no** file fails too (typo guard).

The single payload zip still carries **every** file (one signature, one BLAKE3);
the installer just extracts the active subset. So the `.exe` size covers all
features regardless of what a given run installs.

## Activating features (plugin)

A plugin exports `installway_features`; the host queries it just before staging,
passing this run's page answers, and the plugin emits a delta over the usual
descriptor callback:

```json
{ "enable": ["Maps"], "disable": ["HiResTextures"] }
```

The active set is `(base ∪ enable) \ disable`. A plugin that doesn't export it
(or emits nothing) contributes nothing — the build defaults then stand. Multiple
plugins are unioned.

To **decide at runtime**, the plugin reads `ctx.features_json` —
`{ "all": [...], "active": [...] }` (declared features + the current base) — and
any of:
- **a checkbox page** (`ui = true`): `installway_pages` emits a `multi_choice`
  listing `all`, pre-checked to `active`; `installway_features` turns the checked
  set into the delta. Shown on every interactive install; silent/compact installs
  fall back to the page defaults (the base set).
- a machine/license/env probe, etc.

The host persists the resolved set itself (`installer_info.json` + the filtered
`installer_manifest.json`), so there is no side file. The worked example is
[`sdk/examples/feature_pack`](https://github.com/Tooltip-Focus/Installway/tree/main/sdk/examples/feature_pack)
(the checkbox picker). If the selection is **static**, skip the plugin entirely
and just set `default = true`.

Declare the plugin like any other; `ui = true` enables its page:

```toml
[[plugin]]
name     = "feature-pack"
dll      = "plugins/feature_pack.dll"
phase    = "pre-install"
required = false
ui       = true
```

## Upgrades, patches & removal

- **Sticky.** The active set is remembered in `installer_info.json`, so upgrades
  keep the same features unless a plugin's delta changes them.
- **Adding a feature later** works on both full and patch payloads: a patch ships
  full bytes for a newly-activated feature's files (there's no previous version
  on disk to delta against), which the installer handles automatically.
- **Deactivating a feature** (a plugin `disable`) on a copy that *had* it
  installed removes its files: they're scheduled into the same transactional
  delete pass as a patch's removals (backed up, rollback-safe), and emptied
  folders are pruned. A feature that was never installed is simply not staged.
  Because the set is sticky, *omitting* a feature from the plugin's `enable` is
  not enough to drop it — disable it explicitly.
