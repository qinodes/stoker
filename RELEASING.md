# Release Process

This document is for release maintainers publishing a new `stoker-engine`
release.

The release flow is:

```text
check -> push release/vX.Y.Z -> CI + coverage -> preview tag -> Preview Release -> final tag -> promote artifact -> optional publish
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

## 2. Push the release branch

Commit the intended release changes on the release branch, then push it. For a
`2.0.0` release, the branch name is `release/v2.0.0`:

```bash
make git-release-push
```

The push triggers the full CI workflow and Rust coverage workflow for the exact
release branch commit. Markdown-only and YAML-only changes are filtered out.
`make git-release-push` verifies that the current branch exactly matches
`release/vX.Y.Z`, that its version matches the package version, and then pushes
that branch to `origin`.

## 3. Build a preview artifact

After CI and coverage pass, create a numbered preview tag on the exact commit
that will be released and push it:

```bash
make preview-tag PREVIEW=1
make preview PREVIEW=1
```

`PREVIEW` is the preview attempt number, and the same number must be used for
both commands and for the later final-tag commands. For example, if the first
preview (`preview/v2.0.0-1`) failed after a new commit, use `PREVIEW=2` for the
new preview, not `PREVIEW=1` again:

```bash
make preview-tag PREVIEW=2
make preview PREVIEW=2
```

The `Preview Release` workflow rebuilds and verifies the committed frontend
resources, builds the platform matrix, packages the archives and installers,
verifies `SHA256SUMS`, and stores the complete release artifact. It does not
publish a GitHub Release. The preview number is local bookkeeping: if a new
source commit is needed, use a new preview number (for example `PREVIEW=2`)
after committing and pushing that new release-branch commit.

The preview tag format is `preview/vX.Y.Z-N`, such as
`preview/v2.0.0-1`. The generated installers still contain the stable release
version `X.Y.Z`, so the artifact can be promoted unchanged.

## 4. Create the final release tag

Wait for the Preview Release workflow to pass. Then create the final tag on the
same commit. Use the number of the preview that actually passed:

```bash
make tag PREVIEW=2
```

`make tag` only creates the final annotated tag locally; it does not push it.
It reads the package version from `Cargo.toml`, requires a clean working tree,
and refuses to create the final tag unless `preview/vX.Y.Z-N` (with the same
`PREVIEW=N`) exists on the current commit. The command does not update
`Cargo.toml` or create a release commit. An explicit `VERSION=x.y.z` override
is supported when needed, but normally no version argument is required.

## 5. Push and promote the release artifact

```bash
make release PREVIEW=2
```

This pushes only the annotated release tag (for example `v2.0.0`) to `origin`.
`make release` does not create the tag; run `make tag PREVIEW=N` first. The
`PREVIEW=N` value must refer to the preview workflow that passed.

After the tag is pushed, GitHub Actions runs
`.github/workflows/release.yml`. It locates the successful preview artifact for
the final tag's exact commit, verifies its files and checksums, and attaches the
artifact contents to a GitHub Release for the tag. It does not compile again,
run tests, run coverage, or publish to crates.io. This is the build-once,
promote-artifact path, so correcting a release-workflow publishing problem does
not require rebuilding or moving the final tag.

If the final release workflow needs to be retried, use its `workflow_dispatch`
entry in GitHub Actions and enter `vX.Y.Z`. It reuses the preview artifact for
that tag's commit; do not delete and recreate the final tag.

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
git switch -c release/v2.0.0
make version VERSION=2.0.0
make check
git add Cargo.toml Cargo.lock
git commit -m "release: prepare v2.0.0"
make git-release-push
# Wait for CI and coverage to pass on release/v2.0.0.
make preview-tag PREVIEW=1
make preview PREVIEW=1
# Wait for Preview Release to pass.
make tag PREVIEW=1
make release PREVIEW=1
# Wait for the Release workflow to promote the existing artifact.
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
