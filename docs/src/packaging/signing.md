# Authenticode signing

Installway signs the payload inside the `.exe` with Ed25519 (see the
[security model](../introduction.md#security-model)), but it does not apply
an Authenticode signature to the `.exe` itself. That is a separate, standard
post-build step using your code-signing certificate, and it is what stops
SmartScreen and antivirus engines from flagging your installer as coming from
an unknown publisher.

After `pack` finishes, it prints the exact command:

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

- `/fd SHA256` selects the file digest algorithm.
- `/tr <url> /td SHA256` adds an RFC 3161 timestamp, so the signature stays
  valid after the certificate expires.
- `/a` auto-selects the best certificate from your store. Use
  `/f cert.pfx /p <password>` for a file-based certificate instead.

## Why signing comes last

The payload zip is appended as a PE overlay before signing. `signtool`
appends its certificate table after the overlay, and the installer locates
the overlay from the PE section table rather than the end of the file, so
the trailing certificate is harmless and the order is safe:

```text
pack  >  embed resources  >  stamp icon + version  >  append payload overlay  >  signtool
                                                                                 you, here
```

Never modify the `.exe` after signing: no further `pack`, resource edits, or
overlay appends. Any change invalidates the Authenticode signature.

## Verifying

```pwsh
signtool verify /pa /v .\dist\setup-myapp-1.0.exe
```

This is independent of Installway's own `--verify`, which checks the embedded
payload signature rather than the Authenticode signature.
