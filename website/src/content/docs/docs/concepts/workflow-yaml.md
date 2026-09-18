---
title: Workflow YAML Reference
description: Reference the workflow definition fields used by RelayFold.
---

RelayFold workflow definitions are JSON or YAML documents. The API accepts either
format directly and stores the definition canonically as JSON.

## Top-level fields

```yaml
id: example-workflow
description: Summarize an input document.

tasks: []

data_bindings: []
```

| Field | Required | Description |
| --- | --- | --- |
| `id` | Yes | Workflow definition ID. IDs are normalized during registration. |
| `description` | No | Human-readable workflow description used in workflow discovery lists. Defaults to an empty string. |
| `tasks` | Yes | Task definitions that make up the workflow graph. |
| `data_bindings` | Yes | Edges that pass outputs from source tasks to target task inputs. |

## Task fields

```yaml
tasks:
  - id: summarize
    timeout_secs: 300
    kind:
      function:
        dependencies: []
        code: |
          export default async function run({ inputs }) {
            return { response: "ok" };
          }
    input_schemas: []
    output_schema:
      type: object
    workspace:
      group_name: repo
    required_credentials: []
```

| Field | Required | Description |
| --- | --- | --- |
| `id` | Yes | Logical task ID used by bindings, retries, and results. |
| `kind` | Yes | One of `agent`, `function`, or `apiCall`. |
| `timeout_secs` | No | Task execution timeout. |
| `input_schemas` | No | JSON Schemas for expected input slots. |
| `output_schema` | No | JSON Schema for task output. Verifier tasks must omit this because RelayFold injects the decision schema. |
| `workspace` | No | Workspace group assignment. |
| `required_credentials` | Yes | Named credentials required before execution. Use `[]` when none are needed. |
| `control` | No | Verifier control settings for bounded loops. |

Input and output schemas support standard format validation, including `date`,
`email`, and `uri`.

## Task kinds

Agent task:

```yaml
kind:
  agent:
    model_id: "google/gemini-2.5-flash"
    provider_url: ""
    prompt: Summarize the input.
    tools: []
    skills: []
    ask: false
    schema_failure_retry_times: 2
    reuse_session: true
```

Function task:

```yaml
kind:
  function:
    dependencies:
      - name: lodash-es
        version: 4.17.21
    code: |
      export default async function run({ inputs }) {
        return { response: inputs.length };
      }
```

Registered Function reference:

```yaml
kind:
  function:
    ref: mailgun.fetch_inbound_mail
```

API Call task:

```yaml
kind:
  apiCall:
    url: "https://api.example.com/items/${inputs[0].item_id}"
    method: "GET"
    headers:
      Accept: "application/json"
      Authorization: "Bearer ${credentials.api_token}"
```

`url` supports URL-encoded scalar input interpolation with
`${inputs[index].path}`. `headers` is an optional string-to-string map whose
values support `${credentials.name}` interpolation. A successful API call
returns `{ status, headers, body }`. JSON media types produce a parsed JSON
`body`; other response bodies are strings. When present, `output_schema`
validates this complete response value before it can flow to downstream tasks.

## Data bindings

Data bindings pass one task's output into another task's input array:

```yaml
data_bindings:
  - source_task_id: fetch-data
    target_task_id: summarize
```

If a task has multiple upstream bindings, it receives multiple input values in
the order those bindings appear in `data_bindings`. The target task should
declare `input_schemas` when it depends on specific input shapes.

## Verifier control

```yaml
control:
  verifier:
    max_iterations: 3
    on_exhausted_continue: false
    rerun_from_task_id: draft-report
```

`control.verifier` turns a task into a bounded-loop verifier. The verifier returns `{ "decision": "complete" }` or `{ "decision": "continue", "feedback": "..." }`.

See [Bounded Loops](/relayfold/docs/concepts/bounded-loops/) for the full control-flow behavior.

## Workspaces

```yaml
workspace:
  group_name: repo
```

Tasks with the same `workspace.group_name` share the same workflow-instance workspace. Workspace sharing does not create scheduling dependencies; use `data_bindings` to order tasks.

## Credentials

```yaml
required_credentials:
  - gemini_api_key
  - gh_token
```

Credentials are resolved by the worker before task execution. Missing credentials fail the task before its main work runs.
