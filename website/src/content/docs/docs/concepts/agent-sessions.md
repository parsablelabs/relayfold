---
title: Agent Sessions
description: Understand how Agent tasks reuse conversation state across retries, human input, and verifier feedback.
---

Agent tasks can persist conversation sessions on the worker. A session contains Agent conversation state such as messages, tool calls, compaction entries, and provider-specific details. It does not replace workflow state.

RelayFold keeps workflow state separate from Agent transcripts. The orchestrator remains the source of truth for workflow status, task attempts, verifier generations, input mappings, `InputNeeded` questions, and downstream data binding.

## Reuse policy

Agent task definitions support `reuse_session`:

```yaml
kind:
  agent:
    model_id: "google/gemini-2.5-flash"
    provider_url: ""
    prompt: Review the current implementation.
    tools: []
    skills: []
    ask: false
    reuse_session: true
```

When `reuse_session` is omitted, RelayFold treats it as enabled.

When `reuse_session: true`, the worker derives a session key from the claimed
namespace, workflow instance ID, and logical task ID. Later attempts for the same
logical Agent task can continue the same conversation after human input, retry, or
verifier feedback. Identical workflow and task IDs in another namespace use a
different session.

When `reuse_session: false`, later attempts start without prior logical-task conversation history. Use this when the Agent should evaluate the current inputs fresh every time.

## When to disable reuse

Disable session reuse for Agent verifier tasks when the verifier should judge each generation independently:

```yaml
kind:
  agent:
    prompt: Decide whether the current draft satisfies the acceptance criteria.
    reuse_session: false
```

This avoids a verifier carrying its own prior critique or rejected-output context into the next generation. Keep reuse enabled for implementation or drafting Agents when conversational continuity helps them apply feedback.

## Continuations

RelayFold rebuilds the Agent system instructions on every attempt, including
restored sessions. These include tool and skill guidance, user-input rules, output
format requirements, and early-exit instructions when enabled. Task prompts and
execution data are sent as user messages.

For an initial reusable Agent attempt, the worker prompts the Agent with the task prompt and resolved inputs.

For a continuation attempt with an existing session, the worker appends only the current event context. Examples include:

- a submitted human response
- verifier feedback for the task being regenerated
- the current attempt metadata

The previous prompt, tool results, and assistant turns are expected to already exist in the loaded session.

If a reusable session cannot be loaded, the worker creates a fresh session and rebuilds enough context from structured workflow data: task prompt, upstream inputs, previous output, feedback history when available, and the current human response or verifier feedback.

## Worker-local storage

The default worker file session store is worker-local. It persists across attempts handled by the same worker environment, but it is not durable application storage.

RelayFold does not expose session file paths through task payloads, task results, or orchestrator state. Function and API Call tasks do not receive Agent session keys or transcript contents.

## Relationship to data bindings

Downstream tasks consume structured task outputs, not Agent transcripts. If a downstream step needs data, the Agent must return it in the task output and the workflow should validate it with `output_schema`.
