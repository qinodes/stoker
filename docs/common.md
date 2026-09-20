# Shared Settings and Maintenance Guide

This guide applies to both `serial` mode and `scheduled` mode. It covers timezones, configuration snapshots, logs, execution policies, the Web UI, the database, updates, and uninstallation.

For Job or Flow usage, see the [serial mode guide](serial.md) and the [scheduled mode guide](scheduled.md).

## Web UI

```bash
stoker ui start --open
stoker ui status
stoker ui stop
```

By default, the Web UI only listens on `127.0.0.1:8765`. To allow access from the local network, explicitly set a non-loopback address and use it only on a trusted network:

```bash
stoker ui start --host 0.0.0.0 --port 8765
```

## Timezones and configuration snapshots

Times are always stored as UTC in SQLite. `stoker jobs` and `stoker show` convert them only when displaying them.

During initialization, Stoker writes the detected IANA timezone to `~/.stoker/config.json`.

```bash
stoker config show
stoker config set timezone Asia/Taipei
stoker config get timezone
stoker config unset timezone
stoker config set timezone       # Open the interactive selector.
stoker config snapshot
stoker config restore
```

Configuration values are resolved in this order:

1. CLI option
2. `config.json`
3. The operating system timezone

Use `--timezone` or `--tz` to override the timezone for one display only:

```bash
stoker jobs --tz Asia/Tokyo
stoker show <JOB_ID> --timezone UTC
```

When configuration is created or updated, a snapshot is saved in `~/.stoker/snapshot/`. `config snapshot` creates a new snapshot even when the configuration has not changed.

## Logs and execution policies

By default, one Job log is limited to 64 MB. The combined logs of finished Jobs are limited to 1024 MB, and logs for the latest 100 terminal Jobs are retained.

When less than 512 MB of disk space is available, the scheduler does not start another Job.

Before changing a policy, lock the queue. There must be no `STARTING`, `RUNNING`, or `CANCELLING` Job:

```bash
stoker queue lock
stoker policy set log-max-bytes-per-job 256
stoker policy set log-retention-jobs 30
stoker policy set termination-grace-ms 30000
stoker policy set max-runtime-ms 43200000
stoker policy unset max-runtime-ms
stoker policy show
stoker queue unlock
```

Enter log capacity as an integer number of megabytes without a unit, such as `256`.

Use `stoker policy get <KEY>` to view one effective value. If a log reaches its limit or writing fails, Stoker still reads output from the child process, but older log segments may be discarded.

## Database, updates, and uninstallation

```bash
stoker db check
stoker db check --integrity
stoker db backup
stoker db backup <BACKUP_PATH>

# Stop the scheduler before restoring.
stoker db restore <BACKUP_PATH> --yes

stoker --version
stoker update
stoker uninstall
```

Without a path, `db backup` writes a timestamped backup to `<STOKER_HOME>/backups/`, usually `~/.stoker/backups/`. The backup includes the SQLite WAL.

Restoring replaces the current database. If the scheduler is interrupted, running Jobs are marked `LOST` and the queue is locked. Resolve the Jobs manually, then run `stoker queue unlock`.

Stop the scheduler before updating or uninstalling. Add `--yes` to either command to skip confirmation.

Uninstallation does not delete Job data or logs. To install a specific version with Cargo:

```bash
cargo install stoker-engine --version <VERSION> --force
```
