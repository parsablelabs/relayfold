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
example_input:
  document: "A short document to summarize."

tasks: []

data_bindings: []
```

| Field | Required | Description |
| --- | --- | --- |
| `id` | Yes | Workflow definition ID. IDs are normalized during registration. |
| `description` | No | Human-readable workflow description used in workflow discovery lists. Defaults to an empty string. |
| `example_input` | No | JSON-compatible example payload used to prefill the UI JSON trigger input. |
| `sandbox` | No | Run each task attempt in a fresh Gondolin sandbox. Omit it for normal host execution. |
| `tasks` | Yes | Task definitions that make up the workflow graph. |
| `data_bindings` | Yes | Edges that pass outputs from source tasks to target task inputs. |

The UI prefills **JSON trigger input** with `example_input` as formatted JSON when
you open a workflow. Edit or clear it before choosing **Start workflow**. The
input must still satisfy the entry task input schemas. Omit `example_input` or
set it to `null` to leave the input empty. Objects, arrays, strings, numbers, and
booleans are supported; write the payload as a YAML value, not a JSON string.
This field is an example for the UI, not a default for API or scheduled runs.

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
| `allow_agent_feedback` | No | Request optional Agent feedback about environment and setup friction. Defaults to `false`. |
| `control` | No | Early-exit permission (`allow_early_exit`, default `false`) and verifier settings for bounded loops. |

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

Data bindings pass a source task's output into each target task's input array.
Set `source_task_id` to the source task ID and `target_task_id` to a non-empty
list of target task IDs:

```yaml
data_bindings:
  - source_task_id: fetch-data
    target_task_id: [summarize, archive]
  - source_task_id: summarize
    target_task_id: [publish]
```

Each target receives the same source output and waits for the source task to
satisfy its dependency, including any verifier-loop requirements. Every target
must reference a task defined in the workflow and follow the task ID rules.
Empty lists and non-string items are rejected.

The CloudWatch scanner, GitHub issue-to-PR, and Mailgun dispatcher examples use
target lists wherever one task supplies output to multiple downstream tasks.

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

## Agent feedback

Enable optional feedback about observed environment and setup friction:

```yaml
allow_agent_feedback: true
output_schema:
  type: object
  properties:
    result:
      type: string
  required: [result]
  additionalProperties: false
```

RelayFold adds the optional reserved `_agent_provided_feedback` field to the
effective output schema, including strict schemas. Enabled Agents receive
instructions to report concise observations, any workaround used, and concrete
improvements. They do not investigate solely to generate feedback.

```json
{
  "result": "Implemented the change; tests pass.",
  "_agent_provided_feedback": "Installed ripgrep to inspect the repository. Include it in the worker image to avoid repeated setup."
}
```

Omit the field or return `null` when there is nothing useful to report. Feedback
strings must be at most 255 characters. Longer strings, empty or whitespace-only
strings, and other value types fail output validation. Feedback
has no independent effect on success, retries, satisfaction, or workflow control.
It can coexist with `_workflow_exit_reason` when early exit is enabled separately.

The flag defaults to `false`. Disabled tasks receive no feedback instructions or
schema extension. Unsolicited feedback is ordinary output governed by the task's
schema; RelayFold does not remove it. Do not declare this reserved field in the
configured schema's top-level `properties` or `required`, even when disabled.

As with early exit, enabled tasks require a plain `type: object` output schema
without top-level `$ref`, `allOf`, `anyOf`, or `oneOf`, or no output schema. The
configured schema remains unchanged. Without a user schema, Agents return JSON
and can put their normal answer in the optional `response` string. The flag is
accepted for every task kind, but only Agents receive feedback instructions.

Feedback stays in the originating attempt's output and is available through
[task-result inspection](/relayfold/docs/api-reference/). It requires a valid
final result, so crashes, timeouts, and exhausted execution may produce none.
RelayFold collects these observations without automatically changing the
environment or aggregating feedback across runs.

## Early workflow exit

Enable successful early completion on a task with `control.allow_early_exit`:

```yaml
control:
  allow_early_exit: true
output_schema:
  type: object
  properties:
    result:
      type: string
  additionalProperties: false
```

The flag defaults to `false` and applies to every task kind. RelayFold adds the
optional top-level `_workflow_exit_reason` field to the effective output schema,
including strict schemas, and tells enabled Agents how to use it. Do not declare
this reserved field in `output_schema.properties` or `required`; configuration
validation rejects those declarations.

For Agent tasks, define the conditions for early exit explicitly in the task
prompt. The flag grants permission and supplies the output protocol; it does not
define when to exit. Agents are instructed to continue normal execution if the
task prompt provides no early-exit conditions.

The task requests exit by returning:

```json
{
  "_workflow_exit_reason": "All matching patterns are already covered."
}
```

A missing or `null` reason continues execution. An empty or whitespace-only string,
or another value type, fails output validation. A disabled task's returned field
has no exit effect; its ordinary output schema still applies. The unprefixed
`workflow_exit_reason` is ordinary data.

Enabled tasks require a plain `type: object` output schema, without top-level
`$ref`, `allOf`, `anyOf`, or `oneOf`, or no output schema. Without a user schema,
RelayFold supplies an object schema and retains the result. Agents must then
return JSON; they can put their normal answer in the optional `response` string.
The configured schema remains unchanged; dispatch and result validation use the
same derived schema. Normal required output fields must still be present when
requesting exit.

An exit request preserves the triggering task's output and marks remaining
pending tasks as `Skipped`. Skipped tasks are not dispatched. Already-running tasks
finish, and the workflow completes once those tasks succeed; `Skipped` counts
as terminal for completion. If an already-running task fails, the workflow
becomes `Failed` under normal failure behavior.

The control is independent of task kind. For an external API response that does
not follow this output contract, use a downstream Function to interpret the
response and return `_workflow_exit_reason` at the top level with
`control.allow_early_exit: true` on that Function.
There are no JSON Pointer conditions or static YAML reasons.

The triggering attempt records `early_exit: true`, exposed in task status reports
and task-result metadata. Its preserved output contains the dynamic reason. The
`task_early_exit_set` event applies this flag to task state. Other attempts default
to `false`; retries clear the flag. Orchestrator info logs report whether execution continues, exit
is requested, or validation fails. Exit requests include the reason and pending-task
count; validation failures include the error.
Already-running tasks may request human input; the workflow waits for the response
and allows the continuation to finish while downstream attempts remain skipped.

Tasks within verifier slices can trigger exit. A slice containing skipped attempts
or an attempt with `early_exit: true` creates no further generations and does not
require acceptance for workflow completion. A verifier's `continue` result and
feedback are retained, with that generation unsatisfied.

An independent verifier already running may finish its normal retry loop if its
slice is uninterrupted, even if its direct consumers are skipped or it is terminal.
Normal acceptance, exhaustion, and failure rules apply. These retries and human-input
continuations are exceptions to stopping new work after exit. Pending verifiers
skipped by the exit do not run. Operator-pause ordering remains undecided; existing
pause behavior is preserved.

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
