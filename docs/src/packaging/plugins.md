# Plugins (custom DLLs)

Run your own install/uninstall logic without touching the installer's source.
A **plugin** is a native Windows DLL — written in C/C++/Rust/anything with a C
ABI — bundled into the (signed) installer payload and run at a chosen phase.

Plugins are a **migration pair**: `up` runs at install, `down` runs at
uninstall. Write `down` to reverse `up` when that's possible; otherwise make it
a no-op. The SDK + examples live in [`sdk/`](https://github.com/Tooltip-Focus/Installway/tree/main/sdk).

## The contract

A plugin exports these C functions (see
[`sdk/installway_plugin.h`](https://github.com/Tooltip-Focus/Installway/blob/main/sdk/installway_plugin.h)):

```c
uint32_t installway_abi_version(void);              // return INSTALLWAY_ABI_VERSION
int32_t  installway_up(const InstallwayContext*);   // at install   (0 = ok)
int32_t  installway_down(const InstallwayContext*); // at uninstall (0 = ok)
int32_t  installway_pages(const InstallwayContext*); // optional — see below
```

The host passes a context: `install_dir`, `data_dir` (the folder holding
`installer_info.json` — write persistent plugin state here), `product`,
`product_id`, `version`, the full `exe` path, and a `log(level, message)` callback
that writes to the install/uninstall log. Two more fields serve the
[custom-pages](#custom-wizard-pages-forms) feature: `inputs_json` (the user's
answers, for `up`) and the `emit_pages(json)` callback (`installway_pages` hands
its descriptor to it). A plugin without pages just leaves `installway_pages`
out.

> **Machine-wide installs run plugins elevated — mind per-user state.** For a
> machine-wide install (a shared location such as `Program Files`), the host runs
> in an **elevated subprocess under the admin account**, and your `up`/`down`
> plugins run there too. `ctx->data_dir` correctly points at the machine-wide
> folder (`%ProgramData%\…`) in that case, so prefer it for any state you persist.
> But Windows per-user APIs (`%APPDATA%`, `%USERPROFILE%`, `HKEY_CURRENT_USER`,
> the user's Desktop/Start Menu) resolve to the **elevated admin's** profile —
> *not* the user who launched the installer — so don't write there expecting the
> end user to see it. Use `ctx->data_dir`, `install_dir`, or explicit machine
> locations (`HKLM`, All-Users folders) instead.

The host↔plugin channel uses **no temp files**: the context is streamed to the
child on stdin, and the descriptor comes back over a dedicated pipe the host owns
(the plugin only calls `emit_pages`).

## Declaring plugins

Config-file only (`[[plugin]]` tables):

```toml
[[plugin]]
name  = "uninstall-old-msi"
dll   = "plugins/uninstall_old_msi.dll"   # path to the built DLL
phase = "pre-install"                     # pre-install | post-install
required = true                           # default true
ui    = false                             # default false — see custom pages
```

| Field | Meaning |
|---|---|
| `name` | Unique id (ASCII letters/digits/`-`/`_`). Names the in-payload DLL and the log lines. |
| `dll` | Path to the DLL to bundle. |
| `phase` | `pre-install` (before any file is staged) or `post-install` (after the install is finalized). |
| `required` | If `true` (default), a non-zero `up` **fails the install**. If `false`, it's logged and the install continues. |
| `ui` | If `true`, the plugin contributes [custom wizard pages](#custom-wizard-pages-forms). Default `false`. |

Plugins of the same phase run in declared order; `down` runs in reverse order
at uninstall.

## Phases & failure

| Phase | When | A required `up` failure |
|---|---|---|
| `pre-install` | before staging/commit | aborts cleanly — nothing is committed |
| `post-install` | after finalize (files in place, product registered) | fails the install (files stay; uninstall removes them) |
| `down` | at uninstall, before files are removed | always best-effort: logged, never blocks the uninstall |

## Custom wizard pages (forms)

A plugin marked `ui = true` can add its **own pages** to the installer wizard —
e.g. a country picker whose answer drives a region-specific install. The plugin
**never draws UI**: it returns a *descriptor* and the installer renders the page
with its own native controls, so the plugin stays crash-isolated in its child
process.

`installway_pages` is a **step function** — the host calls it once per page:

1. The host calls `installway_pages` with the answers so far in
   `ctx->inputs_json` (empty on the first call). You build **one** step, hand it
   to `ctx->emit_pages(json)`, and return 0.
2. The installer renders that page, validates required fields, collects the
   answer, and calls `installway_pages` **again** with the updated answers.
3. Return `{ "step": "done" }` when there are no more pages. Then `installway_up`
   gets all the answers in `ctx->inputs_json` (a JSON object keyed
   `"<page_id>.<widget_id>"`).

Because each call sees the answers so far, **a page can depend on an earlier
one** — branch, compute options from a prior answer, validate and re-ask, show a
confirmation summary, or end early. The plugin stays stateless; the host carries
the state.

### A step

```json
{ "step": "page",
  "page": { "id": "region", "title": "...", "widgets": [ ... ] },
  "notice": "",        // optional banner (e.g. a validation error, to re-ask)
  "back": true }       // optional, default true — set false to disable Back here
```
```json
{ "step": "done" }
```

The `page` object is `{ id, title, subtitle?, widgets[] }`; `id` namespaces the
answers and need only be unique per page you show.

#### Example — a dependent page

```c
// pseudo: read ctx->inputs_json, then emit one step
if (!has("region.country"))            emit(country_page);          // first
else if (country == "DOM" && !has("dom.territory")) emit(territory_page); // branch
else                                   emit("{ \"step\": \"done\" }");
```

### Widget palette

| `kind` | Control | Value in `inputs_json` |
|---|---|---|
| `label` | Static text | none |
| `text` | Text box (`password` masks, `number` digits-only, `multiline` taller) | the typed string |
| `checkbox` | Checkbox | `"true"` / `"false"` |
| `single_choice` | Radio group or drop-down (`style`: `radio` \| `combo`) | the chosen option's `value` |
| `multi_choice` | Checkbox group (pick any) | the checked `value`s joined by `,` |

`text`, `single_choice` and `multi_choice` accept `required` (a `single_choice`
is required by default). `default` pre-selects a value (`multi_choice` takes a
list of `value`s); titles/labels/option text are rendered verbatim, so localize
them in the plugin.

### Silent installs

`--silent` and the compact upgrade UI have no form to fill, so the host drives the
step loop itself, answering each page from its widget `default`s (a `single_choice`
with no `default` uses its first option) until `done`. A **required** field with no
usable default — or a gate that keeps re-asking — **fails** the silent install with
a message telling the user to run the interactive installer.

### Remembering choices across upgrades

The host does **not** save your answers. To skip pages an upgrade already
answered (the way the install path is remembered), persist them yourself and
check that in the step:

- In `up`, write the choice to a file under **`ctx->data_dir`** (next to
  `installer_info.json`), e.g. `data_dir\myplugin.txt`.
- In `installway_pages`, read that file. If it exists, return
  `{ "step": "done" }` — the page is skipped and `up` reuses the saved value.

```c
// installway_pages, first thing:
if (file_exists(data_dir, "myplugin.txt"))   emit("{ \"step\": \"done\" }");
else                                          emit(first_page);
```

This works the same **silently**: a first silent install fills widget `default`s
and saves them; later silent upgrades read the file and skip. `data_dir` lives in
`%LOCALAPPDATA%\<publisher>\Uninstall\<product_id>` (per-user) or
`%ProgramData%\<publisher>\Uninstall\<product_id>` (machine-wide) — always read it
from `ctx->data_dir` rather than hard-coding — and is **deleted by the
uninstaller**, so your state is cleaned up automatically (your `down` runs first,
before the folder is removed, if it needs to read it).

> Uninstall (`down`) gets no page answers in `inputs_json`. Persisting in
> `data_dir` (above) is how a plugin carries an install-time choice to uninstall.
> Avoid storing secrets there in plaintext.

See [`sdk/examples/country_picker`](https://github.com/Tooltip-Focus/Installway/tree/main/sdk/examples/country_picker)
for a complete page-contributing plugin (it remembers the country in `data_dir`
and skips on upgrade).

## Example — switch from MSI/InstallShield

A common use is removing a previous-technology install before laying down the
new one:

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

Ready-to-edit **Rust** sources are in
[`sdk/examples/`](https://github.com/Tooltip-Focus/Installway/tree/main/sdk/examples)
(`uninstall_msi`, `uninstall_installshield`, the `country_picker` page example,
and a minimal template). Build with
`cargo build --release`. A plugin can be any language with a C ABI — C/C++
authors use [`installway_plugin.h`](https://github.com/Tooltip-Focus/Installway/blob/main/sdk/installway_plugin.h);
see [`sdk/README.md`](https://github.com/Tooltip-Focus/Installway/blob/main/sdk/README.md).

## Toolchain-free

Plugins are just bundled binaries, so they work in
[toolchain-free packaging](../building/toolchain.md): nothing is compiled on the
packaging machine. Build the DLLs once (anywhere), then reference them from
`pack.toml`.

## Guardrails

- **Signed & hash-checked.** The DLL rides inside the Ed25519-signed payload,
  and its BLAKE3 is re-verified before it's loaded — a tampered DLL is refused.
- **Crash-isolated.** Each plugin runs in a **child process** (the
  installer/uninstaller re-launched as a hidden host), so a crashing or hanging
  plugin can't take down or stall the install. It's killed past a timeout.
- **ABI-checked.** The host refuses a plugin whose `installway_abi_version()`
  doesn't match `INSTALLWAY_ABI_VERSION`.

## ⚠️ A note on AV

A bundled, signed DLL is far less alarming than spawning PowerShell, but loading
a DLL and spawning `msiexec` is still watched by EDR. Sign the final `.exe`
([Authenticode](signing.md)) to build reputation. Keep plugins to genuine
install needs.
