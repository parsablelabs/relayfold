---
title: Workflow Lifecycle
description: Understand workflow states, pause and resume behavior, retries, events, and current-status reads.
---

A workflow instance moves through explicit lifecycle states as RelayFold schedules tasks and records results.

## States

| State | Meaning |
| --- | --- |
| `Pending` | The workflow is waiting for an engine pass or runnable task work. |
| `Running` | RelayFold is actively advancing the workflow or waiting for a claimed task result. |
| `Paused` | The workflow has been stopped by an operator and will not dispatch new work until resumed. |
| `InputNeeded` | An Agent task asked for human input and the workflow is waiting for a response. |
| `Completed` | All required work completed successfully. |
| `Failed` | A task, verifier, host, validation, or retry condition failed the workflow. |

Task status uses a similar set: `Pending`, `Running`, `InputNeeded`, `Completed`, and `Failed`.

For overlap checks, workflow instances in `Pending`, `Running`, `Paused`, or
`InputNeeded` are active. `Completed` and `Failed` instances are terminal.

## Current status and events

Use the current-status endpoint for the latest workflow view:

```bash
curl -sS "$RELAYFOLD_URL/workflows/{workflow_instance_id}"
```

Use the event endpoint for execution history:

```bash
curl -sS "$RELAYFOLD_URL/workflows/{workflow_instance_id}/events"
```

The current status is the normal read path for operators and applications. Events are useful for audit trails and debugging how a workflow reached its current state.

## Pause and resume

Pause stops queued or future workflow passes from dispatching more work:

```bash
curl -sS -X POST "$RELAYFOLD_URL/workflows/{workflow_instance_id}/pause"
```

Pause does not cancel a task that is already in flight. That task may still finish. If an in-flight task completes and more downstream work remains, RelayFold records the result and leaves the workflow paused without dispatching the downstream tasks.

Resume moves a paused workflow back to the queue:

```bash
curl -sS -X POST "$RELAYFOLD_URL/workflows/{workflow_instance_id}/resume"
```

Terminal workflows cannot be resumed. A workflow waiting for human input should continue through the human-input endpoint instead of resume.

## InputNeeded

`InputNeeded` is workflow-blocking. When an Agent task asks for human input, the current engine pass stops after recording the question. Independent branches that could otherwise run remain pending until human input is submitted.

See [Human Input](/relayfold/docs/concepts/human-input/) for the operator flow.

## Retry

Retry resets a failed task attempt and queues the workflow again:

```bash
curl -sS -X POST "$RELAYFOLD_URL/workflows/{workflow_instance_id}/tasks/{task_id}/retry"
```

Default retry preserves the workflow's pinned worker host so host-local workspace and Agent session context remain available.

Force retry may reassign the workflow to another eligible host when the original host is unavailable:

```bash
curl -sS -X POST "$RELAYFOLD_URL/workflows/{workflow_instance_id}/tasks/{task_id}/retry?force=true"
```

Use force retry only when losing host-local workspace or Agent session context is acceptable.

## Queue behavior

RelayFold prevents overlapping engine passes for one workflow instance. If a workflow is queued while an engine pass is active, the queued pass waits until the active pass completes.

The scheduler uses one deployment-wide queue, but queued identity includes the
selected namespace. Identical workflow instance IDs in different namespaces are
independent. Queue status, removal, purge, pause, and resume operations affect
only the namespace selected for the request.
