# Release Process

This document is for release maintainers publishing a new `stoker-engine`
release.

The release flow is:

```text
check -> push release/vX.Y.Z -> CI + coverage -> final tag -> build and publish GitHub Release -> optional crates.io publish
```

## Prerequisites

- Work on a release branch named `release/vX.Y.Z`, such as
  `release/v2.0.0`.
- Choose the next SemVer version and update it in `Cargo.toml`; verify that
  `Cargo.lock` and user-visible version references agree.
- Commit the release changes and push the release branch.
- Wait for both CI and coverage to pass on the exact release branch commit that
  will receive the tag.
- Configure your crates.io token once with `cargo login`.
- Use a version that has not already been published to crates.io.

## 1. Check

Run the complete validation suite:

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

## 2. Push the release branch

Commit the intended release changes on the release branch, then push it. For a
`2.0.0` release, the branch name is `release/v2.0.0`:

```bash
make git-release-push
```

The push triggers the CI and Rust coverage workflows for the exact release
branch commit. Markdown-only and YAML-only changes are filtered out.
`make git-release-push` verifies that the current branch exactly matches
`release/vX.Y.Z`, that its version matches the package version, and then pushes
that branch to `origin`.

## 3. Create the final release tag

After CI and coverage pass, create the final annotated tag on the same commit:

```bash
make tag
```

`make tag` reads the package version from `Cargo.toml`, requires a clean working
tree, verifies the release branch, and creates `vX.Y.Z` locally. It does not
push the tag. The command does not update `Cargo.toml` or create a release
commit. An explicit `VERSION=x.y.z` override is supported when needed, but
normally no version argument is required.

## 4. Build and publish the GitHub Release

Push the final tag:

```bash
make release
```

`make release` verifies that the local tag exists, points to the current commit,
and then pushes it to `origin`. The Release workflow checks out that exact tag,
rebuilds the frontend and platform binaries, packages the archives and
installers, verifies `SHA256SUMS`, and publishes the resulting files to a
GitHub Release. It does not run coverage or publish to crates.io.

If the Release workflow needs to be retried, use its `workflow_dispatch` entry
in GitHub Actions and enter `vX.Y.Z`. It rebuilds the artifact from the tagged
commit; do not delete and recreate the final tag.

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
git switch -c release/v2.0.0
make version VERSION=2.0.0
make check
git add Cargo.toml Cargo.lock
git commit -m "release: prepare v2.0.0"
make git-release-push
# Wait for CI and coverage to pass on release/v2.0.0.
make tag
make release
# Wait for the Release workflow to publish the GitHub Release.
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
