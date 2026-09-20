<h1 align="center">
  <img src="assets/logo.svg" width="300" alt="stoker">
</h1>
<br>

[![Rust](https://img.shields.io/badge/built%20with-Rust-orange?logo=rust)](https://www.rust-lang.org/)
![Platforms](https://img.shields.io/badge/platform-Linux%20%7C%20Windows%20%7C%20macOS-blue)
[![CI](https://github.com/qinodes/stoker/actions/workflows/ci.yml/badge.svg)](https://github.com/qinodes/stoker/actions/workflows/ci.yml)
[![codecov](https://codecov.io/gh/qinodes/stoker/branch/main/graph/badge.svg)](https://codecov.io/gh/qinodes/stoker)
[![Crates.io](https://img.shields.io/crates/v/stoker-engine)](https://crates.io/crates/stoker-engine)
[![Downloads](https://img.shields.io/crates/d/stoker-engine)](https://crates.io/crates/stoker-engine)
[![License](https://img.shields.io/github/license/qinodes/stoker)](https://github.com/qinodes/stoker/blob/main/LICENSE)

English | [繁體中文](README.zh-TW.md) | [日本語](README.ja.md)

**stoker is a lightweight, low-resource task scheduling CLI written in Rust.** It provides two operating modes: serial mode and scheduled mode.

Serial mode is suited to **long-running, GPU-, CPU-, or memory-intensive** jobs and runs them one at a time.

Scheduled mode is suited to **recurring**, lightweight multi-step tasks with dependencies and can run multiple tasks in one Flow.

Job state and logs are stored locally, so no external database service is required.

<p align="center">
  <img src="assets/ui-demo-v2.png" alt="Stoker web UI">
</p>

## Install

The recommended installer downloads the latest release, verifies its SHA256 checksum, installs it for the current user, and updates the user `PATH`. Administrator access is not required.

**Windows PowerShell**

```powershell
irm https://github.com/qinodes/stoker/releases/latest/download/stoker-install.ps1 | iex
```

**Linux / macOS**

```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/qinodes/stoker/releases/latest/download/stoker-install.sh | sh
```

You can also install with Cargo:

```bash
cargo install stoker-engine
```

For manual installation and version-specific downloads, see [GitHub Releases](https://github.com/qinodes/stoker/releases).

## 1. serial mode

Use **serial mode** when each Job should run once, in queue order. Run `stoker create` from the directory where the command should execute.

Basic syntax:

```text
stoker create --user <USER> --name <NAME> --cmd "<COMMAND>"
stoker show <JOB_ID>
stoker commit <JOB_ID>
```

```bash
# Switch to serial mode.
stoker queue lock
stoker mode set serial
stoker queue unlock

# Start the background scheduler once for the shared machine.
stoker start

# Create a DRAFT Job. The command will run later from the current directory.
stoker create --user alice --name exp-a --cmd "python train.py --lr 0.0001"

# <JOB_ID> is the Job UUID printed by the previous command. Review it, then add it to the queue.
stoker show <JOB_ID>
stoker commit <JOB_ID>

# View the queue and this Job's output.
stoker jobs
stoker logs -f <JOB_ID>
```

After commit, Jobs run one at a time. `--user` is an owner label used for display and filtering; it is not an operating-system account or authentication mechanism.

For queue editing, cancellation, and all serial commands, read the [Traditional Chinese serial guide](docs/serial.zh-TW.md).

## 2. scheduled mode

Use **scheduled mode** for recurring work or multiple dependent tasks. Set up the tasks and schedule before committing the Flow.

Basic syntax:

```bash
# Exactly one of --once-at, --daily, and --every is required.
# --schedule-timezone is optional and only valid with --daily.
# --first-at is optional and only valid with --every.
stoker flow create <FLOW_ID> --user <USER> --name <NAME> (--once-at <RFC3339> | --daily <HH:mm> [--schedule-timezone <IANA_ZONE>] | --every <Nm|Nh> [--first-at <RFC3339>])
stoker flow task add <FLOW_ID> <TASK_ID> --name <NAME> --cmd "<COMMAND>"
stoker flow commit <FLOW_ID>
```

```bash
# Switch to scheduled mode.
stoker queue lock
stoker mode set scheduled
stoker queue unlock

stoker start

# Create a DRAFT Flow.
# frequent_a001 is the <FLOW_ID>; use frequent_a001 in later flow commands.
# --every 15m runs every 15 minutes; --first-at sets the first occurrence.
stoker flow create frequent_a001 --user alice --name frequent --every 15m --first-at 2099-01-01T10:00:00+09:00

# You can also choose a one-time or daily schedule:
# stoker flow create my_task_once --user alice --name once --once-at 2099-01-01T10:00:00+09:00
# stoker flow create my_task_nightly --user alice --name nightly --daily 23:30 --schedule-timezone Asia/Tokyo

# Switch to the directory where the tasks should run, then add them to frequent_a001.
# refresh and publish are <TASK_ID>s; --after refresh refers to the first task's ID.
stoker flow task add frequent_a001 refresh --name refresh --cmd "python refresh_cache.py"
stoker flow task add frequent_a001 publish --name publish --cmd "python publish_summary.py" --after refresh

# Validate and activate the Flow.
stoker flow commit frequent_a001
stoker flow list
```

### Declarative Flow definitions

Use JSON source mode when Flow definitions should be reviewed or versioned as one desired-state file:

```bash
stoker flow export --dir ./flow-definitions
# Edit the exported JSON file.
stoker queue lock
stoker flow source-mode sync
stoker flow sync <EXPORTED_JSON> --dry-run
stoker flow sync <EXPORTED_JSON>
# Review the result before resuming scheduling.
stoker queue unlock
```

For one-time and periodic schedules, standalone scheduled jobs, retries, Flow editing, run inspection, cancellation, and recovery, read the [Traditional Chinese scheduled guide](docs/scheduled.zh-TW.md).
