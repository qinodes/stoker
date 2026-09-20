# Detailed Guide to Flows and Scheduled Mode

This guide explains how to use two kinds of work in `scheduled` mode:

- **Flow**: a group of tasks that run as one workflow.
- **standalone scheduled Job**: a scheduled Job that runs one command without using a Flow.

If you are new to stoker, start with the [README](../README.md).
For one-time Jobs that run one at a time in order, see the [serial mode guide](serial.md).

If you only want to create your first Flow, read “Switch to scheduled mode,” “Create and run your first Flow,” and “Inspect, run, and view logs” in that order. The later sections cover editing, JSON source mode, and recovery.

## Key terms

- **Flow**: a group of tasks that run in order or according to dependencies.
- **task**: one command inside a Flow.
- **run**: one actual execution of a Flow.
- **occurrence**: one scheduled execution created by a schedule.
- **draft**: a change that has not been applied yet.
- **standalone scheduled Job**: a scheduled Job that runs one command without a Flow.

## 1. Switch to scheduled mode

Flows are available only in `scheduled` mode. Lock the queue before switching modes.

```bash
stoker mode show
stoker queue lock
stoker mode set scheduled
stoker queue unlock
```

These commands do the following:

1. `stoker mode show`: shows the current mode.
2. `stoker queue lock`: temporarily locks the queue so new work cannot enter while the mode changes.
3. `stoker mode set scheduled`: switches to `scheduled` mode.
4. `stoker queue unlock`: unlocks the queue after the switch is complete.

You cannot switch modes while an execution is starting, running, being cancelled, being cleaned up, or recovering. Wait for those executions to finish first.

## 2. Create and run your first Flow

Create a Flow in three steps:

1. Create a draft Flow with a schedule.
2. Add tasks to the Flow.
3. Commit the Flow so it can run according to its schedule.

### 2.1 Create a Flow

`<FLOW_ID>` is the Flow identifier. `--name` is the display name shown to people. The two values can be different.

Choose one of these three schedule types when you create the Flow:

```bash
# Run once.
stoker flow create <FLOW_ID> --user <USER> --name <FLOW_NAME> --once-at <RFC3339>

# Run every day.
stoker flow create <FLOW_ID> --user <USER> --name <FLOW_NAME> --daily <HH:mm>
stoker flow create <FLOW_ID> --user <USER> --name <FLOW_NAME> --daily <HH:mm> --schedule-timezone <IANA_ZONE>

# Run at a regular interval.
stoker flow create <FLOW_ID> --user <USER> --name <FLOW_NAME> --every <Nm|Nh>
stoker flow create <FLOW_ID> --user <USER> --name <FLOW_NAME> --every <Nm|Nh> --first-at <RFC3339>
```

### 2.2 Add tasks to a Flow

Change to the directory where the task should run before using `flow task add`. The current directory becomes the task's working directory.

`<TASK_ID>` is the task identifier. `--name` is the display name shown to people. The two values can be different.

```bash
# No dependency.
stoker flow task add <FLOW_ID> <TASK_ID> --name <TASK_NAME> --cmd "<COMMAND>"

# Require one upstream task to succeed.
stoker flow task add <FLOW_ID> <TASK_ID> --name <TASK_NAME> --cmd "<COMMAND>" --after <TASK_ID>

# Require two upstream tasks to succeed. Retry once if the task fails.
stoker flow task add <FLOW_ID> <TASK_ID> --name <TASK_NAME> --cmd "<COMMAND>" --after <UPSTREAM_TASK_ID_1> --after <UPSTREAM_TASK_ID_2> --match all --retries 1

# Require an upstream task to fail.
stoker flow task add <FLOW_ID> <TASK_ID> --name <TASK_NAME> --cmd "<COMMAND>" --after-failure <TASK_ID>
```

When you add dependencies, remember:

- `--after <TASK_ID>`: the upstream task must succeed.
- `--after-failure <TASK_ID>`: the upstream task must fail.
- Repeat `--after` or `--after-failure` when you need multiple conditions.
- `--match all`: run only when every condition matches. This is the default.
- `--match any`: run when at least one condition matches.
- `--retries <N>`: set how many times to retry a failed task. `0` means no retries.

