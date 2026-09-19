---
title: Scheduled Workflows
description: Run registered workflows automatically from cron schedules.
---

RelayFold can start registered workflows automatically from a file-configured
scheduler. Scheduling is disabled by default.

## Configure schedules

Create a YAML file containing one or more schedule entries:

```yaml
scheduler:
  - namespace: global-namespace
    workflow_def_id: simple-function-workflow
    cron: "* * * * *"
    input:
      name: Scheduler
```

Each entry requires:

- `namespace`: the namespace containing the workflow definition
- `workflow_def_id`: the registered workflow definition to run
- `cron`: a standard five-field cron expression

`input` is optional. When present, RelayFold passes the value to each workflow
instance created by the schedule.

Cron expressions use UTC and have one-minute resolution. For example,
`0 12 * * *` runs daily at 12:00 UTC.

## Enable the scheduler

Enable scheduling and provide the configuration file path when starting the
orchestrator:

```bash
export RELAYFOLD_SCHEDULER_ENABLED=true
export RELAYFOLD_SCHEDULER_CONFIG_PATH=./scheduler.yaml
```

`RELAYFOLD_SCHEDULER_CONFIG_PATH` defaults to `./scheduler.yaml` when omitted.
Values other than `true` for `RELAYFOLD_SCHEDULER_ENABLED` leave scheduling
disabled.

For the development Docker Compose stack, place `scheduler.yaml` in the
repository root and change `RELAYFOLD_SCHEDULER_ENABLED` to `true` in
`docker-compose.yml`. The stack mounts that file at
`/etc/relayfold/scheduler.yaml`.

## Runtime behavior

The scheduler reads the complete configuration file once per minute, so saved
changes take effect without restarting the orchestrator. A missing, unreadable,
or invalid file prevents new scheduled runs during that evaluation and is
reported in the orchestrator logs. Existing workflow instances are unaffected.

RelayFold starts a scheduled workflow only when no instance of the same
workflow definition and namespace is active. `Pending`, `Running`, `Paused`,
and `InputNeeded` instances are active for this check. After an active instance
reaches `Completed` or `Failed`, the next matching cron occurrence can start a
new instance.

Scheduled occurrences are not durable. RelayFold does not catch up occurrences
missed while the orchestrator was stopped, its configuration was invalid, or no
eligible worker host was available. Removing or changing an entry does not
cancel workflow instances that it already started.
