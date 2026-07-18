# Plugins

Run your own install and uninstall logic without touching the installer's
source. A plugin is a native Windows DLL, written in C, C++, Rust, or any
language with a C ABI, bundled into the signed installer payload and run at a
chosen phase.

Plugins are a migration pair: `up` runs at install, `down` runs at uninstall.
Write `down` to reverse `up` when that is possible; otherwise make it a
no-op. The SDK and ready-to-edit examples live in
[`sdk/`](https://github.com/Tooltip-Focus/Installway/tree/main/sdk).

## The contract

A plugin exports these C functions (see
[`sdk/installway_plugin.h`](https://github.com/Tooltip-Focus/Installway/blob/main/sdk/installway_plugin.h)):

```c
uint32_t installway_abi_version(void);                  // return INSTALLWAY_ABI_VERSION
int32_t  installway_up(const InstallwayContext*);       // at install   (0 = ok)
int32_t  installway_down(const InstallwayContext*);     // at uninstall (0 = ok)
int32_t  installway_pages(const InstallwayContext*);    // optional: custom pages
int32_t  installway_features(const InstallwayContext*); // optional: feature packs
```

The host passes a context with:

- `install_dir`, `product`, `product_id`, `version`, and the full `exe` path;
- `data_dir`, the folder holding `installer_info.json`. Write persistent
  plugin state here;
- `log(level, message)`, a callback that writes to the install or uninstall
  log;
- `lang`, the host's resolved UI language code such as `"en"` or `"fr"`,
  honoring its `--lang` and `INSTALLWAY_LANG` overrides;
- `inputs_json` and the `emit_pages(json)` callback, which serve the
  [custom pages](#custom-wizard-pages) feature. A plugin without pages
  ignores them;
- `features_json`, the [feature pack](features.md) catalog
  (`{ "all": [...], "active": [...] }`, or empty when the build declares
  none).

The host-to-plugin channel uses no temp files. The context is streamed to the
child process on stdin, and page descriptors come back over a dedicated pipe
the host owns.

**Localizing a plugin.** There is no shared string table across the ABI. A
plugin ships its own strings and selects them by `ctx->lang`, falling back to
English for codes it does not translate. See the `uninstall_msi` example,
which localizes its page title and subtitle this way.

## Declaring plugins

Config-file only, as `[[plugin]]` tables:

```toml
[[plugin]]
name  = "uninstall-old-msi"
dll   = "plugins/uninstall_old_msi.dll"   # path to the built DLL
phase = "pre-install"                     # pre-install | post-install
required = true                           # default true
ui    = false                             # default false; see custom pages
```

| Field | Description |
|---|---|
| `name` | Unique id: ASCII letters, digits, `-`, `_`. Names the in-payload DLL and the log lines. |
| `dll` | Path to the DLL to bundle. |
| `phase` | `pre-install` (before any file is staged) or `post-install` (after the install is finalized). |
| `required` | If `true` (default), a non-zero `up` fails the install. If `false`, the failure is logged and the install continues. |
| `ui` | If `true`, the plugin contributes [custom wizard pages](#custom-wizard-pages). Default `false`. |

Plugins of the same phase run in declared order. At uninstall, `down` runs in
reverse order.

## Phases and failure

| Phase | When | A required `up` failure |
|---|---|---|
| `pre-install` | Before staging and commit. | Aborts cleanly; nothing is committed. |
| `post-install` | After finalize: files in place, product registered. | Fails the install. Files stay; uninstall removes them. |
| `down` | At uninstall, before files are removed. | Always best-effort: logged, never blocks the uninstall. |

**Machine-wide installs run plugins elevated; mind per-user state.** For a
machine-wide install, the host runs in an elevated subprocess under the admin
account, and your `up` and `down` code runs there too. `ctx->data_dir`
correctly points at the machine-wide folder (`%ProgramData%\...`) in that
case, so prefer it for any state you persist. But Windows per-user APIs
(`%APPDATA%`, `%USERPROFILE%`, `HKEY_CURRENT_USER`, the user's Desktop and
Start Menu) resolve to the elevated admin's profile, not the user who
launched the installer. Do not write there expecting the end user to see it.
Use `ctx->data_dir`, `install_dir`, or explicit machine locations (`HKLM`,
All-Users folders) instead. See
[Per-user and machine-wide installs](../running/machine-wide.md).

## Custom wizard pages

A plugin marked `ui = true` can add its own pages to the installer wizard,
for example a country picker whose answer drives a region-specific install.
The plugin never draws UI. It returns a descriptor, and the installer renders
the page with its own native controls, so the plugin stays crash-isolated in
its child process.

`installway_pages` is a step function. The host calls it once per page:

1. The host calls `installway_pages` with the answers so far in
   `ctx->inputs_json` (empty on the first call). You build one step, hand it
   to `ctx->emit_pages(json)`, and return 0.
2. The installer renders that page, validates required fields, collects the
   answers, and calls `installway_pages` again with the updated answers.
3. Return `{ "step": "done" }` when there are no more pages. Then
   `installway_up` receives all the answers in `ctx->inputs_json`, a JSON
   object keyed `"<page_id>.<widget_id>"`.

Because each call sees the answers so far, a page can depend on an earlier
one: branch, compute options from a prior answer, validate and re-ask, show
a confirmation summary, or end early. The plugin stays stateless; the host
carries the state.

### The step format

```json
{ "step": "page",
  "page": { "id": "region", "title": "...", "widgets": [ ... ] },
  "notice": "",
  "back": true }
```

```json
{ "step": "done" }
```

- `page` is `{ id, title, subtitle?, widgets[] }`. The `id` namespaces the
  answers and only needs to be unique per page you show.
- `notice` is an optional banner, useful to surface a validation error when
  re-asking.
- `back` defaults to `true`; set `false` to disable the Back button on this
  page.

A dependent-page loop in pseudo-code:

```c
// read ctx->inputs_json, then emit one step
if (!has("region.country"))                          emit(country_page);
else if (country == "DOM" && !has("dom.territory"))  emit(territory_page);
else                                                 emit("{ \"step\": \"done\" }");
```

### Widget palette

| `kind` | Control | Value in `inputs_json` |
|---|---|---|
| `label` | Static text | None. |
| `text` | Text box. `password` masks input, `number` accepts digits only, `multiline` is taller. | The typed string. |
| `checkbox` | Checkbox | `"true"` or `"false"`. |
| `single_choice` | Radio group or drop-down (`style`: `radio` or `combo`) | The chosen option's `value`. |
| `multi_choice` | Checkbox group, pick any | The checked `value`s joined by `,`. |

`text`, `single_choice`, and `multi_choice` accept `required` (a
`single_choice` is required by default). `default` pre-selects a value; for
`multi_choice` it takes a list of `value`s. Titles, labels, and option text
are rendered verbatim, so localize them in the plugin.

### Silent installs

`--silent` and the compact upgrade UI have no form to fill, so the host
drives the step loop itself, answering each page from its widget defaults (a
`single_choice` with no `default` uses its first option) until `done`. A
required field with no usable default, or a gate that keeps re-asking, fails
the silent install with a message telling the user to run the interactive
installer.

### Remembering choices across upgrades

The host does not save your answers. To skip pages an earlier install already
answered, persist the choice yourself and check for it in the step function:

- In `up`, write the choice to a file under `ctx->data_dir`, next to
  `installer_info.json`, for example `data_dir\myplugin.txt`.
- In `installway_pages`, read that file. If it exists, return
  `{ "step": "done" }`. The page is skipped and `up` reuses the saved value.

```c
// installway_pages, first thing:
if (file_exists(data_dir, "myplugin.txt"))  emit("{ \"step\": \"done\" }");
else                                        emit(first_page);
```

This works the same silently: a first silent install fills the widget
defaults and saves them; later silent upgrades read the file and skip.
`data_dir` lives in `%LOCALAPPDATA%\<publisher>\Uninstall\<product_id>`
(per-user) or `%ProgramData%\<publisher>\Uninstall\<product_id>`
(machine-wide). Always read it from `ctx->data_dir` rather than hard-coding
it. The uninstaller deletes the folder, so your state is cleaned up
automatically; your `down` runs first, before the folder is removed, if it
needs to read the state.

Uninstall (`down`) gets no page answers in `inputs_json`. Persisting in
`data_dir` as above is how a plugin carries an install-time choice to
uninstall. Avoid storing secrets there in plaintext.

See
[`sdk/examples/country_picker`](https://github.com/Tooltip-Focus/Installway/tree/main/sdk/examples/country_picker)
for a complete page-contributing plugin. It remembers the country in
`data_dir` and skips the page on upgrade.

## Selecting feature packs

A plugin can also export `installway_features` to choose which
[feature packs](features.md) get installed. The host queries it just before
staging, passing the page answers in `inputs_json` and the catalog in
`features_json`, and the plugin emits `{ "enable": [...], "disable": [...] }`
over the same `emit_pages` channel. The host stages only the base plus the
active features. A `ui = true` plugin can pair this with a checkbox page.

## Example: replacing an MSI or InstallShield install

A common use is removing a previous-technology install before laying down
the new one:

```toml
[[plugin]]
name  = "uninstall-old-msi"
dll   = "plugins/uninstall_old_msi.dll"
phase = "pre-install"

[[plugin]]
name  = "uninstall-old-installshield"
dll   = "plugins/uninstall_old_is.dll"
phase = "pre-install"
```

Ready-to-edit Rust sources are in
[`sdk/examples/`](https://github.com/Tooltip-Focus/Installway/tree/main/sdk/examples):
`uninstall_msi`, `uninstall_installshield`, the `country_picker` page
example, the `feature_pack` picker, and a minimal template. Build them with
`cargo build --release`. C and C++ authors use
[`installway_plugin.h`](https://github.com/Tooltip-Focus/Installway/blob/main/sdk/installway_plugin.h);
see
[`sdk/README.md`](https://github.com/Tooltip-Focus/Installway/blob/main/sdk/README.md).

## Toolchain-free packaging

Plugins are just bundled binaries, so they work in
[toolchain-free packaging](../building/toolchain.md). Nothing is compiled on
the packaging machine: build the DLLs once, anywhere, then reference them
from `pack.toml`.

## Guardrails

- **Signed and hash-checked.** The DLL rides inside the Ed25519-signed
  payload, and its BLAKE3 hash is re-verified before it is loaded. A tampered
  DLL is refused.
- **Crash-isolated.** Each plugin runs in a child process (the installer or
  uninstaller re-launched as a hidden host), so a crashing or hanging plugin
  cannot take down or stall the install. It is killed past a timeout.
- **ABI-checked.** The host refuses a plugin whose `installway_abi_version()`
  does not match `INSTALLWAY_ABI_VERSION`.

## A note on antivirus

A bundled, signed DLL is far less alarming than spawning PowerShell, but
loading a DLL and spawning `msiexec` is still watched by EDR. Sign the final
`.exe` with [Authenticode](signing.md) to build reputation, and keep plugins
to genuine install needs.
