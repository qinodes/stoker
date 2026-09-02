# Release Process

This document is for release maintainers publishing a new `stoker-engine`
release.

The release flow has four steps:

```text
check -> versioning -> release -> publish
```

## Prerequisites

- Work on the `main` branch.
- Choose the next SemVer version and update it in `Cargo.toml`; verify that
  `Cargo.lock` and user-visible version references agree.
- Review and stage the release changes before running `versioning`. The
  `versioning` step does not run `git add`, but it commits staged changes.
- Configure your crates.io token once with `cargo login`.
- Use a version that has not already been published to crates.io.

## 1. Check

Run the complete Rust validation suite:

```bash
make check
cargo package --list
cargo publish --dry-run
```

This runs formatting checks, compilation checks, Clippy, all tests, and a
release build. The package commands verify the files that will be shipped and
that publishing can proceed. Do not continue if any check fails.

## 2. Create the release commit and tag

```bash
make versioning VERSION=0.3.0
```

This creates or updates:

- a release commit with message `Release v0.3.0`;
- an annotated Git tag named `v0.3.0`.

The command does not update `Cargo.toml` automatically. Replace `0.3.0` with
the version already set in `Cargo.toml`, and stage the intended changes first.

## 3. Push the release

```bash
make release
```

This pushes `main` and the annotated tags reachable from it to `origin`.
The version does not need to be provided again.

After the tag is pushed, GitHub Actions runs
`.github/workflows/release.yml`. It builds packages for Windows, Linux, macOS
Apple Silicon, and macOS Intel, then attaches them to a GitHub Release for the
tag.

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
