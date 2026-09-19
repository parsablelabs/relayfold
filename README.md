# RelayFold

<p align="center">
  <img src="resources/relayfold_logo.png" alt="RelayFold logo" width="670">
</p>

RelayFold is an agentic workflow orchestrator for composing AI agents, JavaScript
functions, and API calls into reliable, observable multi-step runs.

Workflows define task dependencies, data flow, schemas, credentials, and
execution constraints. Orchestrator manages workflow state and
scheduling, while workers execute tasks in isolated runtimes. This
separation lets execution scale independently without giving up a consistent, observable
workflow model.

> RelayFold is in early development. Expect bugs and breaking changes.

## What RelayFold Provides

- Mixed workflows of [Agent, Function, and API Call tasks](https://parsablelabs.github.io/relayfold/docs/concepts/tasks/)
- Explicit [workflow data flow and runtime state](https://parsablelabs.github.io/relayfold/docs/concepts/workflows/)
- Observable runs that can be [paused, resumed, and retried](https://parsablelabs.github.io/relayfold/docs/concepts/workflow-lifecycle/)
- [Human input](https://parsablelabs.github.io/relayfold/docs/concepts/human-input/) and [bounded verifier loops](https://parsablelabs.github.io/relayfold/docs/concepts/bounded-loops/) for agentic workflows
- Controlled access to [credentials](https://parsablelabs.github.io/relayfold/docs/operations/credentials/) and [workspaces](https://parsablelabs.github.io/relayfold/docs/operations/workspaces/)
- Independently scalable [orchestrators and workers](https://parsablelabs.github.io/relayfold/docs/operations/scaling/)

## Get Started

The [RelayFold documentation](https://parsablelabs.github.io/relayfold/docs/) is the
authoritative source for installation, concepts, guides, examples, and API
details.

- [Install RelayFold locally](https://parsablelabs.github.io/relayfold/docs/install/)
- [Register and run your first workflow](https://parsablelabs.github.io/relayfold/docs/guides/register-and-run-workflow/)
- [Read the workflow YAML reference](https://parsablelabs.github.io/relayfold/docs/concepts/workflow-yaml/)
- [Try a simple Function workflow](https://parsablelabs.github.io/relayfold/docs/examples/simple-function-workflow/)
- [Use the API reference](https://parsablelabs.github.io/relayfold/docs/api-reference/)
- [Understand the architecture](https://parsablelabs.github.io/relayfold/docs/concepts/architecture/)

## Repository Layout

- [`orchestrator/`](orchestrator/) — Rust control plane for workflow state,
  scheduling, persistence, and APIs
- [`worker/`](worker/) — TypeScript runtime for executing workflow tasks
- [`website/`](website/) — project website and user documentation

## Local Development

- Start both Orchestrator and Worker

`docker compose up --build`

## Acknowledgements

RelayFold's Agent runtime is built on the excellent
[Pi coding agent](https://github.com/earendil-works/pi-mono) project. Pi provides
the underlying agent session, model, tool, and skill capabilities used by
RelayFold workers.

## License

RelayFold is licensed under the [Apache License 2.0](LICENSE).
