---
title: Task Credentials
description: Configure the credentials tasks need before execution.
---

`required_credentials` lists the named secrets a task needs before it can run. The worker reads each name through the configured credentials port.

```yaml
required_credentials:
  - openai_api_key
  - gh_token
```

With the file credential adapter, the worker reads those names from `~/.relayfold/file_credentials.json`:

```json
{
  "openai_api_key": "...",
  "gh_token": "github_pat_..."
}
```

## Runtime exposure

During task execution, the worker exposes every required credential as an uppercased environment variable. For example, `gh_token` becomes `GH_TOKEN` and `openai_api_key` becomes `OPENAI_API_KEY`.

Function tasks receive required credentials in the function context and in the child process environment.

When a workflow declares `sandbox`, each task attempt receives a fresh Gondolin
VM with only its own `required_credentials`. It does not inherit the worker's
environment or a previous task's credential environment. Required credentials
are real values readable by guest code, including model API keys declared by an
Agent task. The VM environment is discarded after the attempt; shared workspace
files persist. There is no workflow-level credential declaration.

Sandboxed `web_search` tasks must explicitly declare `system_brave_api_key`;
the worker does not supply this system credential implicitly inside a sandbox.

Agent tasks expose the complete required credential set while Pi creates the Agent session and executes the prompt. No list position is special. Credentials are also visible to approved Agent tools, so list only what the task needs and keep tool approval narrow.

## Agent model authentication

Pi resolves model authentication from the provider namespace in `model_id`.
For API-key authentication, use the provider-recognized name:

```yaml
kind:
  agent:
    model_id: "google/gemini-2.5-flash"
    # ...
required_credentials:
  - gemini_api_key
```

Common mappings include:

| Credential name | Task environment | Provider namespace |
| --- | --- | --- |
| `openai_api_key` | `OPENAI_API_KEY` | `openai/...` |
| `gemini_api_key` | `GEMINI_API_KEY` | `google/...` |
| `anthropic_api_key` | `ANTHROPIC_API_KEY` | `anthropic/...` |

RelayFold supports Agent model authentication through provider-standard API-key
names. Unsandboxed Agents use environment-based authentication. Sandboxed Agents
resolve the key only from their own `required_credentials` and use an in-memory
Pi authentication override, so another task's environment cannot provide the
model key. RelayFold does not manage persistent Pi or OAuth authentication.

## Missing credentials

If any required credential is missing, the task fails before its main work runs. This keeps credential failures explicit and avoids starting work that cannot complete.

## Usage in API Calls

In API call tasks, credentials can be interpolated into header values using the `${credentials.<name>}` syntax.

```yaml
kind:
  apiCall:
    headers:
      Authorization: "Bearer ${credentials.api_token}"
required_credentials:
  - api_token
```

See [API Call Tasks](/docs/concepts/tasks/api-calls) for more details.
