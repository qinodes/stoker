# stoker

[![Rust](https://img.shields.io/badge/built%20with-Rust-orange?logo=rust)](https://www.rust-lang.org/)
![Platforms](https://img.shields.io/badge/platform-Linux%20%7C%20Windows%20%7C%20macOS-blue)
![Version](https://img.shields.io/badge/version-0.5.4-informational)

[繁體中文](README.zh-TW.md) | [日本語](README.ja.md) | English

**stoker is a lightweight, cross-platform local command-line (CLI) job scheduler.**
Each job runs in the directory where it was submitted.

## Installation

### Without Cargo

Download the archive for your platform from [GitHub Releases](https://github.com/qinodes/stoker/releases), extract the `stoker` executable, and add its directory to `PATH`:

- Windows: `stoker-windows-x86_64.zip`
- Linux: `stoker-linux-x86_64.tar.gz`
- macOS Apple Silicon: `stoker-macos-arm64.tar.gz`

Downloaded executables are not added to the environment automatically. Add the directory containing the executable to `PATH` permanently, then reopen the terminal.

**Windows:** Edit `Path` under User variables in Environment Variables and add the executable's directory.

**macOS/Linux:** Add the following to `~/.zshrc` (macOS) or `~/.bashrc` (Linux), replacing `/path/to/stoker` with the actual directory containing the executable:

```bash
export PATH="/path/to/stoker:$PATH"
```

To apply the setting immediately in the current terminal, run:

```bash
source ~/.bashrc  # Linux
source ~/.zshrc   # macOS
```

Each release also includes a platform binary and `SHA256SUMS`.

### With Cargo already installed

```bash
cargo install stoker-engine
```

## Quick start

```bash

# Start the scheduler in the background (Linux, macOS, and Windows)
stoker start

# Create a DRAFT job in the target directory
stoker add --user <USER_NAME> --name <JOB_NAME> --cmd <COMMAND>
# Example:
# stoker add --user alice --name exp-a --cmd "python train.py --lr 0.0001"

# Review it, then add it to the queue (<JOB_ID> comes from the previous command)
# View the job details
stoker show <JOB_ID>
# Add the job (DRAFT -> QUEUED)
stoker commit <JOB_ID>
# Add every DRAFT job to the queue in creation order
stoker commit --all

# Temporarily hold all queued jobs, then restore them later
stoker pause
stoker resume

# Inspect and manage jobs

# Check the scheduler status
stoker status

# List all jobs
stoker jobs

# Filter jobs
stoker jobs --user alice
stoker jobs --state draft
stoker jobs --state queued
# Combine filters
stoker jobs --user alice --state failed

# Remove SUCCEEDED, FAILED, CANCELLED, and LOST jobs and their logs
# This can also be run while the scheduler is running.
stoker clean

# View the existing logs and exit when finished
stoker logs <JOB_ID>

# Follow new log output until the job ends or you press Ctrl+C
stoker logs -f <JOB_ID>

# Cancel a job (DRAFT, QUEUED, PAUSED, STARTING, RUNNING, or CANCELLING)
stoker cancel <JOB_ID>

# Stop the scheduler
# If a job is active, stoker asks before force-cancelling it.
# QUEUED and PAUSED jobs remain for the next scheduler run.
stoker stop

# Show the current version
stoker --version

# Update to the latest version
# Stop the scheduler before updating.
stoker update

# Uninstall
# Stop the scheduler before uninstalling.
# Job data and logs are kept in the Stoker data folder
# (macOS/Linux: ~/.stoker; Windows: %USERPROFILE%\.stoker).
stoker uninstall
```

The command after `--cmd` must be enclosed in quotes as one complete command string.

Jobs run in the background without an interactive terminal. Use non-interactive commands and flags.

`--user` is a logical owner label, not an operating-system account or an authentication mechanism.

## Job states and cancellation

| State | Meaning |
| --- | --- |
| `DRAFT` | Submitted, but not committed to the queue yet. |
| `QUEUED` | Committed and waiting to run. |
| `PAUSED` | Temporarily held; use `stoker resume` to return it to the queue. |
| `STARTING` | Claimed by the scheduler; its source directory and process are being prepared. |
| `RUNNING` | The job process is running. |
| `CANCELLING` | A cancellation has been requested; stoker is stopping the process and cleaning up. |
| `SUCCEEDED` | The job completed successfully. |
| `FAILED` | The job process failed or stoker could not complete it. |
| `CANCELLED` | The job was cancelled. |
| `LOST` | The scheduler restarted after losing management of an in-progress job. |

## Additional notes

Changes made by a command to files in the source directory are retained. stoker does not automatically modify or restore files in that directory.

Install a specific version:

`cargo install stoker-engine --version <VERSION> --force`.

Logs are stored in `.stoker/runs/<JOB_ID>/stdout.log` and `.stoker/runs/<JOB_ID>/stderr.log`.

## Scope and limits

- Single-machine queue only; no multi-machine, remote, distributed-training, GPU-allocation, or container scheduling.
- The submission directory must exist and be a directory; files in the directory are not inspected.
- stoker does not manage Python/Conda/CUDA environments, datasets, checkpoints, artifacts, or experiment metrics.
- No stoker accounts, login, or authorization; `--user` is only for identification and filtering.
