# Release Process

This document is for release maintainers publishing a new `stoker-engine`
release.

The release flow has four steps:

```text
check -> versioning -> release -> publish
```

## Prerequisites

- Work on the `main` branch.
- Update the package version in `Cargo.toml`.
- Make sure the version change is committed or staged before running
  `versioning`. The `versioning` step does not run `git add`.
- Configure your crates.io token once with `cargo login`.
- Use a version that has not already been published to crates.io.

## 1. Check

Run the complete Rust validation suite:

```bash
make check
```

This runs formatting checks, compilation checks, Clippy, all tests, and a
release build. Do not continue if this step fails.

## 2. Create the release commit and tag

```bash
make versioning VERSION=0.3.0
```

This creates:

- an empty release commit with message `Release v0.3.0`;
- an annotated Git tag named `v0.3.0`.

The command does not update `Cargo.toml` automatically. Replace `0.3.0` with
the version already set in `Cargo.toml`.

## 3. Push the release

```bash
make release
```

This pushes `main` and the annotated tags reachable from it to `origin`.
The version does not need to be provided again.

## 4. Publish to crates.io

```bash
make publish
```

If authentication has not been configured on the machine yet, run:

```bash
cargo login
```

Publishing a version to crates.io is permanent. A version that already exists
cannot be published again with different contents.

## Complete example

```bash
make check
make versioning VERSION=0.3.0
make release
make publish
```
