---
status: proposed
---

# Compose workflows with awaited Workflow tasks

RelayFold will add a first-class Workflow task kind that creates one child
workflow instance and completes only after that instance completes
successfully. The orchestrator, rather than a worker-side Function, will own
the durable parent-child relationship and wake the parent when the child
reaches a terminal state. This permits reusable workflow composition without
occupying a worker while waiting or weakening RelayFold's recovery guarantees.

## Decision

### Definition contract

A Workflow task definition will identify:

- a workflow definition in the same namespace; and
- a result task in that workflow definition whose output becomes the Workflow
  task's output.

The referenced workflow definition and result task must exist when the parent
definition is registered. Registration will reject direct and indirect cycles
between workflow definitions. References are static definition IDs rather than
values selected at runtime.

A Workflow task will accept zero or one resolved input value. That value
becomes the child workflow instance's trigger input without additional
wrapping. A parent that needs to combine several upstream outputs must first
use another task to shape them into one value. The selected result task's
output is validated against the Workflow task's output schema before it is
made available through parent data bindings.

### Runtime lifecycle

When a Workflow task becomes runnable, the orchestrator will:

1. durably associate the parent task attempt with a unique child invocation;
2. create and enqueue the child workflow instance exactly once for that
   invocation;
3. pass the Workflow task's resolved input as the child's trigger input;
4. leave the parent task in an explicit waiting state without dispatching it
   to a worker; and
5. resume the parent workflow when the child becomes terminal.

The child workflow instance will use the parent's pinned worker host for the
initial capability. It will still have its own workflow-instance workspace and
Agent sessions. Files and other host-local state will not cross the workflow
boundary implicitly; workflows must exchange structured data or use an
external durable store.

The parent-child link, including the parent namespace, workflow instance, task
attempt, child workflow instance, and invocation identity, is durable workflow
state. Status and event APIs will expose enough of this relationship to
navigate from parent to child and diagnose why a parent is waiting.

### Completion and failure

If the child completes, the orchestrator reads the configured logical result
task's satisfied output and completes the parent Workflow task with that
value. Downstream parent tasks then proceed through ordinary data bindings.

If the child fails, the parent Workflow task fails and normal parent workflow
failure behavior applies. A child that is paused or waiting for human input is
nonterminal, so the parent continues waiting. Operators resolve or resume the
child directly.

Pausing the parent does not pause an already-created child. The child may
finish while the parent is paused; its result is reconciled when the parent is
resumed. RelayFold does not currently define workflow cancellation, so this
decision adds no cancellation-propagation behavior.

Child completion notification is at least once. Correctness will not depend on
an atomic update spanning both workflow instances: startup recovery and normal
engine reconciliation will detect a terminal child whose parent has not yet
advanced and safely apply the result.

### Retries and reruns

Replaying orchestration after a crash must reuse the child already associated
with the invocation and must not create a duplicate. An explicit retry of a
failed Workflow task creates a new child invocation while retaining the prior
child in history. A verifier rerun creates a new Workflow task attempt and
therefore a new child invocation. Invocation identity must distinguish these
operator-requested executions from recovery of the same execution.

## Considered options

- **Start and poll from a Function task.** Rejected because polling occupies a
  worker, is governed by task timeouts, and can create duplicate children when
  task execution is retried at least once.
- **Start a workflow without awaiting it.** Rejected as the composition
  primitive because downstream tasks could not safely consume the child's
  result or depend on its success. Fire-and-forget triggering may be considered
  separately if a use case requires it.
- **Inline the referenced workflow's tasks into the parent definition.**
  Rejected because it erases the child run as an observable and independently
  recoverable unit, complicates identity and verifier generations, and couples
  the parent to the child's internal graph.
- **Share the parent's workspace with the child.** Rejected because workspaces
  and Agent sessions are scoped to a workflow instance. Implicit sharing would
  make placement, retry, and recovery behavior depend on hidden local state.
- **Allow cross-namespace invocation.** Rejected for the initial capability
  because it introduces authorization and ownership rules that are unnecessary
  for composing definitions managed together.

## Consequences

Workflow authors can build larger static graphs from smaller reusable
workflows, including parallel fan-out through multiple Workflow tasks. Each
child remains independently observable and can pause for human input or use
its own verifier loops while the parent waits durably.

This capability does not add dynamic task generation. A Workflow task creates
exactly one child per invocation, and the number and arrangement of Workflow
tasks remain part of the registered parent definition. Runtime-discovered,
unbounded fan-out requires a separate decision.

The orchestrator gains responsibility for cross-workflow wake-up,
reconciliation, cycle validation, invocation idempotency, and result
projection. Workflow status gains a waiting condition distinct from worker
execution. Public task-kind, status, event, and workflow-definition contracts
must be updated together when this proposal is implemented.

This capability can complement the scheduled PRD delivery design in ADR-0002,
but it does not replace that design: the number of PRD children is discovered
at runtime, while Workflow tasks remain statically declared.
