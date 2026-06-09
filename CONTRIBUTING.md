# Contributing

## Formatting

The codebase is `rustfmt`-clean and CI enforces it
(`cargo fmt --all --check`). Format before committing:

```sh
cargo fmt --all
```

### Pre-commit hook

A hook in [`.githooks/pre-commit`](.githooks/pre-commit) runs
`rustfmt --check` on the **staged** Rust files and blocks the commit if any
need formatting. Enable it once per clone:

```sh
git config core.hooksPath .githooks
```

(On Unix, also `chmod +x .githooks/pre-commit` after checkout.)

The hook is the fast local mirror of the global CI check.

## Build & test

```sh
cargo build --workspace
cargo test --workspace
```

Windows-only project (the `windows` crate + Win32 APIs); build and test on
Windows. CI runs on `windows-latest`.

## Releases

Push a tag like `v1.2.0` to cut a GitHub release (source archives are attached
automatically). See [`.github/workflows/release.yml`](.github/workflows/release.yml).
