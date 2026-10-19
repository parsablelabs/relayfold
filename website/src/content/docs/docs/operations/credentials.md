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

<<<<<<< Updated upstream
RelayFold currently supports Agent model authentication through
provider-standard API-key environment variables. It does not create a Pi
runtime override from `required_credentials`, and it does not manage
persistent Pi or OAuth authentication.
||||||| Stash base
API-key models continue to use provider-standard environment variables. Codex
subscription models use the installation's centrally stored OAuth connection.

## Codex subscription sign-in

Sign in with your existing ChatGPT account on the orchestrator host:

```bash
orchestrator login openai-codex
orchestrator auth-status
orchestrator start
```

`orchestrator` without a command prints help. Login does not require a running
server or a connected worker. Open the displayed authorization URL in a browser.
You can use your laptop's browser for a headless server: paste the full resulting
`http://localhost:1455/auth/callback?...` URL into the login prompt, including
`code` and `state`, even if the browser cannot load that address. A browser on the
same host can complete the localhost callback automatically. Ctrl-C cancels login;
an attempt times out after five minutes.

Workers use Pi `0.99.1`, which includes GPT-6.1 Sol for Codex subscription access. Worker Docker images use Node.js 24; running workers directly requires Node.js 22.19.0 or newer. Rebuild or upgrade worker images to use the updated catalog.

Use a Codex model available in the installed Pi catalog, for example:

```yaml
kind:
  agent:
    model_id: "openai-codex/gpt-6.1-sol"
    provider_url: ""
    prompt: "Review the supplied change."
    tools: []
    skills: []
required_credentials: []
```

No OAuth secret belongs in `required_credentials`. Add tool credentials such as
`gh_token` normally if the task needs them. `openai/...` continues to use API
billing/authentication; it does not use this subscription connection.

There is one Codex account per installation, shared across namespaces and users.
A successful login replaces the prior account; failed or cancelled login preserves
it. `auth-status` prints `Codex: credentials stored` when credentials are present.
It does not contact the provider to verify token validity or model entitlement.
Provider subscription limits and selected-model access still apply. Authentication
errors use existing task failure and retry behavior; there is no automatic switch
to API billing.

If a task reports Codex credential retrieval failure, run `orchestrator auth-status`
using the same OAuth directory as the running server, reconnect if necessary,
and check that the worker can reach the orchestrator's worker API.

For the development Docker Compose installation, you can also run login inside
the orchestrator container:

```bash
docker compose exec orchestrator orchestrator auth-status
docker compose exec orchestrator orchestrator login openai-codex
```

Open the printed URL in your browser and paste the complete callback URL into
the terminal when prompted. The container and host binary share the host's OAuth
directory when configured as described below.

The orchestrator stores credentials under `RELAYFOLD_OAUTH_DIR`, defaulting to
`~/.relayfold/oauth`. Keep this directory persistent and writable by the
orchestrator user. Workers use `RELAYFOLD_ORCHESTRATOR_HTTP_URL` to request access
credentials before each model request, including tool continuations. This is the
same orchestrator URL used for worker registration and task polling; it defaults
to `http://127.0.0.1:3001` for local development. Set it to the orchestrator’s
worker API address when running workers on another host or in containers. Only the
orchestrator stores refresh tokens and renews them. No OAuth volume is needed on
workers. This setup supports one orchestrator instance; independently copied
stores are not a coordinated multi-replica setup.

The worker credential endpoint assumes a trusted worker network and has no caller
API-key validation in this initial implementation. Anyone who can reach it can
obtain the installation's access credentials. Keep it on the installation's
trusted network.

To remove the stored connection:

```bash
orchestrator logout openai-codex
```

Logout is local only: it prevents subsequent credential retrieval and renewal,
without revoking provider authorization or cancelling running tasks. Already-issued
access tokens may remain usable. A new login reconnects the installation.

### Test with one Agent task

