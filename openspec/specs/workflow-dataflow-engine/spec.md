# Capability: workflow-dataflow-engine

## Purpose
Defines how the orchestrator constructs and executes a workflow DAG based on data dependencies, transitioning tasks dynamically as their inputs are satisfied by upstream task outputs.

## Requirements

### Requirement: Workflow Definition Identifier Normalization
The orchestrator SHALL normalize workflow definition IDs and task definition IDs to lowercase during workflow registration and SHALL reject those definition IDs when they contain non-alphanumeric characters.

#### Scenario: Uppercase IDs are registered
- **WHEN** a workflow definition contains uppercase workflow definition or task definition IDs using only ASCII alphanumeric characters
- **THEN** the system registers those IDs in lowercase form

#### Scenario: ID contains brackets
- **WHEN** a workflow definition or task definition ID contains `[` or `]`
- **THEN** the system rejects the workflow definition

#### Scenario: ID contains non-alphanumeric character
- **WHEN** a workflow definition or task definition ID contains a character other than an ASCII letter or digit
- **THEN** the system rejects the workflow definition

#### Scenario: Generated task attempt ID contains brackets
- **WHEN** the orchestrator materializes an internal task attempt ID such as `taska[1]`
- **THEN** that generated attempt ID is not subject to workflow definition ID validation

### Requirement: Workflow Definition Immutability After Instantiation
The orchestrator SHALL reject overwriting an existing workflow definition after any workflow instance has been created for that definition, regardless of the instance state. The orchestrator SHALL NOT require workflow definition IDs to follow a versioning scheme.

#### Scenario: Definition overwrite before instances
- **WHEN** a workflow definition is registered with the same normalized ID as an existing definition
- **AND** no workflow instances exist for that workflow definition ID
- **THEN** the system overwrites the existing workflow definition

#### Scenario: Definition overwrite after instance creation
- **WHEN** a workflow definition is registered with the same normalized ID as an existing definition
- **AND** at least one workflow instance exists for that workflow definition ID
- **THEN** the system rejects the workflow definition overwrite with `409 Conflict`
- **AND** the existing workflow definition remains unchanged
- **AND** the error suggests registering a new ID by appending a suffix such as `_v2`

#### Scenario: Definition overwrite after terminal instance creation
- **WHEN** a workflow definition is registered with the same normalized ID as an existing definition
- **AND** a completed or failed workflow instance exists for that workflow definition ID
- **THEN** the system rejects the workflow definition overwrite

#### Scenario: Workflow definition ID has no version suffix
- **WHEN** a valid workflow definition ID does not contain a version suffix
- **THEN** the system accepts the ID
- **AND** the system does not interpret or enforce suffixes such as `_v2`

### Requirement: Bounded Backedge Validation
The orchestrator SHALL preserve ordinary data binding cycle validation and SHALL allow bounded verifier backedges only through `control.verifier` configuration.

#### Scenario: Ordinary data binding cycle
- **WHEN** workflow data bindings contain a cycle such as `A -> B -> A`
- **THEN** the workflow definition is rejected

#### Scenario: Explicit bounded backedge
- **WHEN** workflow data bindings contain `A -> B -> C -> D` and task `D` declares `control.verifier.rerun_from_task_id: B`
- **THEN** the workflow definition is accepted as a bounded verifier-controlled rerun

#### Scenario: Verifier self-rerun
- **WHEN** task `D` declares `control.verifier` without `rerun_from_task_id`
- **THEN** the workflow definition is accepted and verifier `continue` reruns only `D`

#### Scenario: Backedge target is downstream
- **WHEN** task `B` declares `control.verifier.rerun_from_task_id: D` for downstream task `D`
- **THEN** the workflow definition is rejected

#### Scenario: Verifier slices overlap
- **WHEN** multiple verifier controls create rerun slices that share any task
- **THEN** the workflow definition is rejected

### Requirement: Task Instance Lifecycle Management
The orchestrator SHALL transition each materialized `TaskInstance` through a lifecycle and SHALL track satisfaction separately from lifecycle completion.

#### Scenario: Valid inputs trigger execution
- **WHEN** all input schemas of a `Pending` materialized task instance are satisfied by upstream data bindings
- **THEN** the task status transitions from `Pending` to `Running`

