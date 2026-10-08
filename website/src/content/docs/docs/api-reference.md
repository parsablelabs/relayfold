---
title: API Reference
description: HTTP endpoints for registering workflows, starting runs, inspecting state, controlling execution, and worker coordination.
---

RelayFold exposes a public orchestrator API for users and operators, plus a worker API used by RelayFold workers. Local Docker installs expose the public API on `http://localhost:3000`. The worker API listens separately on `http://localhost:3001` by default and is intended for worker runtime integration.

Examples use:

```bash
export RELAYFOLD_URL=http://localhost:3000
```

For the currently supported single-tenant request path, configure
`RELAYFOLD_USE_GLOBAL_NAMESPACE=true`. RelayFold then selects the exact built-in
namespace `global-namespace` and ignores any `Authorization` header. When the
variable is unset or `false`, missing or malformed bearer credentials return
`401 Unauthorized`. A well-formed `Authorization: Bearer <api-key>` credential
reaches the deferred key-to-namespace resolver, which deliberately panics as not
implemented in this release; it does not select a fallback namespace. Values
other than `true` or `false` are invalid. `/health` does not require a namespace.

The selected namespace scopes every public definition, workflow, task, event, and
queue operation. Resource IDs therefore identify a resource only within that
namespace. Public definition and workflow-list bodies do not include a
`namespace` field, and an ID that exists only in another namespace is reported
the same way as an unknown ID.

## Public endpoints

| Method | Path | Purpose |
| --- | --- | --- |
| `GET` | `/health` | Check API health. |
| `GET` | `/workflow-def` | List registered workflow definition summaries. |
| `POST` | `/workflow-def` | Register a workflow definition. |
| `GET` | `/workflow-def/{def_id}` | Get a complete registered workflow definition. |
| `POST` | `/workflow-def/{def_id}` | Create and queue a workflow instance. |
| `POST` | `/workflow-def/{def_id}/tasks/{task_id}` | Execute one workflow task in isolation. |
| `GET` | `/function-def` | List registered function summaries in the selected namespace. |
| `POST` | `/function-def` | Register a reusable function definition. |
| `GET` | `/function-def/{def_id}` | Get a complete registered function definition. |
| `DELETE` | `/function-def/{def_id}` | Delete a reusable function definition. |
| `GET` | `/workflows` | List workflow instances. |
| `GET` | `/workflows/{id}` | Get workflow status. |
| `GET` | `/workflows/{id}/events` | Get a cursor-paginated workflow event-history page. |
| `POST` | `/workflows/{id}/pause` | Pause one workflow instance. |
| `POST` | `/workflows/{id}/resume` | Resume one paused workflow instance. |
| `POST` | `/workflows/pause` | Pause active workflow instances. |
| `POST` | `/workflows/resume` | Resume paused workflow instances. |
| `GET` | `/workflows/{workflow_instance_id}/tasks` | List materialized task results. |
| `GET` | `/workflows/{workflow_instance_id}/tasks/{task_id}` | Get the latest result for one logical task. |
| `GET` | `/workflows/{workflow_instance_id}/tasks/{task_id}/{generation}` | Get a specific task generation result. |
| `POST` | `/workflows/{workflow_instance_id}/tasks/{task_id}/human-input` | Submit human input for an `InputNeeded` task. |
| `POST` | `/workflows/{workflow_instance_id}/tasks/{task_id}/retry` | Retry a failed task. |
| `GET` | `/workflow-queue` | Inspect queued workflow instance IDs. |
| `DELETE` | `/workflow-queue/{id}` | Remove one queued workflow instance. |
| `DELETE` | `/workflow-queue` | Purge queued workflow instances. |

## Health

```bash
curl -sS "$RELAYFOLD_URL/health"
```

Response:

```text
OK
```

## Workflow definitions

Register a workflow definition:

