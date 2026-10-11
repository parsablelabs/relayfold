# Should RelayFold storage become fully event-sourced?

Research date: 2026-09-29. Repository baseline: `d5e82fa`.

## Recommendation

**There is meaningful benefit in making workflow execution history verifiably replayable, but insufficient evidence to justify converting all storage to event sourcing.** Keep SQL and the current synchronous state reads. First prove that persisted workflow events reproduce committed state; add reconstruction tooling if that proves useful. Consider making events authoritative for workflow instances only when reconstruction becomes a product or operational requirement. Leave definitions and worker artifacts outside that change unless they have their own historical requirements.

RelayFold already has event-driven state transitions, persisted ordered history, a pure reducer, atomic event/state commits, and optimistic concurrency. A conversion would therefore primarily change **which representation is authoritative and whether current state can be rebuilt**, rather than introduce these capabilities. [R1–R4]

This recommendation is an engineering inference from the source and the first-party guidance below, not a measured performance result. No runtime behavior was changed by this investigation.

## What “fully event-sourced” could mean

These are different decisions:

1. **Event-sourced workflow instances:** their ordered event streams are the source of truth. Workflow/task/verifier rows become disposable derived state. Snapshots remain useful for fast reads.
2. **Event-sourced definition registries as well:** workflow and function registration, replacement, and deletion are represented by events, with current definitions derived from history.
3. **A durable execution journal:** also record dispatch intent, claims, leases, timeouts, and accepted results sufficiently to recover execution coordination. This is broader than changing the storage representation.
4. **Only store events, with no state tables or snapshots:** a performance choice, not a requirement of event sourcing. It would make current-state reads and discovery more expensive.

Event sourcing can use append-only SQL tables; it does not require Kafka, a dedicated event database, asynchronous projections, or replay on every read. Microsoft explicitly describes relational event stores and snapshots as optimizations; the stream remains authoritative. Synchronous projections can preserve RelayFold's current read-after-write behavior. [E1, R1]

For RelayFold, option 1 is the promising scope. Options 2 and 3 need separate justification. Option 4 is not recommended.

## What RelayFold does today

| Area | Observed behavior | Consequence |
| --- | --- | --- |
| Transition logic | `WorkflowStateManager` reduces events, assigns timestamps, increments version by event count, and submits events plus the new instance. | Much of the event-sourcing foundation already exists. |
| Authority | `StoragePort::get_workflow_instance` explicitly returns the authoritative committed snapshot. | The stream is not currently the documented source of truth. |
| SQL reads | SQLite and MySQL assemble instances from workflow, task, and verifier rows, not from history. | Events are not the normal recovery/read path. |
| SQL writes | Events and state rows commit together; only task rows identified by events are upserted, while verifier states are deleted and reinserted. | There is no database/event dual-write gap within a workflow transition; task writes are already selective. |
| Concurrency | Commits compare an expected workflow version; MySQL locks the workflow row with `FOR UPDATE`. | Full event sourcing would not newly provide stale-write protection. |
| Definitions | Workflow/function payloads are stored as current JSON. Workflow replacement after instances exist is blocked by workflow-service logic; functions can be replaced or deleted. | Instance history alone does not preserve all historical execution code/configuration. |
| Recovery | Startup discovers nonterminal workflows, loads state, records `StartupRecoveryApplied`, and requeues runnable instances. | Crash recovery already exists without full-history replay. |
| Dispatch | Pending work, leases, and result waiters live in process memory. | Persisting workflow events does not make dispatch coordination durable. |
| Worker artifacts | Sessions use a separate file store with best-effort persistence; workspaces are host-local execution storage. | Rebuilding orchestrator state would not recover lost sessions or files. |

Sources: [R1–R8]. The production workflow commit path inspected uses `WorkflowStateManager`; numerous test fixtures directly call storage with partial or empty event histories. Such fixtures are not evidence that production histories are incomplete, but they would need adjustment before enforcing stream authority. [R2, R3, R4, R9]

## Incremental benefits, rather than benefits already present

### 1. Historical state reconstruction — strongest fit

Replay could answer questions such as:

- What state and upstream input mapping did an attempt have before a human-input continuation?
- Which output was present before a retry cleared it?
- How did verifier feedback and generation selection evolve?
- What changed when a force retry moved a workflow to another host?

The event vocabulary already records task materialization, input data/mapping, outputs, satisfaction, verifier changes, human input, recovery, and retries. The reducer is side-effect-free, so reconstructing stored workflow state does not need to invoke an agent or external system. [R2, R3]

