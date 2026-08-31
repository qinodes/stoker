# stoker

[![Rust](https://img.shields.io/badge/built%20with-Rust-orange?logo=rust)](https://www.rust-lang.org/)
[![Platforms](https://img.shields.io/badge/platform-Linux%20%7C%20Windows-blue)]
[![Version](https://img.shields.io/badge/version-0.1.0-informational)](Cargo.toml)

[繁體中文](README.zh-TW.md) | [日本語](README.ja.md) | English

**stoker is a local, Git-aware job scheduler.**
Any task whose code can be pinned to a Git commit and run as a command can be scheduled; AI training was simply the original example. Each job runs in an isolated Git worktree so it does not interfere with your current working directory.

## Quick start

You need Rust, Git, and a Git repository:

```bash
# Install the CLI (requires Rust and Git)
cargo install --path .

# Start the scheduler in the background (Linux and Windows)
stoker serve

# Create a DRAFT job inside a Git repository
stoker submit --user alice --name exp-a --cmd python train.py --lr 0.0001

# Review it, then add it to the queue (<JOB_ID> comes from the previous command)
stoker show <JOB_ID>
stoker commit <JOB_ID>

# Inspect and manage jobs
stoker status
stoker ps --user alice
stoker logs <JOB_ID>
stoker logs -f <JOB_ID>
stoker cancel <JOB_ID>
stoker stop
```

Everything after `--cmd` is passed unchanged to the program being run. Put stoker's own options before `--cmd`.

`--user` is a logical owner label, not an operating-system account or an authentication mechanism. It lets people sharing one machine identify and filter their jobs.

## Scope and limits

- Single-machine queue only; no multi-machine, remote, distributed-training, GPU-allocation, or container scheduling.
- Jobs require a Git repository with a clean working tree. Without Git or a commit, a job cannot be submitted.
- stoker does not manage Python/Conda/CUDA environments, datasets, checkpoints, artifacts, or experiment metrics.
- No stoker accounts, login, or authorization; `--user` is only for identification and filtering.

See [`docs/requirements.md`](docs/requirements.md) for the detailed design.
