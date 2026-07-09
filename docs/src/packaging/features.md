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
   where *base* depends on `feature_mode` (see below): the previously-installed
   set on an upgrade (`sticky`, the default) or this build's **default features**
   (`override`). A fresh install always starts from the build defaults. Only ids
   the build declares are kept.
4. **Filter** — the manifest is reduced to *base + active features* and used for
   the whole install (staging, verification, disk-space, the on-disk manifest
   the uninstaller reads).
5. **Persist** — the active set is written to `installer_info.json`: the next
   upgrade reads it to clean up any feature it deactivates, and — under `sticky`
   — to re-seed its base.

A feature with `default = false` (the default) and no plugin enabling it is never
installed.

### Upgrade base: `sticky` vs `override`

`feature_mode` (a top-level `pack.toml` key) decides how an **upgrade** seeds the
base — the only thing it affects; a fresh install always seeds from the build
defaults:

| `feature_mode` | Upgrade base | Effect |
|---|---|---|
| `sticky` *(default)* | the previously-installed set | Features are **sticky**: what was installed before carries over, and only a plugin's delta changes it. |
| `override` | this build's `default = true` features | The **running build wins**: an upgrade resets to the new build's defaults. A feature a prior install added is dropped unless this build defaults it on or a plugin re-enables it. |

```toml
feature_mode = "override"   # omit for the default, "sticky"
```

Either way the plugin still has the final say via its `{ enable, disable }` delta,
and the previously-installed set is always used to clean up files of a feature the
upgrade drops. Use `override` when the build should dictate the feature set;
keep `sticky` (or omit the key) when a user's prior selection should persist, and
have the plugin detect and re-apply it if you need finer control.

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
`{ "all": [...], "active": [...] }` (declared features + the base, which follows
`feature_mode`: the prior install's set under `sticky`, the build defaults under
`override`) — and any of:
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

- **Upgrade base follows `feature_mode`.** Under `sticky` (default) an upgrade
  inherits the previously-installed set, so features stay put unless a plugin
  changes them. Under `override` an upgrade re-seeds from *this build's* defaults,
  so a silent upgrade resets to them and a feature a prior install added is dropped
  unless this build defaults it on or a plugin re-enables it. Either way, to carry
  a prior selection forward under your own rules, detect it in the plugin (e.g.
  probe the machine or the install dir) and emit the matching delta.
- **Adding a feature later** works on both full and patch payloads: a patch ships
  full bytes for a newly-activated feature's files (there's no previous version
  on disk to delta against), which the installer handles automatically.
- **Deactivating a feature** on a copy that *had* it installed removes its files:
  they're scheduled into the same transactional delete pass as a patch's removals
  (backed up, rollback-safe), and emptied folders are pruned. A feature that was
  never installed is simply not staged. A feature is deactivated when a plugin
  `disable`s it, or — under `override` — when the new build no longer defaults it
  on and no plugin re-enables it, so the base no longer carries it. Under `sticky`,
  *omitting* a feature from a plugin's `enable` is not enough to drop it — the
  plugin must `disable` it explicitly.
