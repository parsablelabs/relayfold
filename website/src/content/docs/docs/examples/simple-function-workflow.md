---
title: Simple Function Workflow
description: A minimal workflow that uses one inline Function task.
---

This example shows a single Function task that reads trigger input and returns structured output.

The ready-to-run definition is
[`examples/example_simple_function_workflow.yaml`](https://github.com/parsablelabs/relayfold/blob/main/examples/example_simple_function_workflow.yaml)

## Workflow definition

```yaml
id: simple-function-workflow

tasks:
  - id: greeter
    kind:
      function:
        dependencies: []
        code: |
          export default async function run({ inputs }) {
            const user = inputs[0] ?? {};
            const name = user.name ?? "friend";

            return {
              response: `Hello, ${name}!`,
              normalizedName: String(name).trim().toLowerCase()
            };
          }
    input_schemas:
      - type: object
        properties:
          name:
            type: string
    output_schema:
      type: object
      required:
        - response
        - normalizedName
      properties:
        response:
          type: string
        normalizedName:
          type: string
    required_credentials: []

data_bindings: []
```

## Register the workflow

The API accepts JSON and YAML. Download and register the ready-to-run example
directly from GitHub:

```bash
export RELAYFOLD_URL=http://localhost:3000

curl -fsSL https://raw.githubusercontent.com/parsablelabs/relayfold/main/examples/example_simple_function_workflow.yaml \
  | curl -fsS -X POST "$RELAYFOLD_URL/workflow-def" \
      --data-binary @-
```

## Execute the workflow

```bash
curl -fsS -X POST "$RELAYFOLD_URL/workflow-def/simple-function-workflow" \
  -H 'content-type: application/json' \
  -d '{ "name": "Ada Lovelace" }'
```

Example response:

```json
{
  "status": "queued",
  "id": "simple-function-workflow-1780000000000000000",
  "pinned_host_id": "local-dev-host"
}
```

## Schedule the workflow

After registering the workflow, create `scheduler.yaml`:

```yaml
scheduler:
  - namespace: global-namespace
    workflow_def_id: simple-function-workflow
    cron: "* * * * *"
    input:
      name: Scheduler
```

Enable the scheduler and point the orchestrator at the file:

```bash
export RELAYFOLD_SCHEDULER_ENABLED=true
export RELAYFOLD_SCHEDULER_CONFIG_PATH=./scheduler.yaml
```

This schedule starts the workflow once per minute when no instance of the same
workflow is already active.

## Check the output

Replace `<workflow_id>` with the `id` returned when you executed the workflow:

```bash
curl -fsS "$RELAYFOLD_URL/workflows/<workflow_id>/tasks/greeter"
```

Example response:

```json
{
  "status": "success",
  "input": [
    {
      "name": "Ada Lovelace"
    }
  ],
  "output": {
    "response": "Hello, Ada Lovelace!",
    "normalizedName": "ada lovelace"
  },
  "task_def_id": "greeter",
  "task_attempt_id": "greeter[1]",
  "satisfaction": "Satisfied",
  "generation_index": 1
}
```

## Why this is useful

This pattern is the smallest RelayFold workflow shape:

- trigger input becomes the Function task input
- task output is validated with `output_schema`
- task result can feed downstream tasks through `data_bindings`
- no credentials or workspace setup are required
