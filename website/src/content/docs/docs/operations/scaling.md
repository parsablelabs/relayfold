---
title: Scaling
description: Scale RelayFold by adding workers and partitioning workloads across independent orchestrators.
---

RelayFold separates workflow orchestration from task execution. This gives you two
ways to add capacity:

1. add workers to an orchestrator's worker pool
2. partition workloads across independent orchestrators and their worker pools

Start by scaling workers. Add orchestrator partitions when a single control
plane should no longer own all workloads.

## Deployment overview

One orchestrator can coordinate worker instances across multiple worker nodes.
Instances on the same node use a shared host identity when they also share the
node's workspace and Agent session state.

<pre class="mermaid">
flowchart LR
    C["Workflow clients"]

    subgraph OM["Orchestrator machine"]
        O["RelayFold orchestrator"]
        S[("Workflow storage")]
        O --- S
    end

    subgraph WNA["Worker node A<br/>host ID: worker-node-a"]
        direction TB
        A1["Worker instance A1"]
        A2["Worker instance A2"]
        AS[("Shared workspaces<br/>and Agent sessions")]
        A1 --- AS
        A2 --- AS
    end

    subgraph WNB["Worker node B<br/>host ID: worker-node-b"]
        direction TB
        B1["Worker instance B1"]
        B2["Worker instance B2"]
        BS[("Shared workspaces<br/>and Agent sessions")]
        B1 --- BS
        B2 --- BS
    end

    C -->|"workflow API"| O
    A1 <-->|"worker API"| O
    A2 <-->|"worker API"| O
    B1 <-->|"worker API"| O
    B2 <-->|"worker API"| O
</pre>

The orchestrator schedules and tracks workflows. Worker instances register,
claim tasks, and return results through the worker API. A workflow is pinned to
one worker-node host identity, but any eligible instance registered for that
host can execute its tasks.

## Add workers

Each worker registers with one orchestrator, polls that orchestrator for
runnable tasks, and posts task results back to it. Adding worker instances
increases the number of tasks the pool can execute without changing workflow
definitions.

Configure every worker in the pool with the worker API URL for the same
orchestrator:

```bash
RELAYFOLD_ORCHESTRATOR_HTTP_URL=http://orchestrator:3001
```

For a Docker Compose installation, scale the worker service from the directory
that contains the generated Compose file:

```bash
docker compose up -d --scale worker=4
```

Worker processes need unique worker IDs. By default, RelayFold derives an ID from
the worker hostname and a random UUID generated at startup, including when
containers share host networking. If you set `WORKER_ID` explicitly, give each
worker process a unique value.

### Container workers with a local orchestrator

For Linux or macOS development from the source checkout, run the orchestrator on the
host and two container workers with:

```bash
scripts/run-local-dev.sh
```

The script builds the orchestrator using the local Cargo cache, builds and starts
only the Compose workers, and runs the orchestrator in the foreground. Ctrl-C
stops the workers. Pass `--skip-build` to reuse the existing orchestrator binary;
worker images are still built. Storage and logging settings use the same
environment variables as `scripts/run-orchestrator-dev.sh`.
On macOS, the script points workers at `http://host.docker.internal:3001`.

To manage the processes separately, run `scripts/run-orchestrator-dev.sh` in one
terminal and start workers in another:

```bash
docker compose -f docker-compose.yml -f docker-compose.local-workers.yml up -d --build --no-deps worker
```

On macOS with Docker Desktop, set the host gateway URL when starting workers:

```bash
RELAYFOLD_ORCHESTRATOR_HTTP_URL=http://host.docker.internal:3001 \
  docker compose -f docker-compose.yml -f docker-compose.local-workers.yml up -d --build --no-deps worker
```

The override uses host networking so workers can reach the orchestrator at
`http://127.0.0.1:3001` on Linux. The worker URL can be overridden with
`RELAYFOLD_ORCHESTRATOR_HTTP_URL`. `--no-deps` skips starting the Compose orchestrator.
Keep the local orchestrator's worker API on the default `127.0.0.1:3001` address
when using this override. Stop any Compose orchestrator already using port 3000
before starting the local process. The root Compose file continues to use
`http://orchestrator:3001` for the full container stack.
The base Compose configuration starts two workers. When switching an existing
worker deployment to this override, Compose recreates the worker containers.
If launching an instance returns `503` with no eligible worker, check
`docker compose logs --tail 50 worker`. Repeated connection refusals at
`127.0.0.1:3001` on macOS indicate that workers need the host gateway URL above.

### Host identity and shared state

`RELAYFOLD_WORKER_HOST_ID` identifies the durable state domain that owns task
workspaces and Agent sessions. It is not the identity of an individual worker
process.

Use the same host ID for workers only when they can access the same workspace
and session roots. Workers that do not share those roots need different host
IDs. RelayFold pins each workflow instance to one eligible host, and workers
registered for that host can execute its tasks.

See [Worker Host Pinning](/relayfold/docs/operations/worker-host-pinning/) for the
continuity and retry behavior that follows from host identity.

### Worker scaling limits

Adding workers increases task-execution capacity. It does not increase the
capacity of the orchestrator's scheduling, API, queue, or storage path. Monitor
both task throughput and orchestrator load to decide when to add workers and
when to create another partition.

The local installation defaults `RELAYFOLD_MAX_CONCURRENT_WORKFLOWS` to `1`.
Increase it when you want the orchestrator to execute more workflow instances
concurrently; otherwise additional workers may remain idle when only one
workflow is runnable.

## Partition orchestrators

For the next level of scale, divide workloads into partitions and run an
independent orchestrator for each partition. Register a dedicated worker pool
with each orchestrator:

```text
clients ── routing rule ──┬── orchestrator A ── workers A
                          └── orchestrator B ── workers B
```

A partition can represent a tenant group, environment, region, workload class,
or another stable boundary that fits your operating model. Route workflow
registration, invocation, and later status or control requests to the
orchestrator that owns that partition.

Each partition should have:

- its own orchestrator endpoint
- workers configured to use that orchestrator's worker API
- its own storage and operational lifecycle
- routing that consistently sends a workflow's requests to its owning
  orchestrator

Orchestrator partitioning is horizontal sharding, not an active-active cluster.
RelayFold does not automatically route, replicate, or rebalance workflows between
orchestrator instances. Do not point independent orchestrators at the same
storage database as a substitute for clustering.

## Choosing a scaling strategy

Add workers when execution is the bottleneck and the existing orchestrator can
comfortably schedule and track the workload. Add an orchestrator partition when
you need more control-plane capacity, stronger workload isolation, independent
failure domains, or separate operational ownership.

Prefer stable partition boundaries. Moving an in-progress workflow between
partitions also means moving its persisted state and any host-local workspace
or Agent session data; RelayFold does not perform that migration automatically.
