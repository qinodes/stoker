# Release Process

```text
release/vX.Y.Z
    ↓
CI + coverage
    ↓
merge into main
    ↓
main CI + coverage
    ↓
create and push vX.Y.Z from main
    ↓
make publish
```

## 1. Create and work on the release branch

Start from the latest `main`, create the version branch, and make the release
changes there:

```bash
git switch main
git pull --ff-only origin main
git switch -c release/v2.3.4
make version VERSION=2.3.4
git add Cargo.toml Cargo.lock
git commit -m "release: prepare v2.3.4"
```

This keeps the version bump in its own commit. Then update the code and
documentation until the release candidate is ready.

## 2. Check and push the release branch

Run the Cargo/npm checks through the Makefile only after the release changes
reach a checkpoint:

```bash
make check
git add .
git commit -m "release: finalize v2.3.4"
git push --set-upstream origin release/v2.3.4
```

The branch push starts CI and coverage for the release candidate.

## 3. Merge into main

After the release branch checks pass, merge the pushed release branch into
`main` locally and push the merge result:

```bash
git switch main
git pull --ff-only origin main
git merge --no-ff release/v2.3.4 -m "Merge release/v2.3.4 into main"
git push origin main
```

The `git merge` command is the actual integration step. The final push updates
`origin/main` and starts CI and coverage for the integrated commit.

## 4. Create and push the release tag

After main CI and coverage pass, create the tag on the clean `main` commit:

```bash
git status --short
git tag -a v2.3.4 -m "Release v2.3.4"
git push origin v2.3.4
```

The tag push starts the Release workflow. It builds the platform binaries,
packages the installers, verifies checksums, and publishes the GitHub Release.

## 5. Publish to crates.io

After the GitHub Release succeeds:

```bash
make publish
```

This runs `cargo publish` for the version in `Cargo.toml`. A version already on
crates.io cannot be published again.

## Retry

Use the Release workflow's `workflow_dispatch` entry with the existing tag.
Do not delete or recreate a published tag.