### 2.3 Commit the Flow

After adding the tasks, validate and enable the Flow:

```bash
stoker flow commit <FLOW_ID>
```

### 2.4 Complete example

This example creates a Flow that runs every day at 23:30. `nightly` is both the `<FLOW_ID>` and the display name.

```bash
stoker flow create nightly --user alice --name nightly --daily 23:30 --schedule-timezone Asia/Tokyo

# Run these commands from the directory where the tasks should run.
stoker flow task add nightly refresh --name refresh --cmd "python refresh_cache.py"
stoker flow task add nightly publish --name publish --cmd "python publish_summary.py" --after refresh

# Wait for both upstream tasks to succeed. Retry once if this task fails.
stoker flow task add nightly notify --name notify --cmd "python send_notification.py" --after refresh --after publish --match all --retries 1

stoker flow commit nightly
```

To use another schedule, replace the schedule options in the first command:

```bash
stoker flow create once --user alice --name once --once-at 2099-01-01T10:00:00+09:00
stoker flow create frequent --user alice --name frequent --every 15m --first-at 2099-01-01T10:00:00+09:00
```

### 2.5 Schedule rules

#### One-time schedules

`--once-at` must use an RFC 3339 timestamp with seconds and an explicit UTC offset, for example:

```text
2099-01-01T23:30:00+09:00
```

#### Daily schedules

`--daily` uses the `HH:mm` format. Add `--schedule-timezone <IANA_ZONE>` when you need to specify a timezone.

- A missed daily occurrence is not run later.
- A time that does not exist because of daylight saving time (DST) is skipped.
- When a time occurs twice because of DST, the earlier instant is used.

#### Periodic schedules

`--every` accepts only a lowercase integer followed by minutes or hours, such as `1m`, `15m`, `1h`, or `2h`.

- The minimum interval is `1m` for minutes and `1h` for hours.
- Without `--first-at`, the first run occurs after one complete interval following the commit or schedule update.
- With `--first-at`, the time must be a future RFC 3339 timestamp.
- Later runs advance by elapsed UTC time and do not shift with DST.
- Periods missed while the scheduler is stopped are not run later.

## 3. Inspect, run, and view logs

### 3.1 Inspect a Flow

```bash
stoker flow list
stoker flow list --user <USER>
stoker flow show <FLOW_ID>
stoker flow history <FLOW_ID>
stoker flow occurrences <FLOW_ID>
```

- `flow list`: lists Flows.
- `flow show`: shows the Flow configuration and tasks.
- `flow history`: shows the Flow's execution history.
- `flow occurrences`: shows occurrences created by the schedule.

Each task shown by `stoker flow show <FLOW_ID>` includes `cwd`. This is the working directory used when the task runs its command.

### 3.2 Run a Flow manually

```bash
# Create one manual run immediately.
stoker flow run <FLOW_ID>

# Provide a request ID so the same request can be safely retried.
stoker flow run <FLOW_ID> --request-id <UUID>

# Replace one specific future occurrence after the manual run starts.
stoker flow run <FLOW_ID> --replace-next --request-id <UUID>
```

You can safely resend a request with the same `--request-id`. `--replace-next` replaces one future occurrence after the manual run starts.

### 3.3 Inspect a run and its logs

```bash
stoker flow show <FLOW_ID> --run <RUN_ID>
stoker flow show <FLOW_ID> --run <RUN_ID> --task <TASK_ID>

# --attempt <N> starts at 1. Without it, show all attempts for the task.
stoker flow logs <FLOW_ID> --run <RUN_ID> --task <TASK_ID>
stoker flow logs <FLOW_ID> --run <RUN_ID> --task <TASK_ID> --attempt <N>
stoker flow logs <FLOW_ID> --run <RUN_ID> --task <TASK_ID> --follow
stoker flow logs <FLOW_ID> --run <RUN_ID> --task <TASK_ID> --attempt <N> --follow
```