```bash
curl -sS -X POST "$RELAYFOLD_URL/workflow-def" \
  -d '{
    "id": "hello-workflow",
    "description": "Says hello to a named user.",
    "tasks": [
      {
        "id": "hello",
        "kind": {
          "function": {
            "dependencies": [],
            "code": "export default async function run({ inputs }) { return { response: `hello ${inputs[0].name}` }; }"
          }
        },
        "input_schemas": [],
        "output_schema": {
          "type": "object",
          "required": ["response"],
          "properties": {
            "response": { "type": "string" }
          }
        },
        "required_credentials": []
      }
    ],
    "data_bindings": []
  }'
```

Response:

```json
{
  "status": "created",
  "id": "hello-workflow"
}
```

`description` is optional and defaults to an empty string.

Workflow definitions may also be posted as YAML without conversion:

```bash
curl -sS -X POST "$RELAYFOLD_URL/workflow-def" \
  --data-binary @workflow.yaml
```

RelayFold auto-detects JSON or YAML from the request body. A `Content-Type`
containing `yaml` is treated as a parsing hint, but it is not required. An
invalid document returns `400 Bad Request`.

List registered workflow definitions without loading their task definitions or data bindings:

```bash
curl -sS "$RELAYFOLD_URL/workflow-def"
```

Response:

```json
{
  "workflow_defs": [
    {
      "id": "hello-workflow",
      "description": "Says hello to a named user.",
      "created_at_epoch_ms": 1780000000000,
      "last_invoked_at_epoch_ms": 1780000001200
    },
    {
      "id": "not-yet-run",
      "description": "A workflow that has not been invoked.",
      "created_at_epoch_ms": 1779999999000,
      "last_invoked_at_epoch_ms": null
    }
  ]
}
```

`last_invoked_at_epoch_ms` is the creation time of the most recently created workflow instance for that definition. It is `null` until the definition has been invoked.

Get one complete registered workflow definition when you need its tasks, schemas,
or data bindings:

```bash
curl -sS "$RELAYFOLD_URL/workflow-def/hello-workflow"
```

Response:

```json
{
  "id": "hello-workflow",
  "description": "Says hello to a named user.",
  "tasks": [
    {
      "id": "hello",
      "kind": {
        "function": {
          "dependencies": [],
          "code": "export default async function run({ inputs }) { return { response: `hello ${inputs[0].name}` }; }"
        }
      },
      "timeout_secs": null,
      "output_schema": {
        "type": "object",
        "required": ["response"],
        "properties": {
          "response": { "type": "string" }
        }
      },
      "required_credentials": []
    }
  ],
  "data_bindings": []
}
```

The response is the full stored workflow definition model, rather than the
summary returned by `GET /workflow-def`. An unknown `def_id` returns `404 Not
Found`.

JSON is the default response format. Request YAML with the `format` query
parameter:

```bash
curl -sS "$RELAYFOLD_URL/workflow-def/hello-workflow?format=yaml"
```

The supported values are `json` and `yaml`. YAML responses use
`Content-Type: application/yaml`; an unsupported value returns `400 Bad
Request`.

You can overwrite a registered definition while it has no workflow instances.
After any instance has been created, regardless of its state, the definition is
immutable and an overwrite returns `409 Conflict`:

```json
{
  "error": "workflow definition hello-workflow already has workflow instances and cannot be overwritten; register a new ID, for example hello-workflow_v2"
}
```

RelayFold does not enforce a versioning scheme. Suffixes such as `_v2` are a
suggested convention for choosing a new definition ID.

## Function definitions

List registered functions with `GET /function-def`:

```bash
curl -sS "$RELAYFOLD_URL/function-def"
```

The response is `{ "function_defs": [...] }`, ordered by function ID ascending.
Each entry contains only `id`; code and dependencies are fetched separately.
Function descriptions are not currently supported. An empty registry returns
`{ "function_defs": [] }`. Only functions in the selected namespace are listed.

Fetch a complete function definition with `GET /function-def/{def_id}`:

```bash
curl -sS "$RELAYFOLD_URL/function-def/format.hello"
```

The response contains `id`, `dependencies`, and `code`. Unknown IDs, including
IDs that exist only in another namespace, return `404 Not Found`.

Register a reusable function definition:

```bash
curl -sS -X POST "$RELAYFOLD_URL/function-def" \
  -d '{
    "id": "format.hello",
    "dependencies": [],
    "code": "export default async function run({ inputs }) { return { response: `hello ${inputs[0].name}` }; }"
  }'
```