#### Scenario: Workflow initialization without verifier backedges
- **WHEN** a `WorkflowInstance` is initialized from a `WorkflowDef`
- **THEN** generation-1 task attempts are materialized for the static workflow graph

#### Scenario: Bounded retry generation materialization
- **WHEN** a verifier-controlled generation returns `continue` and has remaining iteration budget
- **THEN** the next generation is materialized only for tasks in the verifier rerun slice

#### Scenario: Bounded generation completion
- **WHEN** a verifier-controlled generation produces schema-valid outputs and its verifier returns `complete`
- **THEN** the generation is marked satisfied and downstream tasks after the verifier become eligible

#### Scenario: Rejected bounded generation
- **WHEN** a verifier-controlled generation produces schema-valid outputs and its verifier returns `continue`
- **THEN** the generation remains lifecycle `Completed` but is marked unsatisfied for downstream binding

### Requirement: Data Binding Resolution
The orchestrator SHALL construct executable workflow dataflow from the `DataBinding`s in the `WorkflowDef`, resolving source task IDs to concrete materialized attempts by generation scope and satisfaction state.

#### Scenario: Sequential propagation
- **WHEN** Task A completes successfully outside verifier-controlled rerun handling
- **THEN** the output of Task A is mapped to the input payload of Task B according to the defined `DataBinding`

#### Scenario: Fan-In propagation
- **WHEN** Task C requires inputs from both Task A and Task B
- **THEN** Task C SHALL NOT transition to `Running` until both Task A and Task B have successfully completed and populated their respective input bindings on Task C

#### Scenario: Fan-In input order
- **WHEN** a task has multiple inbound data bindings
- **THEN** its input array SHALL contain source outputs in workflow definition data-binding order

#### Scenario: Latest materialized propagation inside rerun slice
- **WHEN** a verifier rerun slice contains source task `B` and target task `C`
- **THEN** `C` resolves `B` to the latest materialized attempt for `B`
- **THEN** `C` does not run until that latest materialized attempt is completed

#### Scenario: Selected generation propagation after verifier
- **WHEN** verifier task `D[2]` is accepted
- **THEN** downstream tasks bound to `D` receive output from `D[2]`

#### Scenario: Rejected generation does not propagate after verifier
- **WHEN** verifier task `D[1]` is rejected and another generation will run
- **THEN** downstream tasks bound after `D` do not receive output from `D[1]`

#### Scenario: Input mapping records resolved attempts
- **WHEN** a materialized task receives propagated inputs
- **THEN** the task records `input_mapping` for each consumed source task ID and generation

### Requirement: Human Input Continuation Materialization
The workflow engine SHALL materialize human-input continuations with the same logical task identity and workflow pin needed for dataflow, workspace, and Agent session reuse.

#### Scenario: Human input continuation is materialized
- **WHEN** human input is submitted for a waiting Agent task
- **THEN** the workflow engine materializes or resumes a continuation attempt for the same logical task ID
- **THEN** the continuation attempt carries generation lineage that preserves downstream data binding behavior

#### Scenario: Continuation preserves workflow pin
- **WHEN** a human-input continuation is prepared
- **AND** the workflow instance has a host pin
- **THEN** the workflow engine preserves the workflow instance host pin

### Requirement: Agent Session Key Convention
The orchestrator SHALL provide stable task execution identity that allows workers to derive Agent session keys when Agent execution creates or reuses a durable session.

#### Scenario: Reusable Agent attempt has logical key inputs
- **WHEN** an Agent task attempt has `reuse_session` set to `true`
- **THEN** the worker can derive the session key from the workflow instance ID and logical task ID

#### Scenario: Agent continuation attempt is materialized
- **WHEN** the engine materializes a later attempt for the same logical Agent task due to human input or verifier feedback
- **THEN** the later attempt carries its `generation_index`
- **THEN** the worker can derive the same session key for that logical Agent task when `reuse_session` is true

#### Scenario: Human-input continuation preserves session identity
- **WHEN** the public human-input submission flow resumes an Agent task with `reuse_session` set to `true`
- **THEN** the resumed attempt preserves workflow instance ID and logical task ID for session key derivation

#### Scenario: Agent session reuse is disabled
- **WHEN** an Agent task attempt has `reuse_session` set to `false`
- **THEN** the worker does not load prior logical-task conversation history

#### Scenario: Non-Agent attempt is materialized
- **WHEN** the engine materializes a Function or API call task attempt
- **THEN** the attempt does not require Agent session key metadata

