# Release Process

This document is for release maintainers publishing a new `stoker-engine`
release.

The release flow is:

```text
check -> push main -> CI -> tag -> release -> Release workflow -> publish
```

## Prerequisites

- Work on the `main` branch.
- Choose the next SemVer version and update it in `Cargo.toml`; verify that
  `Cargo.lock` and user-visible version references agree.
- Commit the release changes and push `main` before creating the release tag.
- Wait for the CI workflow to pass before creating the release tag.
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
that publishing can proceed. Tests use Cargo's default parallel execution so
that concurrency and resource-competition issues can be detected. If you need
to diagnose a test that is sensitive to shared resources, rerun it with
`cargo test -- --test-threads=1`. Do not continue if any check fails.

## 2. Push `main` and wait for CI

Commit the intended release changes, then push `main`:

```bash
git push origin main
```

Wait for the CI workflow to pass on GitHub before continuing. Do not push the
release tag together with `main`.

## 3. Create the release tag

```bash
make tag VERSION=1.2.1
```

This creates an annotated Git tag named `v1.2.1` on the current commit. The
current commit must be the one that passed CI. The command does not update
`Cargo.toml` or create a release commit. Replace `1.2.1` with the version
already set in `Cargo.toml`.

## 4. Push the release tag

```bash
make release VERSION=1.2.1
```

This pushes only the annotated release tag to `origin`.

After the tag is pushed, GitHub Actions runs
`.github/workflows/release.yml`. It builds packages for Windows, Linux, macOS
Apple Silicon. It attaches archives, platform binaries, and
`SHA256SUMS` to a GitHub Release for the tag.

## 5. Publish to crates.io

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
git push origin main
# Wait for CI to pass on GitHub.
make tag VERSION=1.2.1
make release VERSION=1.2.1
# Wait for the Release workflow to pass on GitHub.
make publish
```
