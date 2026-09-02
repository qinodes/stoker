# stoker

[![Rust](https://img.shields.io/badge/built%20with-Rust-orange?logo=rust)](https://www.rust-lang.org/)
[![Platforms](https://img.shields.io/badge/platform-Linux%20%7C%20Windows%20%7C%20macOS-blue)]
[![Version](https://img.shields.io/badge/version-0.4.1-informational)](Cargo.toml)

[繁體中文](README.zh-TW.md) | [日本語](README.ja.md) | English

**stoker is a local command job scheduler.**
Each job runs in the directory where it was created; `.stoker/runs` stores logs only. Environments and artifacts remain under the user's control.

## Installation

Download the archive for your platform from [GitHub Releases](https://github.com/qinodes/stoker/releases), extract the `stoker` executable, and put it on your `PATH`:

- Windows: `stoker-windows-x86_64.zip`
- Linux: `stoker-linux-x86_64.tar.gz`
- macOS Apple Silicon: `stoker-macos-arm64.tar.gz`
- macOS Intel: `stoker-macos-x86_64.tar.gz`

Each release also includes a platform binary and `SHA256SUMS`. Cargo installation remains available for Rust developers.

## Quick start

You need Rust and a directory in which to run the command:

```bash
# Or install from crates.io if Cargo is available
cargo install stoker-engine

# Start the scheduler in the background (Linux and Windows)
stoker serve

# Create a DRAFT job in the current directory
stoker submit --user alice --name exp-a --cmd python train.py --lr 0.0001

# Review it, then add it to the queue (<JOB_ID> comes from the previous command)
stoker show <JOB_ID>
stoker commit <JOB_ID>

# Inspect and manage jobs
stoker status
stoker jobs
stoker jobs --user alice
stoker jobs --state draft
stoker jobs --state queued
stoker logs <JOB_ID>
stoker logs -f <JOB_ID>
stoker cancel <JOB_ID>
stoker stop
stoker --version
stoker update
stoker uninstall
```

Everything after `--cmd` is passed unchanged to the program being run. Put stoker's own options before `--cmd`.

`--user` is a logical owner label, not an operating-system account or an authentication mechanism. It lets people sharing one machine identify and filter their jobs.

`jobs` shows the queue summary: `queue_order`, `job_id`, owner, name, state, and time. Queued jobs are ordered by `queue_order`; other states show `-`. Filter with `--state draft` or `--state queued`.

Use `stoker show <JOB_ID>` for the Job's full details, command, and execution paths.

## Job states and cancellation

| State | Meaning |
| --- | --- |
| `DRAFT` | Submitted, but not committed to the queue yet. |
| `QUEUED` | Committed and waiting to run. |
| `STARTING` | Claimed by the scheduler; its source directory and process are being prepared. |
| `RUNNING` | The job process is running. |
| `CANCELLING` | A cancellation has been requested; stoker is stopping the process and cleaning up. |
| `SUCCEEDED` | The job completed successfully. |
| `FAILED` | The job process failed or stoker could not complete it. |
| `CANCELLED` | The job was cancelled. |
| `LOST` | The scheduler restarted after losing management of an in-progress job. |

Use `stoker cancel <JOB_ID>` to cancel a `DRAFT` or `QUEUED` job before it runs, or to stop a `STARTING` or `RUNNING` job. An active cancellation first becomes `CANCELLING`; stoker terminates the job's process tree, waits for cleanup, and then records `CANCELLED`. The scheduler must be running for `cancel` to work.

`stoker stop` stops the scheduler and cancels its active job. Jobs still in `QUEUED` remain queued for the next scheduler run.

Jobs use the source directory contents available when they start; changes made by a command remain there. stoker never automatically modifies or restores files in the directory.

`stoker update` checks GitHub Releases, verifies the downloaded binary with `SHA256SUMS`, and only updates after an explicit `y` or `yes` confirmation. Stop the scheduler before updating.

To install a specific version without Cargo, download the matching platform asset from the version's GitHub Release. Rust developers can use `cargo install stoker-engine --version <VERSION> --force`.

`stoker uninstall` requires an explicit confirmation. Stop the scheduler first; your Job data and logs are kept in the Stoker data folder (normally `~/.stoker`).

## Scope and limits

- Single-machine queue only; no multi-machine, remote, distributed-training, GPU-allocation, or container scheduling.
- The submission directory must exist and be a directory. Files in the directory are not inspected.
- stoker does not manage Python/Conda/CUDA environments, datasets, checkpoints, artifacts, or experiment metrics.
- No stoker accounts, login, or authorization; `--user` is only for identification and filtering.