### Requirement: Agent Session Keys Preserve Workflow Determinism
Agent session keys SHALL preserve conversational continuity without replacing structured workflow attempt state.

#### Scenario: Session-backed attempt has lineage
- **WHEN** an Agent task attempt uses a durable session key
- **THEN** the task attempt still records its task status, satisfaction status, generation index, input mapping, and attempt cause metadata

#### Scenario: Downstream bindings resolve outputs
- **WHEN** downstream task input binding resolves an Agent task output
- **THEN** binding resolution uses completed and satisfied task attempts rather than reading the Agent session transcript

#### Scenario: Waiting task reports input needed
- **WHEN** an Agent task attempt is waiting for human input
- **THEN** the workflow state records the `InputNeeded` status and input request independently of the Agent session transcript

### Requirement: Agent Session Recovery Lifecycle
The workflow engine SHALL keep workflow attempt state independent of recoverable Agent session-load problems.

#### Scenario: Worker recovers from session-load problem
- **WHEN** an Agent task attempt cannot load its expected reusable session
- **THEN** the worker logs the session-load problem and continues execution with a fresh session
- **THEN** the workflow task status is determined by the Agent execution result

#### Scenario: Recovered continuation does not bypass validation
- **WHEN** an Agent continuation attempt recovers from a session-load problem
- **THEN** downstream binding satisfaction still depends on the task execution result and normal verifier/dataflow rules

### Requirement: Workspace Group Definition Validation
The orchestrator SHALL validate task `workspace.group_name` declarations during workflow registration.

#### Scenario: Valid workspace group is registered
- **WHEN** a task declares `workspace.group_name` using a valid workspace group identifier
- **THEN** the workflow definition is accepted

#### Scenario: Invalid workspace group is rejected
- **WHEN** a task declares `workspace.group_name` using an invalid workspace group identifier
- **THEN** the workflow definition is rejected

#### Scenario: Multiple workspace groups are rejected
- **WHEN** a task definition attempts to declare more than one workspace group
- **THEN** the workflow definition is rejected

### Requirement: Workspace Group Selection
The workflow engine SHALL select either the default private workspace or the declared workspace group for each task.

#### Scenario: Task has no workspace group
- **WHEN** a task definition omits `workspace.group_name`
- **THEN** the workflow engine selects the task's private logical-task workspace

#### Scenario: Task has workspace group
- **WHEN** a task definition declares `workspace.group_name`
- **THEN** the workflow engine selects a group workspace identity derived from the workflow instance id and normalized group name
- **THEN** the task's private logical-task workspace is not selected for that execution

#### Scenario: Same group resolves same workspace identity
- **WHEN** multiple task definitions in a workflow instance declare the same `workspace.group_name`
- **THEN** the workflow engine resolves those tasks to the same group workspace identity

### Requirement: Workspace Groups Do Not Define Scheduling
Workspace group membership SHALL NOT create implicit task dependencies or change data binding scheduling behavior.

#### Scenario: Shared workspace without data dependency
- **WHEN** two tasks declare the same `workspace.group_name` but no data binding or control dependency orders them
- **THEN** the workflow engine does not infer an execution order from the shared workspace group

#### Scenario: Data binding still controls scheduling
- **WHEN** task B depends on task A through a JSON data binding and both tasks declare the same `workspace.group_name`
- **THEN** task B remains ineligible until task A satisfies the normal data binding requirements

### Requirement: Workflow Instance Events
The orchestrator SHALL represent workflow instance state transitions as ordered domain events.

#### Scenario: Workflow transition emits event
- **WHEN** the workflow engine or workflow service changes workflow instance state
- **THEN** the change is representable as one or more workflow instance events

#### Scenario: Event order is meaningful
- **WHEN** multiple events are produced for one workflow instance transition
- **THEN** the system preserves their order for reducer application and persistence

### Requirement: Core Event Reduction
The orchestrator SHALL apply workflow instance events through core reducer logic outside storage adapters.

#### Scenario: Reducer applies task event
- **WHEN** a task attempt lifecycle event is applied to a workflow instance snapshot
- **THEN** core reducer logic updates the corresponding task attempt state

#### Scenario: Reducer applies verifier event
- **WHEN** a verifier state event is applied to a workflow instance snapshot
- **THEN** core reducer logic updates the verifier state and affected task attempt metadata