### 3.4 Cancel a run or task

```bash
stoker flow cancel <FLOW_ID> --run <RUN_ID>
stoker flow cancel <FLOW_ID> --run <RUN_ID> --task <TASK_ID>
```

### 3.5 Complete operation example

The following commands show how to inspect the `nightly` Flow, run it manually once, and inspect or cancel one of its tasks:

```bash
stoker flow show nightly
stoker flow history nightly
stoker flow occurrences nightly

stoker flow run nightly --request-id <UUID>
stoker flow run nightly --replace-next --request-id <UUID>

stoker flow show nightly --run <RUN_ID> --task refresh
stoker flow logs nightly --run <RUN_ID> --task refresh --attempt 1 --follow
stoker flow cancel nightly --run <RUN_ID>
stoker flow cancel nightly --run <RUN_ID> --task refresh
```

## 4. Edit a committed Flow

A committed Flow cannot be edited directly. Follow these steps:

1. `freeze`: freeze the Flow and create a future draft.
2. Edit the task or schedule.
3. `apply`: apply the draft and unfreeze the Flow.

```bash
stoker flow edit begin nightly
stoker flow task update nightly publish --retries 2 --revision 0
stoker flow schedule set nightly --daily 01:00 --schedule-timezone Asia/Tokyo --revision 1
stoker flow edit apply nightly --revision 2
```

`--revision N` is the draft version. stoker uses it to confirm that you are editing the latest draft. The change is not applied when the version does not match.

These are the common task editing commands:

```text
stoker flow task update <FLOW_ID> <TASK_ID> --cmd <COMMAND>
stoker flow task update <FLOW_ID> <TASK_ID> --cwd <DIR>
stoker flow task update <FLOW_ID> <TASK_ID> --retries <N>
stoker flow task update <FLOW_ID> <TASK_ID> --after <TASK_ID>
stoker flow task update <FLOW_ID> <TASK_ID> --after-failure <TASK_ID>
stoker flow task update <FLOW_ID> <TASK_ID> --match all
stoker flow task update <FLOW_ID> <TASK_ID> --match any
stoker flow task update <FLOW_ID> <TASK_ID> --clear-dependencies

stoker flow task remove <FLOW_ID> <TASK_ID>
stoker flow task remove <FLOW_ID> <TASK_ID> --scope future
stoker flow task remove <FLOW_ID> <TASK_ID> --scope current --run <RUN_ID>
stoker flow task remove <FLOW_ID> <TASK_ID> --scope both --run <RUN_ID>
```

These are the schedule editing commands:

```text
stoker flow schedule set <FLOW_ID> --once-at <RFC3339>
stoker flow schedule set <FLOW_ID> --daily <HH:mm>
stoker flow schedule set <FLOW_ID> --daily <HH:mm> --schedule-timezone <IANA_ZONE>
stoker flow schedule set <FLOW_ID> --every <Nm|Nh>
stoker flow schedule set <FLOW_ID> --every <Nm|Nh> --first-at <RFC3339>
stoker flow schedule set <FLOW_ID> --first-at <RFC3339>
stoker flow schedule set <FLOW_ID> --schedule-timezone <IANA_ZONE>
```

`once`, `daily`, and `every` are three different schedule types. They cannot be exchanged. You can only edit a schedule using options from the same type.

You can add `--revision <N>` to the end of any editing command to confirm the draft revision.

### 4.1 Discard a draft, disable, or enable a Flow

```bash
stoker flow edit discard <FLOW_ID> --revision <N>
stoker flow disable nightly
stoker flow enable nightly
stoker request show <REQUEST_ID>
```

`flow edit discard` only discards the draft. The Flow remains frozen. Run `flow edit apply` to apply the draft and unfreeze the Flow.

`task remove` applies to `future` by default. To use `current` or `both`, provide `--run`, and make sure the Flow is frozen.

`disable` stops future automatic triggers. It does not prevent a valid manual run.

## 5. Standalone scheduled Job

