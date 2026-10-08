---
title: Register and Run a Workflow
description: Register a workflow definition, start a run, inspect status, and read task output.
---

This guide walks through the smallest useful RelayFold API flow:

1. Create a workflow definition.
2. Register a workflow definition.
3. Start a workflow instance.
4. Check workflow status.
5. Read task results.

Set the local API URL:

```bash
export RELAYFOLD_URL=http://localhost:3000
```

## Create a workflow

The quickest way to create a workflow is to give your coding agent access to the
RelayFold repository. Add RelayFold to your application repository as a Git
submodule so the agent can inspect the current examples and documentation while
it works:

```bash
cd path/to/your-application
git submodule add https://github.com/parsablelabs/relayfold.git relayfold
```

Open your application repository in your coding agent, then adapt this prompt:

```text
Create a RelayFold workflow that [describe the outcome you want].

Use the workflow examples in `relayfold/examples/` and the documentation in
relayfold/website/src/content/docs/docs/ as the authoritative references. Inspect
my application to understand the inputs, outputs, APIs, and credentials the
workflow needs.

Keep the workflow as small as possible. Define only the required tasks, data
bindings, input and output schemas, and credentials. Do not invent fields that
are not supported by the current RelayFold examples or documentation.

Save an API-ready JSON or YAML workflow definition to [path and filename]. Then explain
the workflow, list the inputs and credentials I must provide, and give me the
curl commands to register and run it against $RELAYFOLD_URL.
```

Replace the bracketed text with your desired outcome and output path. Review the
generated definition and its credential requirements before registering it.

## Register a workflow

Register a one-task Function workflow:

```bash
curl -sS -X POST "$RELAYFOLD_URL/workflow-def" \
  -d '{
    "id": "hello-workflow",
    "example_input": { "name": "Ada" },
    "tasks": [
      {
        "id": "hello",
        "kind": {
          "function": {
            "dependencies": [],
            "code": "export default async function run({ inputs }) { const name = inputs[0]?.name ?? \"friend\"; return { response: `Hello, ${name}!` }; }"
          }
        },
        "input_schemas": [
          {
            "type": "object",
            "properties": {
              "name": { "type": "string" }
            }
          }
        ],
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

If the definition is a YAML file, register it directly:

```bash
curl -sS -X POST "$RELAYFOLD_URL/workflow-def" \
  --data-binary @hello-workflow.yaml
```

You can register an updated definition under the same ID until its first
workflow instance is created. After an instance exists in any state, including
`Completed` or `Failed`, RelayFold keeps the definition immutable and rejects an
overwrite with `409 Conflict`. Register the update under a new ID instead, for
example `hello-workflow_v2`.

The `_v2` suffix is only a suggested naming convention. RelayFold does not require
or interpret workflow definition versions.

## Start a run

Start a workflow instance from the registered definition:

```bash
curl -sS -X POST "$RELAYFOLD_URL/workflow-def/hello-workflow" \
  -H 'content-type: application/json' \
  -d '{ "name": "Ada" }'
```

Response:

```json
{
  "status": "queued",
  "id": "hello-workflow-1780000000000000000",
  "pinned_host_id": "local-dev-host"
}
```

Save the returned `id`; it is the workflow instance ID used for status and result reads.

If the API returns `503 Service Unavailable`, no eligible worker host is registered yet.

You can also start a run in the UI: open the registered workflow, review or edit
**JSON trigger input**, and choose **Start workflow**. The `example_input` field
in the definition above prefills this input with `{ "name": "Ada" }`. It does
not supply a default for API requests or scheduled runs.

## Check status

```bash
curl -sS "$RELAYFOLD_URL/workflows/hello-workflow-1780000000000000000"
```

Example completed response:

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

## Read task results

List all materialized task results:

```bash
curl -sS "$RELAYFOLD_URL/workflows/hello-workflow-1780000000000000000/tasks"
```

Read one logical task result:

```bash
curl -sS "$RELAYFOLD_URL/workflows/hello-workflow-1780000000000000000/tasks/hello"
```

Example response:

```json
{
  "status": "success",
  "input": [
    {
      "name": "Ada"
    }
  ],
  "output": {
    "response": "Hello, Ada!"
  },
  "task_def_id": "hello",
  "task_attempt_id": "hello[1]",
  "satisfaction": "Satisfied",
  "generation_index": 1
}
```

## Next steps

Use the [Workflow YAML Reference](/relayfold/docs/concepts/workflow-yaml/) when building larger definitions, and the [API Reference](/relayfold/docs/api-reference/) for the full endpoint list.
