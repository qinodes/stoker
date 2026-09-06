# Release Process

This document is for release maintainers publishing a new `stoker-engine`
release.

The release flow is:

```text
check -> push vX.Y.Z -> CI -> merge to main -> tag -> release -> Release workflow -> publish
```

## Prerequisites

- Work on a version branch named with the release version, such as `v1.3.0`.
- Choose the next SemVer version and update it in `Cargo.toml`; verify that
  `Cargo.lock` and user-visible version references agree.
- Commit the release changes and push the version branch before merging it into
  `main`.
- Wait for the CI workflow to pass on the version branch before merging it into
  `main`.
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

## 2. Push the version branch and wait for CI

Commit the intended release changes on the version branch, then push it. For a
`1.3.0` release, the branch name is `v1.3.0`:

```bash
make git-formal-push
```

Wait for the CI workflow to pass on GitHub before continuing. The CI workflow
is configured to run for version branches matching `vX.Y.Z`; Markdown-only
changes remain ignored. `make git-formal-push` detects the current branch,
verifies that it exactly matches `vX.Y.Z`, and pushes that branch to `origin`.
It stops with an error if the branch is detached or has a different name.

## 3. Merge the version branch into `main`

After CI passes, merge the version branch into `main`. Confirm that the merge
contains the exact commit that passed CI before creating the release tag.

## 4. Create the release tag

```bash
make tag
```

`make tag` reads the package version from `Cargo.toml` and creates an annotated
Git tag with the corresponding `v` prefix, such as `v1.2.1`, on the current
commit. Run it on `main` after the version branch has been merged, and verify
that the current commit contains the version branch commit that passed CI. The
command does not update `Cargo.toml` or create a release commit. An explicit
`VERSION=x.y.z` override is supported when needed, but normally no version
argument is required.

## 5. Push the release tag

```bash
make release
```

This pushes only the annotated release tag to `origin`.

After the tag is pushed, GitHub Actions runs
`.github/workflows/release.yml`. It builds packages for Windows, Linux, macOS
Apple Silicon. It attaches archives, platform binaries, and
`SHA256SUMS` to a GitHub Release for the tag. It also attaches the
`stoker-install.ps1` and `stoker-install.sh` installers used by the one-line
installation commands in the README files. The release workflow embeds the
tag version in those installers, so versioned installer URLs remain pinned to
that release.

## 6. Publish to crates.io

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
git switch -c v1.2.1
make version VERSION=1.2.1
make check
git add Cargo.toml Cargo.lock
git commit -m "release: prepare v1.2.1"
make git-formal-push
# Wait for CI to pass, then merge v1.2.1 into main.
git switch main
git pull --ff-only origin main
make tag
make release
# Wait for the Release workflow to pass on GitHub.
make publish
```

## Local coverage

Install the coverage tool once:

```bash
make coverage-install
```

Run coverage locally without pushing a branch:

```bash
make coverage
```

The command runs the tests and generates an HTML report at
`target/llvm-cov/html/index.html`.