#### Scenario: Storage adapter does not interpret event
- **WHEN** a storage adapter receives workflow instance events to persist
- **THEN** it stores the events without applying workflow transition rules

### Requirement: Event-Backed Snapshot Persistence
The orchestrator SHALL persist workflow instance event batches and the resulting `WorkflowInstance` snapshot as one transition commit.

#### Scenario: Event batch updates snapshot
- **WHEN** core code commits an ordered batch of workflow instance events
- **THEN** the system applies the events to the current snapshot and persists the updated snapshot

#### Scenario: Event batch is persisted
- **WHEN** core code commits an ordered batch of workflow instance events
- **THEN** the system appends timestamped event records for that workflow instance

#### Scenario: Event batch commit updates summary
- **WHEN** core code commits an ordered batch of workflow instance events
- **THEN** storage receives the event records and reduced workflow snapshot in one commit operation
- **THEN** storage updates any lightweight summary projection from the reduced workflow snapshot
- **THEN** summary lifecycle timestamps are derived from workflow event record timestamps, not embedded snapshot fields

#### Scenario: Event record contains occurrence time
- **WHEN** core code commits workflow instance events
- **THEN** each persisted workflow event record includes a `created_time` value

#### Scenario: Event history is paginated
- **WHEN** a caller requests workflow instance events with a page limit and optional sequence cursor
- **THEN** the system returns events in ascending event-sequence order
- **THEN** the response includes a next-sequence cursor only when more events remain
- **THEN** durable storage performs a bounded read rather than loading the complete event history before pagination

#### Scenario: Empty event batch
- **WHEN** core code attempts to commit an empty event batch
- **THEN** the system rejects the operation without saving a snapshot

### Requirement: Snapshot-Backed Workflow Reads
The orchestrator SHALL continue serving full current workflow instance reads from snapshots and list queries from lightweight summary data produced with snapshots.

#### Scenario: Workflow instance read
- **WHEN** a caller requests a workflow instance by ID
- **THEN** the system returns the latest saved workflow instance snapshot

#### Scenario: Workflow list read
- **WHEN** a caller lists workflow instances
- **THEN** the system returns lightweight summary data produced by core from saved workflow instance snapshots
- **THEN** the list operation does not return full workflow instance state
- **THEN** the list operation does not load each full workflow instance snapshot to assemble the result

#### Scenario: Workflow list without filters
- **WHEN** a caller lists workflow instances with no filters
- **THEN** the system returns a bounded page of workflow instance summaries

#### Scenario: Workflow list state filter
- **WHEN** a caller lists workflow instances with a workflow state filter
- **THEN** the system returns only summaries matching that filter

#### Scenario: Workflow list multi-state filter
- **WHEN** a caller lists workflow instances with multiple workflow states
- **THEN** the system returns only summaries whose workflow status is included in that set

#### Scenario: Workflow list definition filter
- **WHEN** a caller lists workflow instances with a workflow definition ID filter
- **THEN** the system returns only summaries for that workflow definition ID

#### Scenario: Workflow list combined filters
- **WHEN** a caller lists workflow instances with multiple filters
- **THEN** the system returns only summaries matching all filters

#### Scenario: Workflow list sort order
- **WHEN** a caller lists workflow instances
- **THEN** the system returns summaries sorted by most recent modification time first
- **THEN** summaries with equal modification times are sorted by workflow instance ID descending

#### Scenario: Workflow list pagination
- **WHEN** more workflow instance summaries match than the requested page limit
- **THEN** the system returns at most the requested limit
- **THEN** the system returns a cursor that can retrieve the next page after the last returned summary

#### Scenario: Active workflow discovery
- **WHEN** the orchestrator discovers active workflow instances
- **THEN** the system pages through workflow summaries using a multi-state filter for pending and running workflow statuses

#### Scenario: Full workflow instance read is explicit
- **WHEN** a caller needs task inputs, task outputs, verifier history, or other full workflow instance state
- **THEN** the caller retrieves that state through the workflow instance get-by-ID operation

### Requirement: Workflow Instance Summary Fields
The orchestrator SHALL expose storage-level workflow instance list results as lightweight summaries.

#### Scenario: Summary contains identity and state
- **WHEN** a workflow instance appears in a storage-level list result
- **THEN** the summary includes the workflow instance ID, workflow definition ID, and current workflow state

