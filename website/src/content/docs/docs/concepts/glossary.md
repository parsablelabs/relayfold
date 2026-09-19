---
title: Glossary
description: Learn the core terms used throughout RelayFold documentation and APIs.
---

RelayFold uses the following terms consistently in its documentation, API, and
status views. The most important distinction is between a reusable definition
and the state created when that definition runs.

## Workflow model

### Workflow definition

A registered, reusable description of a workflow. It contains task definitions,
data bindings, schemas, and execution settings, but no run state.

### Workflow instance

One run created from a workflow definition. A workflow instance has its own ID,
trigger input, lifecycle state, task attempts, outputs, and event history.

The documentation sometimes uses **run** as shorthand for workflow instance.

### Namespace

An isolation boundary for workflow definitions, workflow instances, and related
operations. Identical workflow definition or instance IDs in different
namespaces identify independent resources.

### Trigger input

The structured value supplied when a workflow instance is created. A root task
with a declared input schema can consume trigger input directly; downstream
tasks receive values from upstream tasks through data bindings.

## Task model

### Task definition

A named logical step in a workflow definition. It selects a task kind and can
declare schemas, credentials, a timeout, workspace sharing, and verifier
control.

The task definition ID remains stable across retries and reruns.

### Task kind

The strategy used to execute a task:

- **Agent** uses an AI model, prompt, tools, and skills.
- **Function** runs JavaScript for deterministic logic or integration work.
- **API Call** makes a direct HTTP-style request.

### Task attempt

A tracked runtime occurrence of a task definition within a workflow instance.
A task attempt has its own status, resolved inputs, structured output, and
generation number.

Human-input continuations and verifier-controlled reruns create new attempts for
the same task definition. A direct retry resets and executes the same failed
attempt again.

### Generation

The numbered sequence of attempts for a logical task. The initial attempt is
generation 1; later attempts increment the generation number. Data bindings
track which upstream generation supplied each input.

### Data binding

A directed connection that passes one task's structured output into a
downstream task's input array. A data binding establishes execution order;
sharing a workspace does not.

### Runnable task

A pending task whose required upstream outputs are satisfied and that can be
dispatched for execution.

### Verifier task

A task with `control.verifier` that evaluates the current generation. It either
accepts the result with `complete` or requests a bounded rerun with `continue`
and feedback.

### Bounded loop

A verifier-controlled rerun of a configured workflow slice. The workflow
definition remains acyclic, and `max_iterations` limits how many generations
the loop can produce.

## State and interaction

### Workflow status

The current lifecycle state of a workflow instance: `Pending`, `Running`,
`Paused`, `InputNeeded`, `Completed`, or `Failed`.

### Task status

The execution state of a task attempt: `Pending`, `Running`, `InputNeeded`,
`Completed`, or `Failed`.

### Human input

Structured information submitted by a person after an Agent task requests it.
The request places the task and workflow in `InputNeeded`; submitting a response
creates a continuation attempt for the same logical task.

## Runtime

### Orchestrator

The RelayFold component that registers workflow definitions, creates and tracks
workflow instances, finds runnable tasks, dispatches task payloads, and exposes
status and workflow APIs.

### Worker

A RelayFold runtime process that registers with the orchestrator, claims task
payloads, executes task attempts, and returns results.

### Worker host

The durable execution-state domain that owns workspace and Agent-session data.
Multiple worker processes can share one worker-host identity when they share
that state.

### Workspace

A workflow-instance-scoped directory used to share files between tasks with the
same workspace group. A workspace is file storage, not an execution dependency.

### Agent session

Worker-managed conversation state for an Agent task, including messages and
tool calls. A reusable Agent session can continue across task attempts, but it
does not replace orchestrator-owned workflow state.

### Credential

A named secret required by a task and resolved by the worker immediately before
execution. Workflow definitions name required credentials but do not contain
their secret values.