The ready-to-run
[`examples/example_codex_oauth_workflow.yaml`](https://github.com/parsablelabs/relayfold/blob/main/examples/example_codex_oauth_workflow.yaml)
contains one Codex Agent that turns supplied release facts into a short
announcement. It needs no trigger input, tools, skills, or API-key credentials.

After signing in, start the orchestrator and at least one worker. From the
repository root, register and run the example against a local installation:

```bash
export RELAYFOLD_URL=http://localhost:3000
curl -sS -X POST "$RELAYFOLD_URL/workflow-def" \
  --data-binary @examples/example_codex_oauth_workflow.yaml
curl -sS -X POST "$RELAYFOLD_URL/workflow-def/codex-oauth-test" \
  -H 'content-type: application/json' -d '{}'
```

Copy the returned run `id` into `WORKFLOW_ID`, then inspect status and output:

```bash
WORKFLOW_ID='<returned run id>'
curl -sS "$RELAYFOLD_URL/workflows/$WORKFLOW_ID"
curl -sS "$RELAYFOLD_URL/workflows/$WORKFLOW_ID/tasks/summarize-changes"
```

A completed task with an announcement in `output.response` verifies a live model
request through the Codex connection. It does not by itself verify token renewal.
If that model is unavailable to your account, choose another model supported by
the installed Pi `openai-codex` catalog.

### Docker installations

Release installations created with `rf init` persist OAuth credentials in
`~/.relayfold/oauth` and mount them only on the orchestrator. After upgrading an
existing installation, refresh its Compose configuration with `rf init`.

Run login without starting dependencies:

```bash
docker compose --env-file ~/.relayfold/config.env \
  -f ~/.relayfold/docker-compose.yml run --rm --no-deps orchestrator login openai-codex
```

Use the same command with `auth-status` or `logout openai-codex` to manage the
connection. Paste the redirect URL for container login; publishing a browser
callback port is unnecessary for this manual path.

For the repository development stack:

```bash
export RELAYFOLD_ORCHESTRATOR_UID=$(id -u)
export RELAYFOLD_ORCHESTRATOR_GID=$(id -g)
mkdir -p "${RELAYFOLD_OAUTH_DIR:-$HOME/.relayfold/oauth}"
chmod 700 "${RELAYFOLD_OAUTH_DIR:-$HOME/.relayfold/oauth}"
cargo run --manifest-path orchestrator/Cargo.toml -- login openai-codex
docker compose up -d
```

The development stack mounts `~/.relayfold/oauth` into the orchestrator at
`/var/lib/relayfold/oauth`. Set `RELAYFOLD_OAUTH_DIR` to an absolute host path to
use another directory. Run the container with your host UID/GID as above, or save
those two values in the repository's Compose `.env` file. Both processes need
write access because token renewal updates the stored credentials. Host login
does not require a running container, and its browser callback works directly
on the host.

Workers mount only the named string-secret file and approved skills; they obtain
OAuth access tokens through the orchestrator API. Ensure
`~/.relayfold/file_credentials.json` exists before starting workers, using `{}`
when no named secrets are needed.

If upgrading from the previous `relayfold-oauth` named volume, log in locally
again before recreating the orchestrator. Existing volume credentials are not
automatically copied to the host directory.
Development and release containers default to `orchestrator start`.
=======
API-key models continue to use provider-standard environment variables. Codex
subscription models use the installation's centrally stored OAuth connection.

The worker logs `[AgentExecutor] Running agent model` at info level with
`modelId`, `provider`, and `authMethod`. The authentication method is `oauth` for
`openai-codex/...` and `api_key` for other providers. This records the selected
authentication path, not whether authentication succeeded; credential values
are not included in this log entry.

## Codex subscription sign-in

Sign in with your existing ChatGPT account on the orchestrator host:

```bash
orchestrator login openai-codex
orchestrator auth-status
orchestrator start
```

`orchestrator` without a command prints help. Login does not require a running
server or a connected worker. Open the displayed authorization URL in a browser.
You can use your laptop's browser for a headless server: paste the full resulting
`http://localhost:1455/auth/callback?...` URL into the login prompt, including
`code` and `state`, even if the browser cannot load that address. A browser on the
same host can complete the localhost callback automatically. Ctrl-C cancels login;
an attempt times out after five minutes.

Workers use Pi `0.99.1`, which includes GPT-6.1 Sol for Codex subscription access. Worker Docker images use Node.js 24; running workers directly requires Node.js 22.19.0 or newer. Rebuild or upgrade worker images to use the updated catalog.

Use a Codex model available in the installed Pi catalog, for example:

```yaml
kind:
  agent:
    model_id: "openai-codex/gpt-6.1-sol"
    provider_url: ""
    prompt: "Review the supplied change."
    tools: []
    skills: []
required_credentials: []
```

No OAuth secret belongs in `required_credentials`. Add tool credentials such as
`gh_token` normally if the task needs them. `openai/...` continues to use API
billing/authentication; it does not use this subscription connection.

There is one Codex account per installation, shared across namespaces and users.
A successful login replaces the prior account; failed or cancelled login preserves
it. `auth-status` prints `Codex: credentials stored` when credentials are present.
It does not contact the provider to verify token validity or model entitlement.
Provider subscription limits and selected-model access still apply. Authentication
errors use existing task failure and retry behavior; there is no automatic switch
to API billing.

If a task reports Codex credential retrieval failure, run `orchestrator auth-status`
using the same OAuth directory as the running server, reconnect if necessary,
and check that the worker can reach the orchestrator's worker API.

For the development Docker Compose installation, you can also run login inside
the orchestrator container:

```bash
docker compose exec orchestrator orchestrator auth-status
docker compose exec orchestrator orchestrator login openai-codex
```

Open the printed URL in your browser and paste the complete callback URL into
the terminal when prompted. The container and host binary share the host's OAuth
directory when configured as described below.

The orchestrator stores credentials under `RELAYFOLD_OAUTH_DIR`, defaulting to
`~/.relayfold/oauth`. Keep this directory persistent and writable by the
orchestrator user. Workers use `RELAYFOLD_ORCHESTRATOR_HTTP_URL` to request access
credentials before each model request, including tool continuations. This is the
same orchestrator URL used for worker registration and task polling; it defaults
to `http://127.0.0.1:3001` for local development. Set it to the orchestrator’s
worker API address when running workers on another host or in containers. Only the
orchestrator stores refresh tokens and renews them. No OAuth volume is needed on
workers. This setup supports one orchestrator instance; independently copied
stores are not a coordinated multi-replica setup.

The worker credential endpoint assumes a trusted worker network and has no caller
API-key validation in this initial implementation. Anyone who can reach it can
obtain the installation's access credentials. Keep it on the installation's
trusted network.

To remove the stored connection:

```bash
orchestrator logout openai-codex
```

Logout is local only: it prevents subsequent credential retrieval and renewal,
without revoking provider authorization or cancelling running tasks. Already-issued
access tokens may remain usable. A new login reconnects the installation.

### Test with one Agent task

The ready-to-run
[`examples/example_codex_oauth_workflow.yaml`](https://github.com/parsablelabs/relayfold/blob/main/examples/example_codex_oauth_workflow.yaml)
contains one Codex Agent that turns supplied release facts into a short
announcement. It needs no trigger input, tools, skills, or API-key credentials.

After signing in, start the orchestrator and at least one worker. From the
repository root, register and run the example against a local installation:

```bash
export RELAYFOLD_URL=http://localhost:3000
curl -sS -X POST "$RELAYFOLD_URL/workflow-def" \
  --data-binary @examples/example_codex_oauth_workflow.yaml
curl -sS -X POST "$RELAYFOLD_URL/workflow-def/codex-oauth-test" \
  -H 'content-type: application/json' -d '{}'
```

Copy the returned run `id` into `WORKFLOW_ID`, then inspect status and output:

```bash
WORKFLOW_ID='<returned run id>'
curl -sS "$RELAYFOLD_URL/workflows/$WORKFLOW_ID"
curl -sS "$RELAYFOLD_URL/workflows/$WORKFLOW_ID/tasks/summarize-changes"
```

A completed task with an announcement in `output.response` verifies a live model
request through the Codex connection. It does not by itself verify token renewal.
If that model is unavailable to your account, choose another model supported by
the installed Pi `openai-codex` catalog.

### Docker installations

Release installations created with `rf init` persist OAuth credentials in
`~/.relayfold/oauth` and mount them only on the orchestrator. After upgrading an
existing installation, refresh its Compose configuration with `rf init`.

Run login without starting dependencies:

```bash
docker compose --env-file ~/.relayfold/config.env \
  -f ~/.relayfold/docker-compose.yml run --rm --no-deps orchestrator login openai-codex
```

Use the same command with `auth-status` or `logout openai-codex` to manage the
connection. Paste the redirect URL for container login; publishing a browser
callback port is unnecessary for this manual path.

For the repository development stack:

```bash
export RELAYFOLD_ORCHESTRATOR_UID=$(id -u)
export RELAYFOLD_ORCHESTRATOR_GID=$(id -g)
mkdir -p "${RELAYFOLD_OAUTH_DIR:-$HOME/.relayfold/oauth}"
chmod 700 "${RELAYFOLD_OAUTH_DIR:-$HOME/.relayfold/oauth}"
cargo run --manifest-path orchestrator/Cargo.toml -- login openai-codex
docker compose up -d
```

The development stack mounts `~/.relayfold/oauth` into the orchestrator at
`/var/lib/relayfold/oauth`. Set `RELAYFOLD_OAUTH_DIR` to an absolute host path to
use another directory. Run the container with your host UID/GID as above, or save
those two values in the repository's Compose `.env` file. Both processes need
write access because token renewal updates the stored credentials. Host login
does not require a running container, and its browser callback works directly
on the host.

Workers mount only the named string-secret file and approved skills; they obtain
OAuth access tokens through the orchestrator API. Ensure
`~/.relayfold/file_credentials.json` exists before starting workers, using `{}`
when no named secrets are needed.

If upgrading from the previous `relayfold-oauth` named volume, log in locally
again before recreating the orchestrator. Existing volume credentials are not
automatically copied to the host directory.
Development and release containers default to `orchestrator start`.
>>>>>>> Stashed changes

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