In `scheduled` mode, `stoker create` can also create a scheduled Job that runs one command. You must still run `commit` to enable it.

```bash
stoker create --user alice --name frequent --cmd "python refresh.py" --every 15m --first-at 2099-01-01T10:00:00+09:00
stoker commit <JOB_ID>

stoker runs <JOB_ID>
stoker occurrences <JOB_ID>
stoker run <JOB_ID> --skip-next --request-id <UUID>
stoker logs <JOB_ID> --run <RUN_ID> --follow
stoker cancel <JOB_ID> --run <RUN_ID>
```

You can create a standalone scheduled Job with one of these three forms:

```text
stoker create --user <USER> --name <NAME> --cmd <COMMAND> --once-at <RFC3339> [--retry <N>]
stoker create --user <USER> --name <NAME> --cmd <COMMAND> --daily <HH:mm> [--schedule-timezone <IANA_ZONE>] [--retry <N>]
stoker create --user <USER> --name <NAME> --cmd <COMMAND> --every <Nm|Nh> [--first-at <RFC3339>] [--retry <N>]
```

`--retry` is the retry count for a standalone Job. Flow tasks use `--retries`.

To edit a committed schedule, freeze it first, make the change, and then unfreeze it:

```bash
stoker freeze <JOB_ID>
stoker schedule set <JOB_ID> --every 2h --expected-draft-revision <N>
stoker unfreeze <JOB_ID> --expected-draft-revision <N>
```

Other common operations are:

```bash
stoker draft discard <JOB_ID> --expected-draft-revision <N>
stoker disable <JOB_ID>
stoker enable <JOB_ID>
stoker run <JOB_ID> --skip-next --request-id <UUID>
```

`stoker draft discard` only discards the draft. The Job remains frozen.
`disable` and `enable` control automatic triggers.
`run --skip-next` replaces one future occurrence after the manual run starts.

## 6. Declarative JSON source mode

Use JSON source mode when you want to put a complete set of Flow definitions under version control or review it in a code review.
If you only need to create a regular Flow, you can skip this chapter.

The source mode determines whether Flows are edited one by one through the CLI or synchronized from one JSON file:

- `manual`: edit one Flow at a time with `flow create`, `flow task`, and other Flow commands.
- `sync`: use a JSON file as the complete desired state and synchronize all Flows at once. The desired state is the complete state you want the workspace to have.

Existing workspaces use `manual` by default.

### 6.1 Safe sync workflow

Before you start, confirm that:

- No work is starting, running, being cancelled, or recovering.
- Any unfinished Flow has been committed.
- Any frozen draft has been applied or discarded.

Then run these commands in order:

```bash
# 1. Export JSON from the current workspace.
# The exported file includes a base revision and hash.
stoker flow export --dir ./flow-definitions

# 2. Edit the JSON, lock the queue, and switch to sync mode.
stoker queue lock
stoker flow source-mode sync

# 3. Preview the result without writing any state.
stoker flow sync <EXPORTED_JSON> --dry-run

# 4. After reviewing the dry-run result, apply the Flow definitions.
stoker flow sync <EXPORTED_JSON>

# 5. Inspect the result. sync does not unlock the queue automatically.
stoker flow list
stoker queue unlock
```

Changing the source mode and running a formal sync both require the queue to be locked. No other execution can be active during these operations.

`sync` mode does not allow these editing commands:

- `flow create`
- `flow commit`
- `flow task`
- `flow schedule`
- `flow edit`
- `flow enable`
- `flow disable`

These operations are still available:

- Queries
- `run`
- `cancel`
- `log`
- `export`
- `snapshot`

### 6.2 Return to manual mode

To resume editing one Flow at a time, keep the queue locked, switch back to `manual`, and then unlock the queue:

```bash
stoker flow source-mode manual
stoker queue unlock
```

### 6.3 JSON v1 format

The only supported format is UTF-8 JSON. The safest starting point is always to run `flow export` first.

