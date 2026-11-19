---
title: Agent Tasks
description: Use model-backed tasks for reasoning, tool use, and review steps.
---

Agent tasks run through the worker's agent executor. They are useful when a workflow step needs language reasoning, tool use, code editing, review, or a decision over ambiguous input.

```yaml
tasks:
  - id: review-change
    kind:
      agent:
        model_id: "google/gemini-2.5-flash"
        provider_url: ""
        prompt: |
          Review the implementation against the requested behavior.
          Return a concise result with accepted=true only when the change is complete.
        tools: []
        skills: []
        ask: false
        schema_failure_retry_times: 2
    output_schema:
      type: object
      required:
        - accepted
        - response
      properties:
        accepted:
          type: boolean
        response:
          type: string
    required_credentials:
      - gemini_api_key
```

## Reasoning level

Set optional `thinking_level` on an Agent to control its reasoning effort:

```yaml
kind:
  agent:
    model_id: "openai-codex/gpt-6.1-sol"
    thinking_level: high
    # Include the other required Agent fields as usual.
```

Accepted values are `off`, `minimal`, `low`, `medium`, `high`, `xhigh`, and `max`.
Pi adjusts the requested level to the selected model's supported levels. Models
without reasoning support use `off`.

An explicit task value overrides Pi settings and the level saved in a reused
session. When omitted, Pi preserves its existing behavior: reuse the session's
level, otherwise use model-specific or global Pi settings, falling back to
`medium` before adjusting for model support.

## Prompt and inputs

The agent prompt defines the task objective. Inputs from workflow triggers and upstream data bindings are provided to the task execution context so the agent can reason over earlier outputs. Task instructions are refreshed on every attempt, including when an existing session is restored. Session reuse preserves conversation history while applying the current task settings, including `thinking_level`.

Use schemas when downstream tasks depend on specific fields. A schema makes the contract explicit and gives RelayFold a clear validation boundary before the workflow continues.

## Structured output

When `output_schema` is present, the Agent's final response must be a JSON value
that satisfies the schema. RelayFold accepts raw JSON and a whole-response
`json` code fence. Markdown code fences contained inside JSON string fields,
such as an issue body or generated documentation, remain part of the string and
do not affect response parsing.

## Tools and skills

`tools` declares the approved tool names the agent may use. `skills` declares selected skills that should be loaded for the task.

Keep tool access narrow. If a task only needs to classify text, it should not receive file or shell tools. If it needs to edit a workspace, pair tool approval with a workspace group and a prompt that names the expected file work.

For workflows that change external systems, prefer read-only Agent access and
delegate mutations to narrow Function tasks. See
[Safe Agentic Workflows](/relayfold/docs/guides/safe-agentic-workflows/) for the
read, decide, and mutate pattern.

## Credentials

Every `required_credentials` entry is resolved by name and exposed as an
uppercase environment variable for the complete Agent execution. No entry has
special positional meaning.

Use provider-standard names for model API keys, such as `gemini_api_key` for a
`google/...` model or `openai_api_key` for an `openai/...` model. API-key models use these environment variables. For Codex subscription models,
use `openai-codex/<model-id>` after signing in on the orchestrator with
`orchestrator login openai-codex`. The worker obtains access credentials from the
orchestrator before each model request; OAuth tokens are not entries in
`required_credentials`.

See [Task Credentials](/relayfold/docs/operations/credentials/) for provider mappings.
Required credentials are also available to approved Agent tools, so grant both
credentials and tools narrowly.

## Human input

When `ask` is enabled, RelayFold automatically provides the built-in `ask_user`
tool. It does not need to be listed in `tools`. If the task reaches a point
that requires clarification, the workflow can move to `InputNeeded`. After
input is supplied, RelayFold can continue the workflow from the persisted state.

When `ask` is false, `ask_user` is unavailable even when `tools` contains
`ask_user` or `"_all_"`.

See [Human Input](/relayfold/docs/concepts/human-input/) for the `ask_user` flow, API call, and continuation behavior.

## Verifier loops

Agent tasks can participate in [bounded loops](/relayfold/docs/concepts/bounded-loops/) as either the task being revised or the verifier task. Use an Agent verifier when the acceptance decision requires judgment, such as reviewing a generated report, assessing a code change, or deciding whether output satisfies ambiguous requirements.

## Sessions

Agent tasks can reuse conversation state across attempts. See [Agent Sessions](/relayfold/docs/concepts/agent-sessions/) for `reuse_session` behavior and guidance on when verifier Agents should start fresh.