**Incremental value:** a supported “state at sequence N” view and reproducible state-transition debugging. Reading event history is already supported; full storage conversion is not required to add a replay viewer. [R6, R10]

**Limitation:** reconstructing state is not reproducing execution. The event history does not contain worker files, full agent sessions, historical resolved function code, or every dispatch lifecycle fact. [R3, R7, R8]

### 2. Rebuilding derived state — strongest reason to change authority

If history is complete and compatible with the reducer, workflow/task/verifier rows and summaries could be rebuilt after a projection bug or a read-model change. The log could support additional historical projections without changing workflow execution. This is the central architectural payoff of authoritative events. [E1]

Today, there is no inspected implementation that reconstructs a stored instance from its entire persisted history. SQL event rows also reference workflow rows with `ON DELETE CASCADE`, so deleting an instance row would delete its events. A genuine rebuild procedure must protect history and separate stream identity/head from disposable projections. [R3, R4, R11]

**Limitation:** replay cannot recover lost events or fix incorrectly recorded facts. Backups remain necessary. A reducer fix may repair a projection bug; a bad event needs an explicit correction or historical interpretation policy. [E1]

### 3. Better audits — conditional on event content

Event sourcing preserves history, but history only explains what the events contain. Several RelayFold events are field mutations (`TaskStatusChanged`, `TaskInputDataSet`) rather than complete business facts. `WorkflowStatusChanged { Failed }`, for example, does not identify whether the cause was worker-host loss, validation, or another failure. The event envelope contains a timestamp and event, without an explicit schema version, actor, cause, or command correlation. [R3, R5]

For stronger audits, selectively record the reason and authenticated actor where required. Prefer cohesive facts such as a task result or a pause request when they improve explanation. This can be done within the existing hybrid model; it does not justify adding speculative metadata to every event. Microsoft likewise distinguishes intent-focused events from state-change logs. [E1]

### 4. Performance and independent read models — plausible, unproven

An append plus stream-head update and occasional checkpoint may reduce synchronous state writes, especially the current wholesale verifier-state replacement. But the benefit is conditional:

- Changed task rows are already selectively persisted.
- Queries and startup recovery still need indexed current-state discovery.
- Per-workflow expected-version ordering still serializes competing transitions.
- The engine retains a reduced instance across much of an execution pass; replacing every point read with full replay would add work where no database load occurs today.
- Multiple mutation events per task result and growing payloads increase history/replay costs.
- Deferred projections shift work and introduce lag; they do not eliminate the work.

These observations follow from [R3–R5]. Event-sourcing guidance describes snapshotting and projection costs, and Temporal's first-party documentation places limits on workflow histories to preserve performance. Temporal is an illustrative design, not evidence that RelayFold should adopt its implementation or limits. [E1, E2]

**Do not claim a speedup without benchmarks.** Compare the existing adapter with targeted verifier-write optimization and an event-authoritative prototype using representative workflow sizes, payload sizes, generation counts, point reads, recovery scans, and contention.

## What a conversion would not solve

### Exactly-once external side effects

The engine records Running before dispatch. A worker can perform a side effect and the orchestrator can crash before committing the result. Startup recovery may dispatch the task again. An authoritative event log cannot determine whether an unrecorded external effect occurred. Tasks still need external idempotency/deduplication or compensation. [R5–R7, E3]

### Atomic state-to-queue/dispatch coordination

Retry/resume operations commit state and then enqueue separately. The engine commits Running and then dispatches separately. Full event sourcing alone does not close these gaps. [R5, R6]

If durable delivery is the goal, investigate a transactional outbox/durable dispatch intent with stable dispatch identity, idempotent consumers, and fencing of stale results. AWS describes the outbox as the solution to a database-plus-notification dual write, including duplicate-delivery handling. RelayFold's existing recovery provides a retry mechanism, but not an atomic dispatch contract. [E3, R6, R7]

### Distributed orchestrator ownership

Expected-version appends protect committed state but do not themselves establish execution ownership or fence an already-running worker. Multi-orchestrator execution would need coordination of queues, dispatch, ownership, and result acceptance in addition to storage changes. This is an inference from the in-process queue/dispatcher and commit ordering. [R1, R5–R7, R12]

### Reproducible reruns or undoing external work

Historical state replay should never invoke task dispatch. A rerun is a new execution, potentially against changed function definitions, tools, model behavior, and external systems. Undoing publication, file mutations, or payments requires domain-specific compensation. Historical code/configuration identity needs explicit pinning or capture; converting a definition registry into events does not by itself bind an execution to a particular definition version. [R5, R8, E1, E2]

## Gaps to address before declaring workflow events authoritative

