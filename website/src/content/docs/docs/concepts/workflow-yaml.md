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
| `sandbox` | No | Run each task attempt in a fresh Gondolin sandbox. Omit it for normal host execution. |
| `tasks` | Yes | Task definitions that make up the workflow graph. |
| `data_bindings` | Yes | Edges that pass outputs from source tasks to target task inputs. |

## Optional sandbox

```yaml
sandbox:
  network:
    allowed_hosts:
      - api.github.com
      - registry.npmjs.org
      - "*.example.com"
```

The presence of `sandbox` enables Gondolin for Agent tools, Function execution
and dependency installation, and API Calls. `sandbox: {}` blocks outbound task
network access. `network.allowed_hosts` permits HTTP and HTTPS to matching
hostnames; use hostnames rather than URLs. `*` matches any substring, and a lone
`"*"` permits any public hostname. Private and loopback addresses remain blocked.
Arbitrary TCP, SSH, and WebSocket access are not enabled. Redirects follow the
same policy. Omitting `sandbox` preserves normal execution.

The workflow policy is the default for every task. A task can define its own
`sandbox.network.allowed_hosts`, which **replaces** the default rather than
adding to it. Task-level `sandbox: {}` denies all outbound task network access.
Task overrides require a workflow-level `sandbox`; otherwise the workflow is
rejected. Tasks cannot disable workflow sandboxing.

```yaml
sandbox: {} # Sandbox all tasks; deny network by default
tasks:
  - id: research
    sandbox:
      network:
        allowed_hosts: [docs.example.com]
    # Add kind and other task fields here.
  - id: analyze
    # Inherits deny-all; add kind and other task fields here.
  - id: publish
    sandbox:
      network:
        allowed_hosts: [api.example.com]
    required_credentials: [publishing_token]
    # Add kind and other task fields here.
```

Each task attempt receives a new VM containing only that task's
`required_credentials`, exposed as uppercased environment variables. Function
tasks also receive those credentials through their existing context. There is
no workflow-level credential declaration or backend selector. The VM is closed
on completion, failure, human-input pause, or timeout; startup failures never
fall back to host execution.

The selected workspace is mounted at `/workspace` and retains its existing
sharing and retry behavior. Other guest files and processes are disposable.
Secrets that a task explicitly writes into the workspace become shared files.
Approved skill directories are mounted read-only inside the guest.

Sandboxed Agents support the built-in `read`, `write`, `edit`, `bash`,
`http_request`, `fetch_url`, `web_search`, `current_time`, and `ask_user` tools.
Host Pi extension tools are not loaded; explicitly requesting an unsupported
tool fails the task. `web_search` requires `system_brave_api_key` in the task's
`required_credentials` and access to `api.search.brave.com`. `fetch_url` needs
access to its reader service, `r.jina.ai`.

Agent model requests, session storage, and orchestration remain on the trusted
worker host. The network policy governs task I/O rather than the model connection.
See [Task Credentials](/relayfold/docs/operations/credentials/) and
[Production Deployment](/relayfold/docs/operations/production-deployment/).

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
| `sandbox` | No | Replace the workflow's default network policy for this task. Requires workflow-level sandbox activation. |
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
    url: "https://api.example.com/items"
    method: "GET"
    headers:
      Accept: "application/json"
      X-Client-Version: "1"
```

`headers` is an optional string-to-string map of literal request headers. A successful
API call returns `{ status, headers, body }`. JSON media types produce a parsed JSON
`body`; other response bodies are strings. When present, `output_schema` validates
this complete response value before it can flow to downstream tasks.

## Data bindings

Data bindings pass one task's output into another task's input array:

```yaml
data_bindings:
  - source_task_id: fetch-data
    target_task_id: summarize
```

If a task has multiple upstream bindings, it receives multiple input values. The target task should declare `input_schemas` when it depends on specific input shapes.

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