Response:

```json
{
  "status": "created",
  "id": "format.hello"
}
```

Function definitions use the same JSON/YAML auto-detection as workflow
definitions. For example, register a YAML artifact directly with:

```bash
curl -sS -X POST "$RELAYFOLD_URL/function-def" \
  --data-binary @function.yaml
```

Delete a reusable function definition:

```bash
curl -i -X DELETE "$RELAYFOLD_URL/function-def/format.hello"
```

Successful deletion returns `204 No Content`. Missing definitions return `404 Not Found`.

## Start a workflow

Create and queue a workflow instance from a registered workflow definition:

```bash
curl -sS -X POST "$RELAYFOLD_URL/workflow-def/hello-workflow" \
  -H 'content-type: application/json' \
  -d '{ "name": "Ada" }'
```

The request body is stored as the trigger input. `null` means no initial input.

Response:

```json
{
  "status": "queued",
  "id": "hello-workflow-1780000000000000000",
  "pinned_host_id": "local-dev-host"
}
```

If no eligible worker host is registered, the API returns `503 Service Unavailable`.

## Execute one task

Run a task from a registered workflow definition without creating a workflow instance:

```bash
curl -sS -X POST "$RELAYFOLD_URL/workflow-def/hello-workflow/tasks/hello" \
  -H 'content-type: application/json' \
  -d '{ "inputs": [{ "name": "Ada" }] }'
```

This execution mode returns the worker result directly and does not persist a
workflow instance, task attempt, workflow event, queue entry, or result in the
orchestrator storage. It also does not update the definition's
`last_invoked_at_epoch_ms`. An `input_needed` response cannot be resumed through
the workflow APIs; invoking the endpoint again starts a new isolated execution.

The persistence boundary applies to orchestrator workflow state. The task can
still create external side effects, and worker-local workspace or Agent session
files can remain according to their normal storage lifecycles.

Success response:

```json
{
  "status": "success",
  "output": {
    "response": "hello Ada"
  }
}
```

Other result statuses are:

```json
{
  "status": "input_needed",
  "description": "Which release channel should I use?"
}
```

```json
{
  "status": "failure",
  "error_message": "Missing required credential: gh_token"
}
```

## List workflows

```bash
curl -sS "$RELAYFOLD_URL/workflows?status=completed&limit=20"
```

Query parameters:

- `status`: `pending`, `running`, `paused`, `input_needed`, `completed`, or `failed`.
- `limit`: maximum number of workflows to return.
- `cursor`: pagination cursor returned by the previous response.

Workflow cursors are opaque and namespace-scoped. Reuse a cursor only with the
same namespace context that produced it.

Response:

```json
{
  "workflows": [
    {
      "id": "hello-workflow-1780000000000000000",
      "workflow_def_id": "hello-workflow",
      "created_at_epoch_ms": 1780000000000,
      "modified_at_epoch_ms": 1780000001200,
      "completed_at_epoch_ms": 1780000001200,
      "status": "Completed",
      "total_task_count": 1,
      "completed_task_count": 1
    }
  ],
  "next_cursor": null
}
```

## Workflow status

```bash
curl -sS "$RELAYFOLD_URL/workflows/hello-workflow-1780000000000000000"
```

Response:

```json
{
  "instance_id": "hello-workflow-1780000000000000000",
  "workflow_def_id": "hello-workflow",
  "status": "Completed",
  "tasks": [
    {
      "task_attempt_id": "hello[1]",
      "task_def_id": "hello",
      "status": "Completed",
      "satisfaction": "Satisfied",
      "generation_index": 1
    }
  ],
  "verifier_states": []
}
```

## Workflow events

```bash
curl -sS "$RELAYFOLD_URL/workflows/hello-workflow-1780000000000000000/events?limit=100"
```

Response shape:

```json
{
  "workflow_instance_id": "hello-workflow-1780000000000000000",
  "events": [],
  "next_sequence": null
}
```

Events are returned in ascending sequence order. When `next_sequence` is not
`null`, pass it as `after_sequence` to request the next page:

