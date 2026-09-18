# Workflow scheduler implementation guide

This document explains how to implement the file-configured workflow scheduler described by [ADR 0001](../adr/0001-file-configured-workflow-scheduler.md). It is an implementation guide, not an implementation. Decisions still listed as outstanding in the ADR must be resolved before the behavior is considered complete.

## Desired behavior

When enabled, the scheduler runs as a Tokio task inside the orchestrator process. It wakes once per minute, rereads and validates the complete scheduler configuration, finds entries eligible at the current time, and attempts to create and enqueue their workflows.

The scheduler does not persist cron occurrences or reconstruct occurrences missed during downtime. Invalid configuration suspends the complete scheduling iteration but does not stop the orchestrator or affect workflow instances that have already been created.

```text
                    once per minute
┌──────────────┐   ┌──────────────────────┐
│ Tokio timer  │──▶│ WorkflowScheduler    │
└──────────────┘   │                      │
                   │ load configuration   │
┌──────────────┐   │ validate snapshot    │
│ YAML file    │──▶│ evaluate entries     │
└──────────────┘   │ prevent overlap      │
                   │ request workflow run │
                   └──────────┬───────────┘
                              ▼
                   ┌──────────────────────┐
                   │ WorkflowStarter      │
                   │                      │
                   │ select worker host   │
                   │ persist instance     │
                   │ enqueue instance     │
                   └──────────────────────┘
```

## Configuration contract

Use one canonical top-level field, `schedules`, matching the proposed YAML:

```yaml
schedules:
  - namespace: global-namespace
    workflow_def_id: daily-report
    cron: "0 12 * * *"
    run_on_startup: false
    input:
      ticker: TSLA
```

The corresponding models should remain small:

```rust
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct SchedulerConfig {
    schedules: Vec<ScheduleEntry>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct ScheduleEntry {
    namespace: Namespace,
    workflow_def_id: String,
    cron: String,
    #[serde(default)]
    run_on_startup: bool,
    input: Option<serde_json::Value>,
}
```

`run_on_startup` defaults to `false`, and an omitted `input` naturally deserializes to `None`. `deny_unknown_fields` ensures misspellings such as `run_on_startp` fail validation instead of silently changing behavior.

The current implementation sketch uses `SchedulerConfig { scheduler: ... }` while the YAML uses `schedules:`. These names must agree, either by renaming the Rust field to `schedules` or by adding an explicit Serde rename. Prefer matching names directly.

## Configuration and runtime settings

Two environment settings control the module:

```text
RELAYFOLD_SCHEDULER_ENABLED=true
RELAYFOLD_SCHEDULER_CONFIG_PATH=/etc/relayfold/scheduler.yaml
```

The names above are recommended but are not yet fixed by the ADR.

When scheduling is disabled, do not construct or spawn the scheduler. When it is enabled, a missing path setting, missing file, unreadable file, invalid YAML, invalid namespace, invalid cron expression, or invalid workflow reference must be reported without aborting orchestrator startup.

The enabled-value parser should follow the existing strict configuration style: accept `true` and `false`, choose an explicit default, and reject other values. Even a configuration-setting error must leave the main orchestrator operational when it affects only the optional scheduler.

## Split structural and storage-backed validation

Configuration validation has two different dependency classes and should be split accordingly.

Structural validation is local and synchronous:

- deserialize YAML
- validate required strings are non-empty
- parse every namespace
- parse every cron expression
- enforce the duplicate-entry policy

Reference validation requires storage and is asynchronous:

- verify that every `(namespace, workflow_def_id)` identifies a registered workflow definition

Keep the structural parser pure. Let the scheduler perform reference validation through `StoragePort` after loading the file and before triggering any entry. This prevents the file adapter from becoming coupled to workflow storage.

Validation functions should return `Result<()>`, not `Result<bool>`:

```rust
fn validate_config(config: &SchedulerConfig) -> anyhow::Result<()>;

fn validate_cron(expression: &str) -> anyhow::Result<ParsedCron>;

async fn validate_workflow_references(
    storage: &dyn StoragePort,
    config: &SchedulerConfig,
) -> anyhow::Result<()>;
```

