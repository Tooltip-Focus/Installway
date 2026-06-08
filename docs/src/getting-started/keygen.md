# Generate a signing key

Every installer is signed with an **Ed25519** private key, and the matching
public key is compiled into the installer stub. Generate the pair once per
product:

```pwsh
.\target\release\installer_builder.exe keygen --out .\keys
```

This writes two hex-encoded files:

```text
keys\priv.key   ← KEEP SECRET
keys\pub.key
```

- **`priv.key`** signs the payload at pack time. Anyone holding it can produce
  installers your stubs will accept — guard it like a code-signing key.
- **`pub.key`** is compiled into every installer stub (build-time
  `INSTALLER_PUB_KEY`). At runtime the stub verifies the payload signature
  against this baked-in key before touching the disk.

> **The two keys are bound together.** A stub built with a given `pub.key` will
> only accept payloads signed by the matching `priv.key`. If they don't match,
> the installer rejects its own payload at runtime. This matters most in
> [toolchain-free packaging](../building/toolchain.md), where the packager is
> handed a `priv.key` that *must* pair with the public key already baked into
> the prebuilt stub.

## Losing the key

There is no recovery. Lose `priv.key` and every installer signed with it must
be re-issued from a stub rebuilt with a fresh `pub.key`. Back it up offline.

## One key per product (recommended)

Use a distinct keypair per product line. A leak then only affects that one
product, and you can rotate it without re-issuing unrelated installers.

Next: [Full installer](../building/full.md).