```bash
curl -sS "$RELAYFOLD_URL/workflows/hello-workflow-1780000000000000000/events?limit=100&after_sequence=100"
```

The default page size is 100 and the maximum is 500. Exact event payloads depend
on the workflow operations that have occurred.

## Pause and resume

Pause one workflow:

```bash
curl -sS -X POST "$RELAYFOLD_URL/workflows/hello-workflow-1780000000000000000/pause"
```

Response:

```json
{
  "status": "paused",
  "workflow_instance_id": "hello-workflow-1780000000000000000"
}
```

Resume one workflow:

```bash
curl -sS -X POST "$RELAYFOLD_URL/workflows/hello-workflow-1780000000000000000/resume"
```

Response:

```json
{
  "status": "queued",
  "workflow_instance_id": "hello-workflow-1780000000000000000"
}
```

Pause all active workflows:

```bash
curl -sS -X POST "$RELAYFOLD_URL/workflows/pause"
```

Resume all paused workflows:

```bash
curl -sS -X POST "$RELAYFOLD_URL/workflows/resume"
```

Bulk responses include the operation status, affected count, and workflow instance IDs:

```json
{
  "status": "paused",
  "count": 2,
  "workflow_instance_ids": [
    "workflow-a-1780000000000000000",
    "workflow-b-1780000000000000000"
  ]
}
```

## Task results

List task results for a workflow instance:

```bash
curl -sS "$RELAYFOLD_URL/workflows/hello-workflow-1780000000000000000/tasks"
```

Response:

```json
{
  "workflow_instance_id": "hello-workflow-1780000000000000000",
  "tasks": [
    {
      "task_attempt_id": "hello[1]",
      "result": {
        "status": "success",
        "input": [
          {
            "name": "Ada"
          }
        ],
        "output": {
          "response": "hello Ada"
        },
        "task_def_id": "hello",
        "task_attempt_id": "hello[1]",
        "satisfaction": "Satisfied",
        "generation_index": 1
      }
    }
  ]
}
```

Get the latest result for one logical task:

```bash
curl -sS "$RELAYFOLD_URL/workflows/hello-workflow-1780000000000000000/tasks/hello"
```

Get a specific generation:

```bash
curl -sS "$RELAYFOLD_URL/workflows/hello-workflow-1780000000000000000/tasks/hello/1"
```

Task status reports and task-result metadata include `early_exit`. It is `true`
on the completed attempt that triggered early exit and `false` on other attempts,
including those skipped as a result. The saved task output contains
`_workflow_exit_reason`, which explains why it exited.

The state-changing `task_early_exit_set` event records the flag:

```json
{
  "type": "task_early_exit_set",
  "task_attempt_id": "prune-covered-logs[1]",
  "early_exit": true
}
```

Workflow status remains `Completed` on successful completion; no separate workflow
exit flag is stored. New attempts default to `early_exit: false`, and retries clear
it on the retried attempt.

The triggering task output is retained. Task status reports use `Skipped` for
unexecuted attempts; workflow list `completed_task_count` counts only executed,
completed attempts.

Task result statuses include `success`, `failure`, `pending`, `running`, `skipped`, and `input_needed`. A `skipped` result means the attempt was not executed after a successful early-exit request; it has no output and does not count as a successful execution. Success results include `output`; failure results include `error_message`; input-needed results include `input_request`.

## Human input

Submit input for an Agent task currently waiting in `InputNeeded`:

See [Human Input](/relayfold/docs/concepts/human-input/) for the full workflow behavior and Agent configuration.

```bash
curl -sS -X POST "$RELAYFOLD_URL/workflows/human-workflow-1780000000000000000/tasks/collect-release-preference/human-input" \
  -H 'content-type: application/json' \
  -d '{ "input": "Use the stable release channel." }'
```

Response:

```json
{
  "status": "queued",
  "workflow_instance_id": "human-workflow-1780000000000000000",
  "task_attempt_id": "collect-release-preference[2]"
}
```

The API returns `409 Conflict` if the task is not waiting for input or is not an Agent task.

## Retry tasks