```json
{
  "schema_version": 1,
  "base": {
    "revision": 12,
    "hash": "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
  },
  "flows": [
    {
      "id": "nightly",
      "name": "Nightly publish",
      "owner": "alice",
      "enabled": true,
      "schedule": {
        "type": "daily",
        "time": "23:30",
        "timezone": "Asia/Tokyo"
      },
      "tasks": [
        {
          "id": "publish",
          "name": "Publish",
          "cwd": {
            "default": ".",
            "windows": "D:/work/site",
            "linux": "/srv/site",
            "macos": "/Users/alice/site"
          },
          "command": "python publish.py",
          "retry": 1,
          "depend_mode": "all",
          "depends_on": []
        }
      ]
    }
  ]
}
```

#### Flow and task order

- The order of the `flows` array is the Flow schedule order.
- The order of the `tasks` array is the task sequence.

#### Schedule fields

`schedule.type` can be one of these three values:

- `once`: uses `at`.
- `daily`: uses `time` and `timezone`.
- `periodic`: uses `every` and may also use `first_at`.

#### Task fields

- `command`: the command to run.
- `retry`: the number of retries after the task fails.
- `depend_mode`: how to evaluate multiple dependencies.
- `depends_on`: conditions on upstream tasks. Each condition has this form:

```json
{
  "task_id": "...",
  "status": "succeeded|failed"
}
```

#### The `cwd` field

`cwd` can be a string or a platform map:

```json
{
  "default": ".",
  "windows": "D:/work/site",
  "linux": "/srv/site",
  "macos": "/Users/alice/site"
}
```

stoker selects the working directory in this order:

1. The value for the current operating system.
2. `default` when there is no value for the current operating system.
3. The directory containing the definition file when neither value exists.

Relative paths are also resolved from the directory containing the definition file.

- The selected directory must already exist.
- Overrides for other operating systems are kept in the file, but their paths are not checked on the current machine.

#### JSON validation

The parser rejects:

- Unknown or duplicate fields
- Missing required fields
- Unsupported schemas
- Invalid schedules
- Duplicate IDs
- Missing dependencies
- Contradictory dependency edges
- Cyclic dependencies

Spelling mistakes are not ignored.

### 6.4 Revision, conflicts, and no-op

`base.revision` and `base.hash` describe the workspace state when you started editing.

If someone changes the workspace after you export the file, sync reports a stale conflict. Resolve it as follows:

1. Export again.
2. Apply your changes to the new file again.
3. Run dry-run again.

If two users synchronize different content from the same base at the same time, only one synchronization succeeds.

If the desired state is already the same as the current state, sync safely reports no-op even when the base is old. It does not increase the revision or create a duplicate snapshot.

### 6.5 Snapshots and source archives

```bash
stoker flow snapshot
```

`snapshot` is available in both `manual` and `sync` source modes.

- Snapshots are stored in `<STOKER_HOME>/flows/snapshots/`.
- The filename includes the UTC time, revision, and hash.
- A snapshot is created before a formal sync overwrites state.
- Sync input is saved in `<STOKER_HOME>/flows/sources/`.
- Equivalent content is deduplicated with SHA-256.
- Completed artifacts are read-only.
- The hash is verified when an artifact is read.
- If an artifact cannot be written, the desired state is not applied.

Sync uses the complete desired state.
Flows missing from the JSON are removed from future scheduling, but the following are not rewritten:

- Existing runs
- Attempts
- Logs
- Definition snapshots saved during execution

When a schedule is added or changed, old pending or reserved occurrences are replaced by the new schedule.

## 7. Recovery

If a run remains in `RECOVERING` after a restart, first confirm that the related process has stopped. Then run reconcile:

```bash
stoker recovery reconcile <RUN_ID> --confirm-stopped
stoker queue unlock
```

Unlock the queue only after all recovery work is complete.

## 8. Command naming and help

All Flow commands start with `stoker flow`.

Top-level scheduled-job commands accept only a standalone Job UUID. You cannot use a Flow ID in place of a Job UUID.

For complete option details, use:

```bash
stoker flow --help
stoker flow task --help
```

You can also add `--help` to any subcommand.