#### Scenario: Summary contains lifecycle timestamps
- **WHEN** lifecycle timestamps are available for a workflow instance
- **THEN** the summary includes created time, modified time, and completed time

#### Scenario: Summary contains task counts
- **WHEN** a workflow instance appears in a storage-level list result
- **THEN** the summary includes total task count and completed task count produced from the saved snapshot

### Requirement: Workflow Definition Discovery
The orchestrator SHALL expose registered workflow definitions as compact summaries without returning task definitions, data bindings, or other full definition payload data.

#### Scenario: List registered workflow definitions
- **WHEN** a caller requests the workflow definition list
- **THEN** the system returns a summary for every registered workflow definition
- **THEN** each summary includes the workflow definition ID, description, and creation time
- **THEN** the summaries are sorted by creation time descending and then workflow definition ID descending

#### Scenario: List an invoked workflow definition
- **WHEN** a registered workflow definition has one or more workflow instances
- **THEN** its summary includes the creation time of its most recently created workflow instance as the last invoked time

#### Scenario: List a never-invoked workflow definition
- **WHEN** a registered workflow definition has no workflow instances
- **THEN** its summary has no last invoked time

#### Scenario: Persist workflow description metadata
- **WHEN** a workflow definition is registered with a description
- **THEN** storage persists the description with the workflow definition
- **THEN** the workflow definition summary exposes that description without loading the full workflow definition payload

### Requirement: Definition API Formats
The orchestrator SHALL accept workflow and function definition documents as JSON or YAML while keeping JSON as the canonical persisted representation. The orchestrator SHALL return complete workflow definitions as JSON by default and YAML when requested.

#### Scenario: Register a JSON or YAML definition
- **WHEN** a caller posts a valid JSON or YAML workflow or function definition with any or no `Content-Type`
- **THEN** the API detects and parses the document format
- **THEN** a YAML `Content-Type` may be used as a parsing hint
- **THEN** storage receives the same typed definition model for canonical JSON persistence

#### Scenario: Retrieve a complete definition in the requested format
- **WHEN** a caller gets a complete workflow definition with `format=json` or without a `format` query parameter
- **THEN** the API returns JSON
- **WHEN** a caller gets a complete workflow definition with `format=yaml`
- **THEN** the API returns YAML

### Requirement: Storage Adapter Boundary
Storage adapters SHALL be responsible for persistence mechanics and SHALL NOT own workflow event semantics.

#### Scenario: Memory storage commits events
- **WHEN** memory storage commits workflow instance event records with a reduced snapshot
- **THEN** it stores the event records without deciding how their payloads affect workflow, task, or verifier state

#### Scenario: Commit receives reduced state
- **WHEN** storage commits workflow instance event records and snapshot state
- **THEN** the snapshot has already been produced by core logic
- **THEN** any summary projection is derived from the committed snapshot, not from event payload semantics

#### Scenario: Event batch identifies changed tasks
- **WHEN** a workflow event changes or creates task-attempt state
- **THEN** the event identifies every affected task attempt ID
- **THEN** startup recovery records the exact running task attempt IDs it resets

#### Scenario: Durable storage persists event-identified tasks
- **WHEN** durable storage commits workflow events and the resulting workflow snapshot
- **THEN** a core-owned exhaustive event classifier supplies the affected task attempt IDs
- **THEN** storage writes the authoritative task records for those IDs from the resulting snapshot
- **THEN** storage creates missing task records and updates existing task records without bulk replacement or deletion
- **THEN** storage does not reapply workflow event transition semantics

#### Scenario: SQL storage persists event-identified tasks
- **WHEN** SQL storage commits a transition whose events identify changed tasks
- **THEN** it upserts only those task rows without replacing unchanged task rows

#### Scenario: SQL storage reconstructs aggregate state
- **WHEN** SQL storage persists workflow instance state across workflow, task, verifier, and event tables
- **THEN** full workflow instance reads reconstruct the same `WorkflowInstance` aggregate shape used by core workflow code
- **THEN** memory storage may continue storing the aggregate directly

#### Scenario: SQL storage derives summaries
- **WHEN** SQL storage serves workflow summary list requests
- **THEN** it may derive summary fields from workflow instance and task attempt tables instead of storing a separate summary projection table

#### Scenario: SQL storage keeps task status detail
- **WHEN** SQL storage persists a task attempt whose status carries additional data such as an input request
- **THEN** the persisted task row stores enough task status detail to reconstruct the full task attempt