Retry a failed task on the workflow's existing pinned host:

```bash
curl -sS -X POST "$RELAYFOLD_URL/workflows/hello-workflow-1780000000000000000/tasks/hello/retry"
```

Response:

```json
{
  "status": "queued",
  "workflow_instance_id": "hello-workflow-1780000000000000000",
  "task_attempt_id": "hello[1]",
  "pinned_host_id": "local-dev-host",
  "local_context_may_be_lost": false
}
```

Force retry can reassign the workflow to another eligible host when the original host is unavailable:

```bash
curl -sS -X POST "$RELAYFOLD_URL/workflows/hello-workflow-1780000000000000000/tasks/hello/retry?force=true"
```

If no retry host is available, the API returns `503 Service Unavailable`.

## Workflow queue

Inspect pending workflow instance IDs:

```bash
curl -sS "$RELAYFOLD_URL/workflow-queue"
```

Response:

```json
{
  "pending": [
    "hello-workflow-1780000000000000000"
  ]
}
```

Remove one queued workflow instance:

```bash
curl -i -X DELETE "$RELAYFOLD_URL/workflow-queue/hello-workflow-1780000000000000000"
```

Successful removal returns `204 No Content`.

Purge the queue:

```bash
curl -sS -X DELETE "$RELAYFOLD_URL/workflow-queue"
```

Response:

```json
{
  "status": "purged",
  "purged": [
    "hello-workflow-1780000000000000000"
  ]
}
```

## Worker endpoints

Worker endpoints are for RelayFold worker processes. Application clients normally use the public endpoints above.

Use a separate worker API base URL:

```bash
export RELAYFOLD_WORKER_URL=http://localhost:3001
```

### Register worker

```bash
curl -sS -X POST "$RELAYFOLD_WORKER_URL/workers/register" \
  -H 'content-type: application/json' \
  -d '{
    "worker_id": "worker-1",
    "host_id": "local-dev-host"
  }'
```

Response:

```json
{
  "type": "registration_ack",
  "worker_id": "worker-1",
  "heartbeat_interval_ms": 5000
}
```

### Heartbeat worker

```bash
curl -sS -X POST "$RELAYFOLD_WORKER_URL/workers/heartbeat" \
  -H 'content-type: application/json' \
  -d '{
    "worker_id": "worker-1",
    "host_id": "local-dev-host"
  }'
```

Response:

```json
{
  "status": "accepted",
  "worker_id": "worker-1"
}
```

### Claim task

```bash
curl -sS -X POST "$RELAYFOLD_WORKER_URL/workers/tasks/claim" \
  -H 'content-type: application/json' \
  -d '{ "worker_id": "worker-1" }'
```

Task response:

```json
{
  "type": "task_dispatch",
  "namespace": "550e8400-e29b-41d4-a716-446655440000",
  "workflow_inst_id": "hello-workflow-1780000000000000000",
  "task_id": "worker-pool-0",
  "task": {
    "id": "hello",
    "kind": {
      "function": {
        "dependencies": [],
        "code": "export default async function run() { return { response: \"ok\" }; }"
      }
    },
    "required_credentials": []
  },
  "workspace_path_suffix": "550e8400-e29b-41d4-a716-446655440000/hello-workflow-1780000000000000000/taskid-hello",
  "inputs": [
    {
      "name": "Ada"
    }
  ],
  "execution_metadata": {}
}
```

No-task response:

```json
{
  "type": "no_task"
}
```

### Complete task

```bash
curl -sS -X POST "$RELAYFOLD_WORKER_URL/workers/tasks/worker-pool-0/result" \
  -H 'content-type: application/json' \
  -d '{
    "kind": "success",
    "output": {
      "response": "hello Ada"
    }
  }'
```

Other completion payloads:

```json
{
  "kind": "input_needed",
  "description": "Which release channel should I use?"
}
```

```json
{
  "kind": "failure",
  "reason": "Missing required credential: gh_token"
}
```

The dispatch ID in the request path identifies the active lease, so completion
payloads do not repeat the namespace from the claimed task.

Response:

```json
{
  "status": "accepted"
}
```
