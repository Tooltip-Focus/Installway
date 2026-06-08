# Signing (Authenticode)

Installway signs the **payload** with Ed25519 (see [the security
model](../introduction.md#security-model)), but it does **not** apply an
**Authenticode** signature to the `.exe` itself. That's a separate, standard
post-build step using your code-signing certificate — and it's what stops
SmartScreen and AV from flagging your installer as an unknown publisher.

After `pack` finishes it prints the exact command:

```text
Next step (Authenticode): signtool sign /fd SHA256 /tr http://timestamp.digicert.com setup-myapp-1.0.exe
```

Run it with your certificate:

```pwsh
signtool sign /fd SHA256 `
    /tr http://timestamp.digicert.com /td SHA256 `
    /a `
    .\dist\setup-myapp-1.0.exe
```

- `/fd SHA256` — file digest algorithm.
- `/tr <url> /td SHA256` — RFC 3161 timestamp, so the signature stays valid
  after the cert expires.
- `/a` — auto-select the best cert from your store (or use `/f cert.pfx /p
  <password>` for a file-based cert).

## Why signing comes last

The payload zip is appended as a **PE overlay** *before* signing. `signtool`
appends its certificate table after the overlay, and the installer locates the
overlay from the PE **section table** (not the end of the file) — so the
trailing certificate is harmless and the order is safe:

```text
pack  →  embed resources  →  stamp icon + version  →  append payload overlay  →  signtool
                                                                                  ▲ you, here
```

Never modify the `.exe` after signing (no further `pack`, resource edits, or
overlay appends) — any change invalidates the Authenticode signature.

## Verifying

```pwsh
signtool verify /pa /v .\dist\setup-myapp-1.0.exe
```

This is independent of Installway's own `--verify`, which checks the *embedded
payload* signature rather than the Authenticode signature.

Next: [Install modes](../running/install.md).
