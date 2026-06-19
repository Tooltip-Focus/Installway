/* SPDX-License-Identifier: MIT
 * Installway plugin ABI.
 *
 * A plugin is a Windows DLL exporting the functions below. It is bundled into
 * the (signed) installer payload and run in an isolated child process at a
 * chosen phase. `up` runs at install, `down` runs at uninstall (write it to
 * reverse `up` when that's possible; otherwise make it a no-op).
 *
 * Return 0 for success, non-zero for failure. A required plugin's non-zero
 * `up` aborts the install before any file is committed (pre-install) or fails
 * the install (post-install). `down` failures are logged but never block an
 * uninstall.
 *
 * Optional custom wizard pages: a plugin marked `ui = true` in the build config
 * also exports `installway_pages`, which describes one or more form pages the
 * installer renders itself (the plugin never draws UI). The plugin hands its
 * descriptor back through the `emit_pages` callback; the user's answers come
 * back to `installway_up` via `ctx->inputs_json`.
 *
 * The host<->plugin channel uses no temp files: the context is delivered on the
 * child's stdin, and the descriptor is forwarded over a dedicated pipe by the
 * host (the plugin just calls `emit_pages`).
 */
#ifndef INSTALLWAY_PLUGIN_H
#define INSTALLWAY_PLUGIN_H

#include <stdint.h>
#include <wchar.h>

#define INSTALLWAY_ABI_VERSION 1u

#ifdef __cplusplus
extern "C" {
#endif

typedef struct InstallwayContext {
    uint32_t       abi_version;   /* = INSTALLWAY_ABI_VERSION */
    const wchar_t* install_dir;   /* chosen install directory */
    const wchar_t* data_dir;      /* per-user data dir (holds installer_info.json);
                                     write persistent plugin state here */
    const wchar_t* product;       /* display name */
    const wchar_t* product_id;    /* registry-safe id */
    const wchar_t* version;       /* to-version */
    const wchar_t* exe;           /* full path to the installed main exe */
    /* Append a line to the install/uninstall log.
     * level is one of L"INFO", L"WARN", L"ERROR". */
    void (*log)(const wchar_t* level, const wchar_t* message);
    /* For installway_up: a JSON object of the user's page answers, keyed
     * "<page_id>.<widget_id>" with string values. NULL when the plugin has no
     * pages or for installway_pages / installway_down. */
    const wchar_t* inputs_json;
    /* For installway_pages: call this with your page-descriptor JSON (the host
     * forwards it). Call it once before returning 0. */
    void (*emit_pages)(const wchar_t* json);
} InstallwayContext;

/* ABI version the plugin was built against. The host refuses to load a plugin
 * whose value differs from its own INSTALLWAY_ABI_VERSION. */
__declspec(dllexport) uint32_t installway_abi_version(void);

/* Forward action, at install. 0 = success. Reads ctx->inputs_json. */
__declspec(dllexport) int32_t installway_up(const InstallwayContext* ctx);

/* Reverse action, at uninstall. 0 = success. No-op if irreversible. */
__declspec(dllexport) int32_t installway_down(const InstallwayContext* ctx);

/* Optional: describe custom wizard pages. Build the page-descriptor JSON, hand
 * it to ctx->emit_pages(json), and return 0. Only called for plugins marked
 * `ui = true`. Absence is an error only for such plugins. */
__declspec(dllexport) int32_t installway_pages(const InstallwayContext* ctx);

#ifdef __cplusplus
}
#endif

#endif /* INSTALLWAY_PLUGIN_H */
