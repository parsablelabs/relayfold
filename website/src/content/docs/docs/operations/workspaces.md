---
title: Task Workspaces
description: Use workspaces to share files safely between related workflow tasks.
---

Task workspaces let related tasks share files during a workflow run. A task can declare a workspace group so later tasks in the same group can see files created or changed by earlier tasks.

## Workflow YAML

Workspace configuration is declared on tasks:

```yaml
workspace:
  group_name: repo
```

Tasks with the same `group_name` share the same workspace for the workflow run. Data bindings still define ordering; sharing a workspace does not create task dependencies by itself.

## Function executor context

Function tasks receive workspace information in their execution context. This lets functions read or write files in the assigned workspace without needing to know the host layout.

## Agent working directory

Pi starts each Agent task in its assigned workspace. Shell commands and
relative paths in Pi's built-in file tools start there, including when a task
resumes an existing session. A task with `workspace.group_name` starts in that
group's shared workspace. A task without a workspace group starts in its own
task workspace. If you clone a repository into a subdirectory of the workspace,
run repository commands from that subdirectory.

## Workspace root and layout

The worker uses a configured workspace root. In Docker-based local installs, the workspace directory is mounted into worker containers so files can persist across task executions for the same workflow workspace group.
The packaged worker uses the numeric UID and GID recorded by `rf init`, so
workspace files have the same ownership as the user who initialized RelayFold.

Workspace paths include the workflow namespace before the workflow instance and
task or group identity. Identical workflow and task IDs in separate namespaces
therefore use separate directories. The worker uses the namespace carried by the
claimed task; it does not derive namespace from worker environment configuration.

Workspaces are scoped to the execution environment and intended to isolate task file access from unrelated host paths.

## Cleanup

Workspace cleanup is a lifecycle concern for the worker environment. Do not treat workspaces as durable application storage unless a workflow explicitly copies artifacts to an external system.

See [Worker Host Pinning](/relayfold/docs/operations/worker-host-pinning/) for how RelayFold keeps workflow tasks on the host that owns workspace state, and [Reliability and Side Effects](/relayfold/docs/operations/reliability/) for retry and idempotency guidance.
