---
title: Function Registry
description: Register reusable JavaScript functions and reference them from workflow definitions.
---

The Function registry stores reusable Function definitions. A workflow can reference a registered function by `ref` instead of embedding code inline in every task.

Use registered functions for shared integrations, common transformations, or any code that is easier to build and test outside a workflow file.

## Create a function

Give your coding agent access to the current RelayFold function implementations
and documentation. If you have not already done so, add RelayFold to your
application repository as a Git submodule:

```bash
cd path/to/your-application
git submodule add https://github.com/parsablelabs/relayfold.git relayfold
```

Open your application repository in your coding agent, then adapt this prompt:

```text
Create one or more reusable RelayFold functions that [describe what the
functions should do].

Use the implementations and build tooling in relayfold/functions/ and the
documentation in relayfold/website/src/content/docs/docs/ as the authoritative
references. Ensure to understand the function's inputs, output and credential 
requirements.

Create the functions in one workspace under [path and package name]. List every
stable Function ID and source path in the workspace's `relayfold.functions`
manifest. Follow the existing RelayFold source, build, and test patterns. Keep
each implementation and its dependencies as small as possible, pin every
runtime dependency to a specific version, and do not invent fields that are not
supported by the current examples or documentation.

Add tests for each Function's behavior and generate the registry-ready JSON and
YAML artifacts. Run the tests and build, then explain each Function's ID,
inputs, output, dependencies, and credentials that a referencing workflow task
must declare. Give me the curl commands to register the JSON artifacts against
$RELAYFOLD_URL.
```

Replace the bracketed text with the desired behavior and output location. Review
the generated code, dependency versions, and credential usage before
registering the Functions.

## Function definition shape

A registered function definition contains:

```json
{
  "id": "format.hello",
  "dependencies": [],
  "code": "export default async function run({ inputs }) { return { response: `Hello, ${inputs[0].name}!` }; }"
}
```

Fields:

- `id`: registry identifier used by workflow `ref`.
- `dependencies`: npm packages the worker installs before execution.
- `code`: JavaScript ESM source that exports a default function.

## Register a function

In the UI, open **Functions → Register function**, paste a complete YAML or JSON
definition, and click **Register function**. The page shows success or the API
error and keeps your pasted definition available for edits. Registering an
existing function ID replaces its code and dependencies.

Choose **View registered functions** after success, or open the **Registered
functions** tab, to browse the registry. The list shows function IDs. Click an ID
to fetch its complete definition and view dependency versions and JavaScript code. The list refreshes
automatically and has a manual **Refresh** button.

```bash
export RELAYFOLD_URL=http://localhost:3000

curl -sS -X POST "$RELAYFOLD_URL/function-def" \
  -d '{
    "id": "format.hello",
    "dependencies": [],
    "code": "export default async function run({ inputs }) { const name = inputs[0]?.name ?? \"friend\"; return { response: `Hello, ${name}!` }; }"
  }'
```

Response:

```json
{
  "status": "created",
  "id": "format.hello"
}
```

## Reference a registered function

Use `ref` in the workflow task:

```yaml
tasks:
  - id: hello
    kind:
      function:
        ref: format.hello
    input_schemas: []
    output_schema:
      type: object
      required:
        - response
      properties:
        response:
          type: string
    required_credentials: []
```

The orchestrator resolves the function reference before dispatching the task to a worker. If the reference does not exist, execution fails before task code runs.

## Delete a function

```bash
curl -i -X DELETE "$RELAYFOLD_URL/function-def/format.hello"
```

Successful deletion returns `204 No Content`.

Delete only when no active or future workflows need the reference. Workflows that still refer to a deleted function cannot execute that Function task.

## Build artifacts from source

Use the [`functions/example` README](https://github.com/parsablelabs/relayfold/blob/main/functions/example/README.md)
for the multi-Function TypeScript workspace template, direct testing, dependency
handling, artifact generation, and guidance for maintaining larger Function
collections. A workspace declares all entries in `package.json` under
`relayfold.functions`; one build emits a YAML and JSON artifact for every entry.

For a complete integration with standalone scanning and publishing Functions,
see [FillMyFunnel CloudWatch Review](/relayfold/docs/examples/fillmyfunnel-cloudwatch-workflow/).
Its Function workspace includes deterministic tests, generated-artifact checks,
and an optional read-only live CloudWatch check.

## When to use inline functions

Use inline functions for small workflow-local logic:

```yaml
kind:
  function:
    dependencies: []
    code: |
      export default async function run({ inputs }) {
        return { response: inputs.length };
      }
```

Use registered functions when code is reused, tested separately, or packaged from TypeScript source.
