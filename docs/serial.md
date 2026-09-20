# Detailed Serial Mode Guide

This guide explains `serial` mode. Each Job runs once, one at a time, in queue order.
Start with the [README](../README.md). For scheduled work or tasks with dependencies, see the [scheduled mode guide](scheduled.md).

## Creating and committing a Job

Create a DRAFT Job in the directory where the command should run. The complete command after `--cmd` must be enclosed in quotes.

```bash
stoker create --user <USER> --name <NAME> --cmd "<COMMAND>"
stoker show <JOB_ID>

# Commit selected DRAFT Jobs in the order they are listed.
stoker commit <JOB_ID> [<JOB_ID>...]

# Commit all DRAFT Jobs, or all DRAFT Jobs owned by a user, in creation order.
stoker commit --all
stoker commit --user <USER>
```

`--user` is a logical owner label used for identification and filtering. It is not an operating-system account or an authentication mechanism.

Jobs run in the background and do not have an interactive terminal. Use non-interactive commands and options.

Linux and macOS run commands with `sh`. Windows uses `cmd.exe`. Shell syntax and available programs may differ between platforms.

```bash
# View and filter Jobs.
stoker jobs
# stoker jobs [filters]
stoker jobs --user alice
stoker jobs --state queued
stoker jobs --user alice --state failed

# View existing logs, or follow logs until the Job finishes.
stoker logs <JOB_ID>
stoker logs -f <JOB_ID>

# Cancel a DRAFT, QUEUED, STARTING, RUNNING, or CANCELLING Job.
stoker cancel <JOB_ID>
```

## Scheduler and queue

```bash
stoker start
stoker status
stoker stop
```

If `stoker stop` finds an active Job, it asks whether to force cancellation. Use `--yes` to skip the confirmation. `QUEUED` Jobs remain until the next scheduler start.

You must lock the queue before reordering Jobs. Unlock it explicitly when you are done. While the queue is locked, you cannot commit Jobs, but you can still create or cancel them.

```bash
stoker queue lock
stoker queue edit
stoker queue unlock
```

`queue edit` only shows `QUEUED` Jobs. In browse mode, use `↑` and `↓` to select a Job, `Enter` to enter move mode, and `q` or `Esc` to exit while keeping the queue locked.

In move mode, use `↑` and `↓` to change the position. Press `Enter` to keep the move. Press `q` or `Esc` to undo the current move.

| State | Meaning |
| --- | --- |
| `DRAFT` | Created, but not committed to the queue. |
| `QUEUED` | Committed and waiting to run. |
| `STARTING` | The scheduler is preparing the Job. |
| `RUNNING` | The Job process is running. |
| `CANCELLING` | Cancellation was requested. The process and its resources are being cleaned up. |
| `SUCCEEDED` | The Job completed successfully. |
| `FAILED` | The Job process failed, or stoker could not complete the execution. |
| `CANCELLED` | The Job was cancelled. |
| `LOST` | The scheduler restarted and lost management of a Job that was running. |

Use `stoker clean` to remove `SUCCEEDED`, `FAILED`, `CANCELLED`, and `LOST` Jobs together with their logs. You can run it while the scheduler is active.

## Docker Jobs

If Stoker should wait for a container to finish before starting the next Job, run Docker in the foreground:

```bash
docker run <IMAGE> <COMMAND>
```

Do not use `docker run -d`. The command returns immediately after the container starts, so Stoker will treat the Job as finished.
