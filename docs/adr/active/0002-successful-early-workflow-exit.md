---
status: accepted
---

# Successful early workflow exit through a task output boolean

Workflows can discover that no remaining work is needed while still executing downstream tasks, adding latency and Agent cost. For [issue #91](https://github.com/parsablelabs/relayfold/issues/91), we chose an explicit task control that completes the workflow successfully when a selected output boolean is true. This is an accepted design decision; the capability is not yet implemented.

## Decision

Any task kind can declare the following control:

```yaml
control:
  exit_workflow:
    when: /no_work
```

`when` is a [JSON Pointer (RFC 6901)](https://www.rfc-editor.org/rfc/rfc6901) evaluated against the triggering task's successful, validated output. The selected value must be a boolean: `true` requests successful early completion, and `false` preserves normal execution. A missing value or a non-boolean value is a control-validation error; values are not coerced to booleans. Failed execution or invalid output cannot trigger successful early completion.

The triggering task attempt is recorded as `Completed`, and its output is preserved. All remaining pending task attempts become `Skipped`, including pending tasks on independent branches and downstream tasks that perform side effects. Previously completed task outputs are retained. `Skipped` represents work that was not executed and must not be counted as successfully executed or satisfy downstream data bindings. Workflows without this control retain their existing behavior.

The field name is chosen by the workflow author rather than reserved by RelayFold. For example, `/already_processed`, `/result/should_terminate`, or an API Call's `/body/no_work` can express the relevant decision. Task implementations compute the boolean; the orchestrator interprets the control consistently across task kinds. The configuration alone opts into early exit, so an output boolean without this control does not terminate the workflow.

## Concurrent tasks and completion

When the orchestrator detects an early-exit request, it marks only pending task attempts as `Skipped` and stops dispatching new tasks. Already-running task attempts continue executing and retain their actual execution statuses and outputs; they are not cancelled or marked `Skipped`.

The workflow waits for those running tasks to finish. For successful workflow completion, `Skipped` counts as terminal alongside `Completed`, without implying successful execution or satisfaction of data bindings. Once all remaining running tasks complete successfully and every task attempt relevant to completion is `Completed` or `Skipped`, the workflow becomes `Completed`. If no tasks remain running, completion can occur immediately when the exit is recorded.

The persisted skipped task states must keep the early exit effective while running tasks finish: their results must not cause downstream tasks to be dispatched after exit has been requested. If an already-running task fails, normal failure behavior applies and the workflow becomes `Failed`; an early-exit request does not override that failure. Human-input and verifier-result interactions during this interval still require the decisions listed below.

## Why JSON Pointer

JSON Pointer selects one specific value using an established syntax for nested objects and array elements. We chose it over JSONPath because this control does not need queries, wildcards, filters, or multiple matches. We also chose a configurable pointer over a mandatory output field name to avoid imposing a reserved field on every task's output contract.

`when` contains the pointer directly. The longer `when: { output_pointer: /no_work }` form adds nesting without changing the semantics.

## Exit observability without a configured reason

The control has no `reason` field. A required static explanation would repeat information already conveyed by the task and selected output field.

Persist the triggering task attempt ID and the configured JSON Pointer in an `early_completion_requested` workflow event, and expose them through workflow event inspection. Do not persist a separate early-exit field on the workflow snapshot or a dedicated database column. Scheduling and completion use task states: pending attempts become `Skipped`, and the workflow completes when every relevant attempt is `Completed` or `Skipped`. Skipped verifier slices must not create further generations or require acceptance. Together with preserved task outputs, these identify why the workflow ended early. Inspection can explain, for example, that `prune-patterns[1]` returned `true` at `/no_work`. Workflow authors can include a dynamic explanation in the ordinary task output when needed; the control does not require a separate explanation contract.

## Outstanding decisions

- How a task still running after an early-exit request affects the workflow if it requests human input.
- Whether early exit is allowed on verifier tasks or inside verifier retry slices, and how it interacts with acceptance, satisfaction, and future generations. Restricting it in these locations was suggested but has not been agreed.
- The ordering of early completion and an operator pause that arrives while the triggering task is running.

These questions remain open and must be settled before implementing the affected behavior.