`Ok(())` already means validation succeeded; a success boolean adds an unnecessary state. Functions should borrow entry fields, such as `&entry.cron` and `&entry.namespace`, rather than trying to move values out of a borrowed entry.

Validation is transactional at the snapshot level. Validate all entries before starting any workflow. If the final entry is invalid, earlier entries from that same file must not have run.

## Cron expression definition

Do not implement cron parsing manually. Use a parser that directly supports five-field expressions and can evaluate whether a supplied date and time matches an expression.

`croner` is a good fit because it accepts five-field patterns and exposes `is_time_matching`. If named timezones are selected later, it can evaluate timezone-aware `chrono` values. See the [Croner documentation](https://docs.rs/croner/latest/croner/struct.Cron.html).

The similar `cron` crate is less aligned with this contract because its documented expression format includes seconds and an optional year. See the [cron crate documentation](https://docs.rs/cron/latest/cron/).

Even when using a library, RelayFold must define the accepted language. For the initial implementation, prefer:

- exactly five whitespace-separated fields
- minute, hour, day of month, month, and day of week
- no seconds or year
- POSIX weekday numbering
- no Quartz compatibility
- no library-specific extensions unless explicitly added to the ADR

An illustrative parser shape is:

```rust
fn parse_cron(expression: &str) -> anyhow::Result<Cron> {
    if expression.split_whitespace().count() != 5 {
        anyhow::bail!("cron expression must contain exactly five fields");
    }

    expression.parse::<Cron>().context("invalid cron expression")
}
```

The exact parser configuration depends on the selected crate version. Keep the parsed cron value in the validated in-memory snapshot so an expression is parsed only once per scheduler iteration, not once for every check performed during that iteration.

## Scheduler module interface

The scheduler should be a deep module. Its caller should only need to provide dependencies and start it:

```rust
let scheduler = WorkflowScheduler::new(config_path, storage, workflow_starter);
tokio::spawn(scheduler.run());
```

The `run` implementation owns:

- the Tokio interval
- configuration reloads
- logging configuration failures
- startup-trigger state
- current-minute normalization
- cron matching
- duplicate-minute protection
- overlap handling
- conversion of workflow-start outcomes into logs

Avoid exposing separate public methods for loading, validating, matching, and triggering merely to make tests convenient. These are internal seams. Tests should exercise scheduler behavior through its small interface using in-memory storage, queue, and worker registry adapters where practical.

## One scheduling iteration

Conceptually, every iteration performs the following work:

```text
read file
   │
   ├─ error ──▶ log error and end iteration
   │
   ▼
parse and structurally validate all entries
   │
   ├─ error ──▶ log error and end iteration
   │
   ▼
validate all workflow references
   │
   ├─ error ──▶ log error and end iteration
   │
   ▼
calculate one normalized current minute
   │
   ▼
for each entry:
   due = startup_due OR cron_due
   │
   ├─ false ──▶ continue
   │
   ▼
attempt start-if-inactive
```

Startup and cron eligibility must be combined before attempting a start. If the process starts at noon and an entry has both `run_on_startup: true` and `cron: "0 12 * * *"`, it should produce at most one start attempt.

Use one timestamp for the whole iteration. Calling the clock independently for each entry could evaluate entries against different minutes when an iteration crosses a minute boundary.

## Tokio timer behavior

`tokio::time::interval(Duration::from_secs(60))` provides the basic loop. Configure its missed-tick behavior as `MissedTickBehavior::Skip`. Tokio's burst behavior is unsuitable because a delayed scheduler could perform several iterations immediately, which resembles catch-up behavior that the ADR rejects.

Retain the most recently evaluated wall-clock minute in memory. If the timer or surrounding logic attempts to evaluate that exact minute again, skip it. This matters when the first interval tick is immediate or after a runtime stall.

This minute marker is deliberately not durable. Restarting does not reconstruct missed cron occurrences.

## Reuse one workflow-starting module

The public HTTP handler currently performs three related operations directly:

1. select an eligible worker host
2. persist a new workflow instance
3. enqueue the instance

Move this behavior behind a cohesive `WorkflowStarter` module so the HTTP handler and scheduler cannot drift apart. The interface could be:

```rust
struct StartWorkflowRequest {
    namespace: Namespace,
    workflow_def_id: String,
    input: Option<serde_json::Value>,
}

enum StartWorkflowOutcome {
    Queued {
        instance_id: String,
        pinned_host_id: WorkerHostId,
    },
    SkippedActiveInstance,
    SkippedNoEligibleWorker,
    WorkflowDefinitionNotFound,
}
```

Expected conditions should be represented by outcomes rather than detected by matching error-message strings. The HTTP handler translates outcomes into HTTP status codes, while the scheduler translates them into structured logs.

The module may expose `start` for ordinary HTTP invocation and `start_if_inactive` for scheduled invocation. Both methods must reuse the same host-selection, persistence, and enqueue implementation.

## Prevent overlapping workflow runs

Do not use `StoragePort::list_workflow_info` alone to enforce the no-overlap invariant. Its interface permits collection reads to lag behind committed state, so a recently created active instance may be absent.

For the supported model of one orchestrator per storage partition, a reasonable implementation is:

1. acquire an in-process lock keyed by `(namespace, workflow_def_id)`
2. perform an authoritative active-instance query
3. if active, return `SkippedActiveInstance`
4. select an eligible worker host
5. create and persist the workflow instance
6. enqueue it
7. release the lock

This requires an authoritative storage operation such as:

```rust
async fn has_nonterminal_workflow_instance(
    &self,
    namespace: &Namespace,
    workflow_def_id: &str,
) -> StorageResult<bool>;
```

Implement it for memory, SQLite, and MySQL storage. If all workflow starts pass through `WorkflowStarter`, the keyed lock prevents races between scheduler and HTTP requests inside one orchestrator process.

Database-level atomic creation would be required to support multiple orchestrators sharing one storage database. RelayFold explicitly does not support that deployment model, so do not add that complexity for this capability.

The ADR still needs to decide which states block a new scheduled run. The recommended set is every non-terminal state:

- `Pending`
- `Running`
- `Paused`
- `InputNeeded`

Treating only `Pending` and `Running` as active would allow a recurring workflow waiting for human input to accumulate additional runs.

## No-worker behavior

For ordinary cron eligibility, absence of an eligible worker produces `SkippedNoEligibleWorker`. Log the condition and do not retry that occurrence. The next opportunity is the next cron occurrence.

`run_on_startup` needs a separate decision. The worker registry is empty when the orchestrator process first starts, and workers can register only after the worker-facing HTTP listener begins serving. An immediate startup evaluation will therefore usually miss its run:

```text
orchestrator starts
      │
      ├─ scheduler evaluates startup entry ── no worker ── skipped
      │
      └─ worker registers seconds later
```

The recommended behavior is to retain startup eligibility in memory until the scheduler can make one attempt with an eligible worker. This is not cron catch-up: it is completion of the current process's startup trigger. Allowing unpinned workflow creation would conflict with RelayFold's existing worker-host pinning requirement.

## Reload and removal behavior

The scheduler does not cache a last-known-good configuration. Every interval starts with a new load:

- valid snapshot: evaluate it
- invalid snapshot: log and schedule nothing
- corrected snapshot: use it on the next interval
- removed entry: stop considering it

Already-persisted or enqueued workflow instances are independent of later configuration changes and continue normally.

If a configuration is invalid for a scheduled minute and corrected afterward, the missed occurrence is not replayed.

## Duplicate entries without entry IDs

Without schedule-entry IDs, `(namespace, workflow_def_id)` is the only available identity. The recommended validation rule is to reject duplicate pairs:

```yaml
schedules:
  - namespace: global-namespace
    workflow_def_id: report
    cron: "0 9 * * *"

  - namespace: global-namespace
    workflow_def_id: report
    cron: "0 17 * * *" # duplicate pair
```

Multiple times can often be represented by one expression:

```yaml
cron: "0 9,17 * * *"
```

Different input payloads for different schedules of the same workflow cannot be represented under this rule. Supporting that use case would require restoring a dedicated schedule-entry ID.

## Suggested source layout

Keep the first implementation cohesive and avoid splitting every function into its own file:

```text
orchestrator/src/
├── core/
│   ├── scheduler.rs
│   └── workflow/
│       └── workflow_starter.rs
├── adapters/
│   └── file_schedule_source.rs
├── ports/
│   └── schedule_source.rs
└── main.rs
```

Possible responsibilities:

- `core/scheduler.rs`: configuration models, validated entry model, timer loop, eligibility policy, and logging.
- `ports/schedule_source.rs`: small interface for obtaining the current configuration snapshot.
- `adapters/file_schedule_source.rs`: asynchronous file read and YAML deserialization.
- `core/workflow/workflow_starter.rs`: overlap guard, worker selection, durable instance creation, and enqueue.
- `main.rs`: environment parsing, dependency construction, and spawning the Tokio task.

A separate schedule-source port is useful only if both the file adapter and a controlled test adapter are used. If tests can exercise the behavior cleanly with temporary files, keep file loading inside the scheduler module rather than adding a hypothetical seam.

## Incremental implementation order

Implement in slices that leave behavior testable after each step:

1. Finalize the outstanding ADR decisions.
2. Define and test YAML deserialization and structural validation.
3. Add and test five-field cron parsing against explicit timestamps.
4. Extract `WorkflowStarter` and make the existing HTTP handler use it without changing HTTP behavior.
5. Add authoritative active-instance storage queries for all storage adapters.
6. Add `start_if_inactive` and its concurrency tests.
7. Implement one scheduler iteration against injected dependencies and an explicit timestamp.
8. Add the Tokio interval around the tested iteration behavior.
9. Wire enablement and configuration-path settings in `main.rs`.
10. Add user-facing scheduler configuration and operations documentation under `website/`.

Extracting `WorkflowStarter` before writing the timer is important. It gives the scheduler one safe operation to call and prevents orchestration behavior from being duplicated inside the scheduling loop.

## Behavioral test matrix

At minimum, cover these scenarios:

- disabled scheduler performs no configuration reads or workflow starts
- enabled scheduler with a missing path logs an error and keeps running
- missing or unreadable file suspends only scheduling
- invalid YAML suspends the complete iteration
- unknown YAML fields are rejected
- one invalid cron expression suspends the complete iteration
- one unknown workflow reference suspends the complete iteration
- corrected configuration is used on the next interval
- due expression creates exactly one queued workflow instance
- non-due expression creates no instance
- startup and cron eligibility in the same minute create at most one instance
- repeated evaluation of the same minute creates no duplicate
- an active instance blocks a scheduled run
- no eligible worker skips an ordinary cron occurrence
- configuration removal does not affect an already-created workflow instance
- concurrent scheduler and HTTP starts follow the selected overlap policy
- startup triggering follows the selected worker-readiness policy
- timezone and daylight-saving transitions follow the selected timezone policy

Prefer tests that assert persisted workflow state and queue contents through existing module interfaces. Avoid tests that merely verify private helper call order.

## Outstanding decisions before implementation

Resolve these items in the ADR before treating the implementation contract as final:

1. Use UTC for all schedules or allow an IANA timezone per entry.
2. Reject duplicate `(namespace, workflow_def_id)` pairs or permit ambiguous duplicate entries.
3. Decide whether `Paused` and `InputNeeded` instances block new scheduled runs.
4. Confirm that process-level overlap serialization is sufficient.
5. Decide whether `run_on_startup` waits for worker readiness or is forfeited when no worker is initially available.
6. Decide whether a configuration that is invalid on the initial read can perform startup-triggered runs after its first successful load.