### Requirement: Durable SQL Backend Selection
The orchestrator SHALL select a supported durable SQL adapter directly from `RELAYFOLD_STORAGE` while preserving the same `StoragePort` behavior for each supported backend.

#### Scenario: SQLite storage configuration
- **WHEN** `RELAYFOLD_STORAGE` is `sqlite` and a non-empty `RELAYFOLD_STORE_SQLITE_PATH` is configured
- **THEN** the orchestrator uses the SQLite storage adapter
- **THEN** the adapter opens or creates the database at the configured filesystem path
- **THEN** schema migrations are applied before storage is used

#### Scenario: MySQL storage configuration
- **WHEN** `RELAYFOLD_STORAGE` is `mysql` and non-empty `RELAYFOLD_STORE_MYSQL_HOST`, `RELAYFOLD_STORE_MYSQL_USERNAME`, and `RELAYFOLD_STORE_MYSQL_PASSWORD` settings are configured
- **THEN** the orchestrator uses the MySQL storage adapter
- **THEN** the MySQL port defaults to `3306` and the database defaults to `relayfold` when those settings are omitted
- **THEN** schema migrations are applied before storage is used

#### Scenario: Invalid durable storage configuration
- **WHEN** a required backend-specific setting is missing or empty, the MySQL port is invalid, or `RELAYFOLD_STORAGE` is unsupported
- **THEN** orchestrator startup fails with a configuration error identifying the invalid setting

#### Scenario: Legacy SQL configuration is rejected
- **WHEN** `RELAYFOLD_STORAGE` is `sql` or only `RELAYFOLD_DATABASE_URL` is configured
- **THEN** the legacy URL configuration does not select or configure a durable adapter
- **THEN** `RELAYFOLD_STORAGE=sql` fails startup as unsupported

#### Scenario: Legacy backend variable names are ignored
- **WHEN** a durable adapter is selected but only the previous `RELAYFOLD_MYSQL_*` or `RELAYFOLD_SQLITE_*` variables are configured
- **THEN** those variables do not configure the selected adapter
- **THEN** startup fails when the corresponding required `RELAYFOLD_STORE_*` variable is absent

#### Scenario: Concurrent MySQL transition
- **WHEN** concurrent MySQL transactions attempt to commit transitions from the same workflow instance version
- **THEN** no more than one transition commits from that version
- **THEN** stale transitions return a workflow version conflict without partially committing their event or snapshot data

### Requirement: Atomic Transition Batches
The orchestrator SHALL treat an ordered event batch from one workflow decision as a single transition.

#### Scenario: Multi-event verifier transition
- **WHEN** a verifier rejection creates multiple state changes such as recording feedback, marking attempts unsatisfied, and materializing the next attempts
- **THEN** those changes are committed as one ordered event batch

#### Scenario: Durable storage transaction expectation
- **WHEN** a durable storage adapter implements event-backed snapshots
- **THEN** event append, snapshot save, and summary projection update for one transition batch are performed atomically by the adapter's persistence mechanism

### Requirement: Namespace-Scoped Workflow Data
The orchestrator SHALL use namespace as part of definition and workflow-instance identity throughout registration, invocation, execution state, event persistence, task and verifier reads, listing, and storage projection updates.

#### Scenario: Definition identity is namespace-scoped
- **WHEN** two namespaces register workflow or function definitions with the same ID
- **THEN** each namespace reads, lists, invokes, overwrites, or deletes only its own definition

#### Scenario: Definition immutability is namespace-scoped
- **WHEN** one namespace has instantiated a workflow definition
- **AND** another namespace registers a definition with the same ID
- **THEN** the first namespace's instance does not prevent the second namespace from replacing its own definition

#### Scenario: Workflow state is namespace-scoped
- **WHEN** two namespaces contain workflow instances with the same ID
- **THEN** snapshot reads, task reads, event history, human input, retry, pause, resume, and commits affect only the selected namespace

#### Scenario: Definition invocation projection is namespace-scoped
- **WHEN** a workflow instance updates definition last-invoked metadata
- **THEN** storage updates only the workflow definition in the instance's namespace

#### Scenario: Namespace is absent from public definition body
- **WHEN** a caller registers or retrieves a workflow or function definition
- **THEN** the public definition payload does not gain a namespace routing field
