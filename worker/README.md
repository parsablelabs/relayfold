# RelayFold Worker

The worker is the Node.js process that executes tasks for the RelayFold
orchestrator. It registers with the orchestrator, claims work, executes tasks,
and reports results. Workers initiate all communication with the orchestrator
over HTTP.

This README is for contributors working on the worker. For user-facing
documentation, see:

- [Installation](../website/src/content/docs/docs/install.md)
- [API reference](../website/src/content/docs/docs/api-reference.md)
- [Task types](../website/src/content/docs/docs/concepts/tasks/index.md)
- [Register and run a workflow](../website/src/content/docs/docs/guides/register-and-run-workflow.md)
- [Workflow examples](../examples)

## Requirements

- Node.js 24+
- For sandboxed workflows: QEMU (`qemu-system-aarch64` or `qemu-system-x86_64`) and `qemu-img`. The worker Docker images include these tools.
- npm
- Docker, if building the worker image

## Development

Install dependencies:

```bash
npm install
```

Build TypeScript:

```bash
npm run build
```

Run the worker from compiled output:

```bash
npm start
```

Run the worker from TypeScript source:

```bash
npm run dev
```

Run tests:

```bash
npm test
```

Commands in this section run from the `worker/` directory.

To verify real Gondolin execution with QEMU installed, run:

```bash
npm run build
RELAYFOLD_TEST_GONDOLIN=1 node --test test/GondolinSandbox.integration.test.mjs
```

The integration test downloads guest assets on first use and exercises public
HTTP access and npm dependency installation. It is skipped by normal `npm test`.

## Runtime behavior

By default, the worker connects to the orchestrator worker API at
`http://127.0.0.1:3001`. Set `RELAYFOLD_ORCHESTRATOR_HTTP_URL` when that API is
reachable at a different URL.

Set `RELAYFOLD_WORKER_HOST_ID` before starting the worker. It identifies the
durable host state domain that owns the worker's local workspace and session
stores, rather than a short-lived process or container.

The worker registers before polling for tasks. Registration is retried until it
succeeds, which allows the worker to start before the orchestrator is ready
during container startup.

Credentials are loaded from `~/.relayfold/file_credentials.json`. The file must
contain a flat JSON object whose keys are credential names and whose values are
strings:

```json
{
  "gemini_api_key": "example-gemini-key",
  "system_brave_api_key": "example-brave-key"
}
```

For details about credential exposure, task execution, workspaces, and worker
host identity, see the website documentation:

- [Credentials](../website/src/content/docs/docs/operations/credentials.md)
- [Workspaces](../website/src/content/docs/docs/operations/workspaces.md)
- [Worker host pinning](../website/src/content/docs/docs/operations/worker-host-pinning.md)

## Configuration

| Variable | Default | Description |
| --- | --- | --- |
| `RELAYFOLD_ORCHESTRATOR_HTTP_URL` | `http://127.0.0.1:3001` | Worker API base URL used for registration, task claiming, and task completion. |
| `RELAYFOLD_WORKER_HOST_ID` | required | Stable host identity. Workers sharing durable workspace and session roots must use the same value. |
| `RELAYFOLD_WORKSPACE_ROOT` | `$HOME/.cache/relayfold/workspaces` | Root for task workspaces. The Docker Compose worker uses `/workspaces`. |
| `WORKER_ID` | hostname plus process ID | Worker identity sent during registration. |
| `RELAYFOLD_FUNCTION_TIMEOUT_MS` | `300000` | Timeout for Function dependency installation and execution. |
| `RELAYFOLD_TASK_TIMEOUT_SECS` | `300` | Fallback timeout for tasks without `timeout_secs`. |
| `RELAYFOLD_AGENT_EXTENSION_PATHS` | unset | Comma-separated Pi extension files, directories, or package roots. Relative paths resolve from the worker process directory. |
| `RELAYFOLD_PI_AGENT_DIR` | `$HOME/.pi/agent` | Pi resource-loader directory used for user-level extension discovery metadata. |

Agent session JSONL files are stored under
`$HOME/.cache/relayfold/file_session_store`. This worker-local cache allows Agent
sessions to be reused across attempts handled by the same live worker
container.

## Docker

Build the worker image from the repository root:

```bash
docker build -t relayfold-worker worker
```

The image installs Pi resource packages separately from `worker/package.json`.
By default, it includes `@ogulcancelik/pi-web-browse@1.0.6`. Override the
image-only package list with:

```bash
docker build \
  --build-arg RELAYFOLD_PI_PACKAGES="@ogulcancelik/pi-web-browse@1.0.6 @acme/relayfold-tools@1.2.3" \
  -t relayfold-worker worker
```

Use an empty build argument to include no extra Pi packages:

```bash
docker build --build-arg RELAYFOLD_PI_PACKAGES= -t relayfold-worker worker
```

Run the worker with access to the orchestrator worker API:

```bash
docker run --rm \
  -e RELAYFOLD_ORCHESTRATOR_HTTP_URL=http://host.docker.internal:3001 \
  -e RELAYFOLD_WORKER_HOST_ID=local-docker-host \
  -v ~/.relayfold:/home/relayfold/.relayfold:ro \
  relayfold-worker
```

The read-only `~/.relayfold` mount must contain `file_credentials.json`.