1. **Prove replay parity.** Run real service/engine transitions; page persisted events; replay from creation; compare with committed state after each batch. Cover human input, retries, force retry, pauses with in-flight results, startup recovery, bounded loops, namespaces, and SQL reopen. Existing tests cover individual reductions and storage round trips, not end-to-end persisted-history parity. [R2–R4, R9]
2. **Handle version and summary metadata explicitly.** `reduce_workflow_instance_events` does not advance `WorkflowInstance.version`; the state manager does. Rehydration must set/check the version against committed sequence. Creation/modification/completion timestamps live in summary/storage logic and need an equivalent fold over event metadata. [R2–R4]
3. **Separate committed batches from arbitrary prefixes.** A logical result may consist of several events committed atomically. A state at an intermediate event sequence was not necessarily ever externally committed. Preserve or derive batch boundaries for historically accurate committed-state views; do not confuse a reducer prefix with a committed snapshot. [R1–R5]
4. **Strengthen the storage invariant.** The storage interface accepts both events and an independently supplied snapshot. Inspected adapters check expected version but do not verify replay equivalence or the documented `instance.version = expected_version + events.len()` invariant. The state manager maintains it on its normal path; full event authority needs enforcement at the appropriate seam. [R1–R4]
5. **Provide a committed stream reader.** The existing event-list interface permits lag, and `WorkflowEventRecord` does not expose sequence in each record. Authoritative replay needs a verified committed head, contiguous sequences, a bounded target version, and no lag in the missing tail. A collection/history interface allowed to lag is not sufficient as-is. [R1, R3, R4]
6. **Define event evolution.** Establish persisted event-schema versioning and compatibility tests for older payloads/reducer behavior. Instance sequence/version is not schema version. Embedded `WorkflowInstance`, `TaskInstance`, and verifier payloads couple event compatibility to current structs. [R3, E1]
7. **Protect history during rebuild.** Revise event foreign-key lifecycle before treating workflow rows as disposable. Define stream-head/checkpoint consistency and atomic append behavior, including failure, conflict, and repeated-request handling. [R11, E1]
8. **Make recovery discovery reliable.** If summaries become asynchronous, startup must catch projections up to a known watermark or use an authoritative nonterminal index. The current interface allows list lag, while recovery relies on list discovery; re-reading discovered IDs cannot recover an undiscovered workflow. Current SQL adapters derive lists from committed state rows. [R1, R4, R6]
9. **Decide retention deliberately.** Events include trigger input, human input, task inputs/outputs, and verifier data, potentially including personal or sensitive content. Immutable retention needs an explicit deletion/redaction strategy and replay semantics. SQL already stores these payloads in history; event authority would make their lifecycle more consequential. [R3, R11, E1]

An additional current-state concern, independent of event sourcing: SQL point reads assemble workflow/task/verifier rows using separate pool queries, without an enclosing read transaction in the inspected methods. Concurrent commits could therefore yield a mixed-version assembly. This deserves a targeted consistency test and, if confirmed under supported isolation, a consistent-read fix; a whole event-sourcing migration is not required. [R4]

## Compare the reasonable options

| Option | New capability | Cost/risk | Recommendation |
| --- | --- | --- | --- |
| Keep hybrid; add replay/parity tests and diagnostic reconstruction | Historical debugging; detect divergence | Small-to-moderate; event compatibility still matters | **First step** |
| Make workflow streams authoritative; keep synchronous SQL state rows/checkpoints | Supported rebuilds; explicit provenance | Moderate; stream lifecycle, schema evolution, rebuild/recovery guarantees | Adopt when rebuilds/history are required |
| Append-only workflow writes with deferred projections | Potential lower command-path latency; independent read projections | Higher; checkpoints, lag, discovery, idempotent projection workers | Only after evidence of a write bottleneck |
| Event-source workflow/function registries too | Historical registration/deletion audit | More streams and lifecycle rules; execution pinning still required | Not justified by inspected requirements |
| Journal dispatch/leases/results durably | Better coordination recovery | Significant execution protocol change | Separate reliability project, not a storage shortcut |
| Remove all state tables/snapshots | Minimal persisted state representations | Expensive replay and discovery | Avoid |

The comparisons are recommendations/inferences based on the observed architecture and trade-offs in [E1–E3], not benchmark conclusions.

## Suggested next investigation, without committing to migration

