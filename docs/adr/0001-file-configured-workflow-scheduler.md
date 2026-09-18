---
status: proposed
---

# Run workflows from a file-configured scheduler

RelayFold will provide an optional scheduler that starts workflow instances from cron-based entries in a configuration file. The scheduler will run as a Tokio background task within the orchestrator process, matching the orchestrator's existing asynchronous runtime rather than introducing a dedicated operating-system thread.

## Decision

Scheduling is disabled unless an environment variable explicitly enables it. A separate environment variable identifies the scheduler configuration file. The exact environment-variable names remain part of implementation design.

When scheduling is enabled, failure to find, read, parse, or validate the configuration file will not prevent the orchestrator from starting or affect workflow instances that have already been created. The scheduler will log the error on every one-minute interval and suspend all new scheduling until it can read a valid configuration. It will not retain and execute the last valid configuration while the current file is invalid.

The scheduler will wake once per minute. On every interval it will reread the complete configuration file before evaluating entries, allowing configuration changes without restarting the orchestrator. Removing or changing an entry will not cancel or otherwise affect workflow instances that were already created or enqueued.

Each schedule entry will contain:

- the namespace that owns the workflow
- the registered workflow definition ID
- a standard five-field cron expression with minute-level resolution
- optional workflow trigger input
- `run_on_startup`, a boolean that requests a startup-triggered run

Schedule entries will not have dedicated IDs. The scheduler will identify the workflow to start by its namespace and workflow definition ID.

`run_on_startup` replaces the previously considered missed-occurrence recovery behavior. RelayFold will not inspect workflow history, calculate a lookback window, or replay cron occurrences missed while the orchestrator was stopped. A configured startup run is a separate trigger from cron eligibility and does not depend on whether a manual or scheduled run previously completed.

The scheduler will not start a new occurrence while an instance of the configured workflow is already active. If no eligible worker host is available when a cron occurrence is evaluated, that occurrence is missed; the scheduler will wait for the next cron occurrence rather than retrying or catching it up.

Scheduler-specific provenance, including an entry ID, intended scheduled time, or startup-trigger marker, will not be added to workflow status or events. Structured error and execution logs are sufficient for this capability.

## Consequences

This design intentionally provides current-time cron triggering rather than durable schedule processing. Scheduler downtime, invalid configuration, and temporary worker unavailability can cause occurrences to be skipped. Conversely, the absence of catch-up state keeps the scheduler independent of persisted workflow history and avoids introducing durable schedule-occurrence records.

Because schedule entries have no dedicated identity, configurations that contain multiple entries for the same namespace and workflow definition need an explicit validation or execution rule before implementation.

## Outstanding decisions

- Whether schedules always use UTC or allow an IANA timezone per entry.
- Whether `(namespace, workflow_def_id)` must be unique within the configuration file.
- Whether `Paused` and `InputNeeded` instances block a new occurrence in addition to `Pending` and `Running` instances.
- Whether overlap prevention must be atomic against concurrent manual API triggers or may use a read-before-create check with a small race window.
- Whether `run_on_startup` is forfeited when the configuration is invalid during the initial startup evaluation or runs after the first successful configuration load.
