---
title: Install RelayFold Locally
description: Run RelayFold locally with Docker and the rf wrapper.
---

The Docker-first local install path does not require Rust, Node.js, or a source checkout after installation. It uses prebuilt images by default and manages local config under `~/.relayfold`.

:::caution
This local install path is intended for development and evaluation only. It is
not suitable for production deployments. See
[Production Deployment](/relayfold/docs/operations/production-deployment/) for
work-in-progress guidance on changes required to harden this starter
configuration.
:::

```bash
curl -fsSL https://raw.githubusercontent.com/parsablelabs/relayfold/main/packaging/install.sh | sh
rf init --version VERSION
rf up
```

Pass a different release to `--version` when needed. If `--version` is omitted,
`rf init` resolves and uses the latest RelayFold GitHub release:

```bash
rf init
```

`rf up` runs in the foreground and streams the container logs to the
terminal. Press `Ctrl+C` to stop the containers. Run commands such as
`rf status` from another terminal while RelayFold is running.
On Apple Silicon Macs, `rf up` automatically runs the published AMD64 images
through Docker's architecture emulation by setting
`DOCKER_DEFAULT_PLATFORM=linux/amd64`.

The installer also creates `relayfold` as an alias for `rf` so existing commands
continue to work.

## Try an example

Once RelayFold is running, start with the
[Simple Function Workflow](/relayfold/docs/examples/simple-function-workflow/) to
register and execute a minimal workflow through the HTTP API.

For more complete patterns, try the
[Human Input Workflow](/relayfold/docs/examples/human-input-workflow/),
[GitHub Issue to PR workflow](/relayfold/docs/examples/github-issue-pr-workflow/),
or [Daily Stock Report workflow](/relayfold/docs/examples/daily-stock-report-workflow/).

## Local files

`rf init` creates local state under:

```text
~/.relayfold/
  config.env
  file_credentials.json
  docker-compose.yml
  cache/
  skills/
  workspaces/
  workflows/
```

The generated config is written to `~/.relayfold/config.env`, and the generated Compose file is written to `~/.relayfold/docker-compose.yml`.
The installer places the canonical Compose template beside the `rf`
executable, and `rf init` copies that template into the local environment.
It also records the current user's numeric UID and GID so the non-root worker
can write to the bind-mounted `workspaces/` and `cache/` directories.
Workflow definitions and run state use SQLite at `/tmp/relayfold.db` inside the
orchestrator container. This database is temporary and is discarded when the
orchestrator stops.

## Global namespace mode

Public resource endpoints require a namespace. Local and single-tenant
deployments can select the readable built-in global namespace:

:::caution
Bearer-token API-key authentication is not fully implemented yet. The generated
local Compose environment sets `RELAYFOLD_USE_GLOBAL_NAMESPACE=true` so public
resource endpoints are usable.
:::

```text
RELAYFOLD_USE_GLOBAL_NAMESPACE=true
```

When enabled, RelayFold selects the exact namespace `global-namespace`. This mode
is authoritative, so public requests do not need an authorization header and
ignore one if supplied.

When `RELAYFOLD_USE_GLOBAL_NAMESPACE` is unset or `false`, missing or malformed
bearer credentials return `401 Unauthorized`. A well-formed bearer credential
cannot authenticate requests until API-key-to-namespace resolution is
implemented. Values other than `true` or `false` are invalid. Health checks
remain available without namespace configuration or authorization.

The repository's local-development `docker-compose.yml` explicitly uses
`RELAYFOLD_USE_GLOBAL_NAMESPACE=true`.

## Image overrides

Override image references in `~/.relayfold/config.env` when using an internal registry:

```text
RELAYFOLD_ORCHESTRATOR_IMAGE=registry.example.com/relayfold-orchestrator:dev
RELAYFOLD_WORKER_IMAGE=registry.example.com/relayfold-worker:dev
```

## Self-build path

Users who need to own their image artifacts can build them from a checkout or git ref:

```bash
packaging/build-images.sh --ref vVERSION --tag-prefix registry.example.com/relayfold --push
```

Use the source-build path for contributor workflows or controlled image publishing. The normal local-user path should stay Docker-first and use published images.

## Orchestrator commands and Codex login

The orchestrator executable requires an explicit command: `orchestrator start`
runs the server; bare `orchestrator` prints help. Container images pass `start`
by default. For local source development, `scripts/run-orchestrator-dev.sh`
also uses `start`.

`orchestrator login openai-codex` connects your existing ChatGPT account without
a running server or worker. `orchestrator auth-status` checks stored connection
presence, and `orchestrator logout openai-codex` removes local credentials.
OAuth data uses `RELAYFOLD_OAUTH_DIR` (default `~/.relayfold/oauth`) and requires
persistent writable storage on the orchestrator only. Release Compose mounts
`$RELAYFOLD_HOME/oauth`; `rf init` creates this directory with restrictive
permissions. The orchestrator runs with the configured installation UID/GID so
it can write that directory.

See [Task Credentials](/relayfold/docs/operations/credentials/#codex-subscription-sign-in)
for headless and Docker login instructions and selecting Codex models.