1. Build a replay-parity test harness and historical diagnostic reader using the existing pure reducer. Retain snapshot authority during this exercise.
2. Determine whether the actual requirement is historical inspection, projection repair, durable dispatch, or higher write throughput. These point to different changes.
3. If repair/rebuild is required, prototype **workflow-instance-only event authority with synchronous SQL projections**, preserving fast, committed point reads and namespace-scoped concurrency. Keep replay and checkpoint complexity inside a cohesive workflow-state module rather than exposing it to every caller.
4. Benchmark before introducing deferred projections or periodic checkpoints. Optimize selective verifier persistence separately if that is the measured bottleneck.
5. Decide how to handle existing histories: validate and preserve them, seed a baseline event/checkpoint with an explicit historical cut-off, or require a destructive reset. Never promise pre-cut-off reconstruction from a baseline alone.

No `openspec/specs/` directory exists in this checkout. This is a research note, not a new specification or an architecture decision. Making streams authoritative would change the existing storage interface's documented authority and must be agreed before implementation.

## Validation and limitations

- Read the storage interface, memory/SQLite/MySQL implementations, MySQL schema, workflow events/reducer/state manager, engine, workflow/function services, recovery/queue coordination, dispatcher, worker session persistence, and relevant user documentation.
- Ran `cargo test --manifest-path orchestrator/Cargo.toml --lib`: **216 passed**. This validates the existing library tests, not a new replay implementation.
- Did not run the external MySQL integration test, benchmark storage, inspect a production database, or prove completeness/compatibility of existing persisted histories.
- External references were fetched directly from the owning vendors' documentation on the research date.

## Sources

### RelayFold primary sources

- **R1:** [Storage interface and consistency contract](../orchestrator/src/ports/storage.rs), especially `get_workflow_instance`, collection/history reads, and `save_workflow_instance`.
- **R2:** [Workflow state manager](../orchestrator/src/core/workflow/state_manager.rs), reduction, metadata/version assignment, and conflict tests.
- **R3:** [Workflow events and reducer](../orchestrator/src/core/workflow/events.rs), event payloads, changed-task extraction, and pure reduction.
- **R4:** [SQLite storage](../orchestrator/src/adapters/sqlite_storage.rs), [MySQL storage](../orchestrator/src/adapters/mysql_storage.rs), and [memory storage](../orchestrator/src/adapters/memory_storage.rs), point reads, transition commits, and summary persistence/querying.
- **R5:** [Workflow engine](../orchestrator/src/core/engine.rs), `run_workflow_instance`, task result batches, and function resolution before dispatch.
- **R6:** [Orchestrator](../orchestrator/src/core/orchestrator.rs) and [workflow service](../orchestrator/src/core/workflow/workflow_service.rs), creation, replacement, retry, pause/resume, startup recovery, and discovery.
- **R7:** [Task dispatcher](../orchestrator/src/adapters/task_dispatcher.rs), pending/in-flight state, dispatch identity, result waiters, timeouts, and stale-result tests.
- **R8:** [Function service and reference resolution](../orchestrator/src/core/function/function_service.rs), [worker session interface/persistence](../worker/src/core/ports/SessionStore.ts), [file session store](../worker/src/adapters/FileSessionStore.ts), and [reliability documentation](../website/src/content/docs/docs/operations/reliability.md).
- **R9:** [Engine tests](../orchestrator/src/core/engine_tests.rs), state-manager/reducer/storage tests in R2–R4, and [MySQL integration test](../orchestrator/tests/mysql_storage.rs).
- **R10:** [Workflow lifecycle documentation](../website/src/content/docs/docs/concepts/workflow-lifecycle.md) and [event endpoint documentation](../website/src/content/docs/docs/api-reference.md#workflow-events).
- **R11:** [MySQL schema](../orchestrator/migrations/mysql/001_initial_schema.sql) and SQLite `INITIAL_SCHEMA_SQL` in R4, including event keys and cascading foreign keys.
- **R12:** [In-process workflow queue](../orchestrator/src/adapters/memory_workflow_queue.rs).

### External first-party sources

- **E1:** Microsoft Azure Architecture Center, [Event Sourcing pattern](https://learn.microsoft.com/en-us/azure/architecture/patterns/event-sourcing). Source-of-truth distinction, selective adoption, relational event stores, event intent, concurrency, snapshotting, compatibility, projections, and privacy trade-offs.
- **E2:** Temporal, [Events and Event History](https://docs.temporal.io/workflow-execution/event). Durable execution history, activity lifecycle facts, bounded history, reset behavior, and recorded nondeterministic results instead of rerunning them on replay.
- **E3:** AWS Prescriptive Guidance, [Transactional outbox pattern](https://docs.aws.amazon.com/prescriptive-guidance/latest/cloud-design-patterns/transactional-outbox.html). Database/message dual-write failure modes, atomic outbox persistence, ordered delivery, and idempotent consumption.
