---
title: RelayFold Documentation
description: Learn what RelayFold is and how its orchestrator, workers, and workflow model fit together.
---

RelayFold is an agentic workflow orchestrator for teams that want to compose AI agents, API calls, and code execution into reliable multi-step runs. Agent tasks use a provider-agnostic model interface, so each task can select the model provider that fits its role without changing the workflow execution model.

It is built around a separation between the control plane and execution plane:

- The **orchestrator** owns workflow definitions, run state, scheduling, and status APIs.
- The **worker** executes individual task payloads in an isolated runtime with typed inputs and outputs.

## Why RelayFold

Most agent demos stop at "the model produced an answer." Real systems need more structure:

- explicit workflow definitions instead of one-off prompts
- task dependencies and data flow between steps
- observable run state
- resumable execution
- typed contracts between tasks
- pluggable execution backends and credentials

RelayFold treats an agent the same way it treats a function or API task: as a node in a workflow with declared inputs, outputs, and credentials.

## RelayFold features

- **Provider-agnostic Agent tasks** — choose the model provider that fits each step without changing the workflow execution model.
- **Mixed task workflows** — compose AI agents, JavaScript functions, and direct API calls in one workflow, with explicit data bindings and optional schema validation between steps.
- **Agent-directed human input** — let an Agent request clarification only when it needs it, pause the workflow in `InputNeeded`, and continue from persisted state after a response.
- **Bounded verifier loops** — repeat AI agent task sequences with structured feedback and a configured attempt limit.
- **Observable, resumable runs** — inspect workflow and task state, follow lifecycle events, and retry or resume work without restarting the entire workflow.
- **Controlled execution environments** — grant each task only the tools, skills, credentials, and shared workspace access it needs.
- **Scalable worker execution** — separate orchestration from task execution so workers can register, claim tasks, and scale independently while preserving workflow-local state.

## Browser console

RelayFold's frontend provides a terminal-inspired browser console with compact
panels, monospace text, blue accents matching the RelayFold logo, and labeled
workflow statuses. The sidebar shows unnumbered menu items, with the active view
highlighted. Use **Workflows** to browse
definitions, inspect diagrams and YAML, and start runs with JSON input. Use
**Instances** to filter runs, inspect task attempts and events, respond to human
input requests, pause or resume runs, and restart failed tasks. Human input forms
appear only while the workflow is in **InputNeeded** and has a task requesting
input; they disappear when the workflow leaves that state. Only the latest
attempt of each task can show an input form, so repeated questions replace the
previous request. Earlier attempts remain visible in the task history. These views
refresh every five seconds and also offer a manual refresh button. Instance
details open on **Instance Status**, which shows task attempts, execution
controls, and a diagram highlighting each step's latest attempt and generation.
Completed diagram nodes and **Success** labels in the instance list, instance
summary, and task attempts use matching green accents.
Verifier tasks appear in the diagram and task table alongside other steps.
Diagrams use compact spacing and small monospace labels; wide diagrams scroll
horizontally within their panel.
Instance status polls while **Pending** or **Running**, and stops in other
states. Use **Refresh** to check for changes made elsewhere. Resume, retry, and
human-input submission refresh immediately and restart polling if the returned
state is active. Diagram YAML loads once and is reused for status updates;
workflow definition pages also load YAML once, with manual Refresh available.
**Instance Events** fetches event history only when opened; reopen
the tab or select its **Refresh** button to update events. Event history does
not refresh automatically.

Configure the orchestrator's public API host in **Settings**. The host is saved
in your browser. Controls support standard Tab navigation and Enter activation.

## Current status

> **RelayFold is still in an early development stage, expect bugs and breaking changes.**

## License

RelayFold is open source under the
[Apache License 2.0](https://github.com/parsablelabs/relayfold/blob/main/LICENSE).

## Where to start?

Start with the [install guide](/relayfold/docs/install/) for local setup, then try [Register and Run a Workflow](/relayfold/docs/guides/register-and-run-workflow/). After that, read the [workflow concepts](/relayfold/docs/concepts/workflows/), [task concepts](/relayfold/docs/concepts/tasks/), [workflow YAML reference](/relayfold/docs/concepts/workflow-yaml/), [API reference](/relayfold/docs/api-reference/), and [architecture overview](/relayfold/docs/concepts/architecture/).
