# Signing keys

Every installer is signed with an Ed25519 private key, and the matching public
key is compiled into the installer stub. At runtime, the stub verifies the
payload signature against its baked-in key before touching the disk. This page
covers generating, using, and protecting that keypair.

## Generate a keypair

```pwsh
.\target\release\installer_builder.exe keygen --out .\keys
```

This writes two hex-encoded files:

```text
keys\priv.key   KEEP SECRET
keys\pub.key
```

- **`priv.key`** signs the payload at pack time. Anyone holding it can produce
  installers that your stubs will accept. Guard it like a code-signing key.
- **`pub.key`** is compiled into every installer stub through the
  `INSTALLER_PUB_KEY` build-time environment variable. It is not secret.

## The two keys are bound together

A stub built with a given `pub.key` only accepts payloads signed by the
matching `priv.key`. If they do not match, the installer rejects its own
payload at runtime.

`pack` protects you from shipping such a build: after producing the `.exe`,
it runs the installer's own `--verify` as a self-check and fails the build on
a mismatch. The mismatch mostly matters in
[toolchain-free packaging](../building/toolchain.md), where the packager
receives a `priv.key` that must pair with the public key already baked into
the prebuilt stub.

## Passing keys in CI

Both keys can be passed as hex strings instead of file paths, which is
convenient for CI/CD secret stores:

```pwsh
installer_builder.exe pack --config .\pack.toml `
    --priv-key-literal $env:MYAPP_PRIV_KEY
```

- `--priv-key-literal <hex>` replaces `--priv-key <file>`. The two are
  mutually exclusive.
- `--pub-key-literal <hex>` replaces `--pub-key <file>` the same way.

The config file accepts the same values as `priv_key_literal` and
`pub_key_literal`, but a hex private key in a committed file defeats the
purpose. Prefer injecting it from your CI secret store on the command line.

## If you lose the private key

There is no recovery. Lose `priv.key` and every installer signed with it must
be re-issued from a stub rebuilt with a fresh `pub.key`. Back the key up
offline.

## One key per product

Use a distinct keypair per product line. A leak then only affects that one
product, and you can rotate the key without re-issuing unrelated installers.

## Relationship to Authenticode

The Ed25519 signature protects the payload inside the `.exe`. It does not make
Windows trust the file: SmartScreen and antivirus reputation come from an
Authenticode signature, which you apply to the final `.exe` with `signtool` as
a separate step. See [Authenticode signing](../packaging/signing.md).
