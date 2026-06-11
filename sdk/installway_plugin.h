/* SPDX-License-Identifier: MIT
 * Installway plugin ABI (v1).
 *
 * A plugin is a Windows DLL exporting the three functions below. It is bundled
 * into the (signed) installer payload and run in an isolated child process at a
 * chosen phase. `up` runs at install, `down` runs at uninstall (write it to
 * reverse `up` when that's possible; otherwise make it a no-op).
 *
 * Return 0 for success, non-zero for failure. A required plugin's non-zero
 * `up` aborts the install before any file is committed (pre-install) or fails
 * the install (post-install). `down` failures are logged but never block an
 * uninstall.
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
    const wchar_t* product;       /* display name */
    const wchar_t* product_id;    /* registry-safe id */
    const wchar_t* version;       /* to-version */
    const wchar_t* exe;           /* full path to the installed main exe */
    /* Append a line to the install/uninstall log.
     * level is one of L"INFO", L"WARN", L"ERROR". */
    void (*log)(const wchar_t* level, const wchar_t* message);
} InstallwayContext;

/* ABI version the plugin was built against. The host refuses to load a plugin
 * whose value differs from its own INSTALLWAY_ABI_VERSION. */
__declspec(dllexport) uint32_t installway_abi_version(void);

/* Forward action, at install. 0 = success. */
__declspec(dllexport) int32_t installway_up(const InstallwayContext* ctx);

/* Reverse action, at uninstall. 0 = success. No-op if irreversible. */
__declspec(dllexport) int32_t installway_down(const InstallwayContext* ctx);

#ifdef __cplusplus
}
#endif

#endif /* INSTALLWAY_PLUGIN_H */
