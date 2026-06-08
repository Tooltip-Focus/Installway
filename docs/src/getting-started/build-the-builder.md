# Build the builder

Everything starts from one tool: `installer_builder`. Build it once from the
workspace root:

```pwsh
cargo build --release -p installer_builder
```

The binary lands at `target\release\installer_builder.exe`. It has two
subcommands:

| Command | What it does |
|---|---|
| `keygen` | Generate an Ed25519 signing keypair. See [Generate a signing key](keygen.md). |
| `pack`   | Pack a directory into a self-contained installer `.exe`. |

You do **not** build the `installer` stub yourself. By default `pack` runs
`cargo build` for the stub and the uninstaller on demand, threading your public
key through as a build-time env var. (If you'd rather package without a Rust
toolchain on the packaging machine, see
[With vs. without the Rust toolchain](../building/toolchain.md).)

## Optional: HDiffPatch deltas

Patch installers can ship small binary deltas instead of whole files. That
needs `hdiffz.exe` next to `installer_builder.exe`:

```text
target\release\installer_builder.exe
target\release\hdiffz.exe          ← drop it here
```

Without `hdiffz.exe` the builder still produces working patch installers — it
just falls back to shipping each changed file in full and prints a warning.
See [Patch installer](../building/patch.md).

## Check your toolchain

```pwsh
.\target\release\installer_builder.exe --help
.\target\release\installer_builder.exe pack --help
```

Next: [Generate a signing key](keygen.md).
