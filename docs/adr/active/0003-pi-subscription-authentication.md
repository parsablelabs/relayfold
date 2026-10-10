---
status: proposed
date: 2026-10-10
---

# Centralized subscription authentication with Pi workers

RelayFold users want Agent tasks to use subscription-backed model providers,
starting with Codex. The agreed architecture centralizes OAuth login, credential
storage, and token refresh in the Rust orchestrator. Workers keep Pi as the agent
runtime and translate the orchestrator's access credentials into Pi authentication.
This record includes agreed decisions and remaining questions; it does not
represent shipped behavior.

## Current behavior

- `AgentExecutor` resolves the provider and model from `model_id` through Pi's
  `getModel`, then creates a session with empty `AuthStorage.inMemory()`.
- `required_credentials` are string secrets resolved through `CredentialsPort`
  and exposed as uppercase environment variables. Pi uses provider-standard
  variables such as `OPENAI_API_KEY` and `GEMINI_API_KEY`.
- `FileCredentialsAdapter` loads `~/.relayfold/file_credentials.json` at worker
  startup and supports reads only. This path has no OAuth management API.
- Worker credential mounts and worker root filesystems are read-only. There is
  currently no writable OAuth store on the orchestrator.
- The installed Pi packages include OAuth adapters for Codex, Anthropic, and
  GitHub Copilot. Their registry does not include a Google OAuth adapter.
  Adapter availability alone does not establish permitted subscription usage.

No `openspec/specs/` directory is present in this checkout. Check again before
implementation and reconcile applicable specifications with agreed behavior.

## Agreed decisions

### Orchestrator CLI surface

| Invocation | Behavior |
| --- | --- |
| `orchestrator` | Print help and exit without starting the server or initiating login. |
| `orchestrator start` | Start the server with its existing environment-based configuration. |
| `orchestrator login` | Start interactive provider authorization independently of server startup. |

Initial login requires human authorization through a browser, which may run on
another machine. The Codex flow must support displaying an authorization URL and
accepting a pasted redirect URL/code on a headless server. Login requires neither
an execution worker nor a running orchestrator server. Provider selection syntax,
status, and logout commands remain to be defined.

Both orchestrator Dockerfiles will retain the executable entry point and default
container arguments to `start`. Local launch scripts and direct binary
instructions must also use the explicit server command.

### Centralized authentication in Rust

The orchestrator owns provider login, token refresh, and durable credentials.
It does not invoke Node.js or Pi to perform authentication. RelayFold implements
provider-specific OAuth adapters in Rust behind interfaces, starting with Codex.

Share connection lifecycle, storage, refresh coordination, and worker credential
access across providers. Each provider adapter supplies its authorization and
exchange parameters, refresh behavior, and required metadata. Adding another
provider requires validating its supported authentication route and implementing
its adapter; standard OAuth does not make subscriptions interchangeable.

Pi remains responsible for model request protocols and Agent execution. There is
no new Codex app-server, auth service deployment, or inference proxy. Workers use
the orchestrator address they already know to request access credentials and then
call the selected model provider directly. The initial implementation assumes
trusted callers on the worker network, as described below.

### Minimum OAuth permissions

Each provider adapter requests only the scopes required for model inference and
the agreed unattended token renewal. Request identity or account scopes only
when the provider requires them to authorize or select the model account. Do not
request unrelated permissions. Determine the exact minimum for each provider
during compatibility verification; provider scope names are not interchangeable.

### File-based provider credential store

Store OAuth credentials in a dedicated protected file-based store on the
orchestrator, separate from the worker's named string-secret file. This design
initially targets a single orchestrator instance. Persist credentials in a
writable deployment directory with restrictive permissions, atomic updates, and
coordination between CLI operations and the running server.

Preserve provider token-response fields such as `access_token`, `refresh_token`,
and `token_type`, retaining only the response and context fields needed for use
and renewal. Record an absolute expiry derived from `expires_in` and retain
provider identity, OAuth client information, and any required account, project,
scope, or resource metadata. A token response alone is not necessarily sufficient
for later refresh. The file schema and path remain implementation details.

Pi's `access`, `refresh`, `expires`, and provider-specific fields are its storage
representation of provider-issued credentials. The orchestrator's storage and
worker API do not depend on that Pi schema.

### Central refresh and worker translation

The orchestrator is the refresh authority. Serialize refreshes for a connection,
recheck stored expiry after acquiring coordination, and persist replacement
credentials together before returning success. Workers never receive refresh
tokens or mount the OAuth store. They do not independently refresh provider
sessions.

Workers request usable access credentials for their task's provider connection.
The response contains access credentials, expiry, and only the provider metadata
needed to execute requests. It is not a raw token-endpoint response or a full Pi
OAuth credential object.

A worker adapter translates this response into Pi's request authentication.
Some providers need more than a plain bearer token, so validate this boundary
against each supported Pi model provider. Resolve credentials at request time,
including subsequent requests in long-running tasks, rather than keeping one
access token for an entire session. Do not load a refresh-capable OAuth object
into worker Pi auth storage.

### Installation-wide connection scope

Provider connections belong to the RelayFold installation. They are shared
across its namespaces and users rather than owned by an individual namespace or
user. Authorized workers in that installation may use those connections for
tasks across namespaces. Those tasks consume the selected account's allowance.

Workers can share one authorized connection without each performing login.
The initial deployment assumes all callers on the worker network are trusted.
Routine
access-token expiry triggers unattended refresh; invalid or revoked renewable
credentials require reconnecting the account.

### Trusted worker environment initially

For the initial implementation, worker credential requests do not require worker
identity validation, API keys, or installation-membership checks. The deployment
assumes a trusted environment: callers able to reach the worker credential
endpoint can retrieve the installation's access credentials.

Workers still obtain usable access credentials through an orchestrator adapter
and supply them to Pi. This decision defers caller authentication only; provider
OAuth validation, credential expiry handling, and keeping refresh tokens on the
orchestrator remain part of the design.

Worker API-key authentication is the intended follow-up, outside this initial
implementation. Keep credential retrieval behind the adapter boundary so that
caller authentication can be added there without changing task definitions.

### One account per provider

Store at most one active account connection per provider for the installation.
Tasks select that connection through the provider namespace in `model_id`; no
named connections or additional task connection-reference field are needed.
Different providers can each have their own account connection.

A new login for a provider replaces its existing connection only after
authorization, credential validation, and durable persistence succeed. Cancelled
or failed login leaves the existing connection usable. Coordinate replacement
with refresh so an in-flight refresh of the old connection cannot overwrite the
new account. Subsequent credential requests use the replacement connection;
previously issued access tokens are not actively recalled.

### Existing environment-variable credentials

Keep `required_credentials` and their environment-variable behavior unchanged.
OAuth access credentials bypass that mapping and remain outside workflow YAML,
task outputs, conversation history, and logs. Tool credentials such as `gh_token`
continue through the existing path.

`openai/<model-id>` selects API authentication; `openai-codex/<model-id>` selects
subscription authentication. `OPENAI_API_KEY` does not authenticate Codex. Resolve
authentication for each task's provider, so one workflow can mix API-key and OAuth
tasks. Do not silently switch providers or API billing after an OAuth failure.

### Existing failure handling

Use RelayFold's existing task failure and retry behavior for missing or unusable
authorization, credential retrieval or refresh errors, and provider usage limits.
Propagate these errors through the existing failure result with a useful message
that does not expose credentials. Introduce no authentication-specific workflow
states, pause/resume flow, or new recovery modes. CLI login and persistence errors
use ordinary command failure reporting.

### Codex first and live developer verification

Implement Codex OAuth as the initial provider. Claude, Gemini, and custom OAuth
providers are outside the initial implementation. Users authorize their existing
provider account and subscription; no dedicated RelayFold user account is needed.

Provider OAuth compatibility is an implementation prerequisite verified live by
the developer, rather than an outstanding architectural choice. Verify the Rust
login, persisted credentials, worker translation, and a successful Pi Codex model
request, then repeat model execution after token renewal. This includes checking
the OAuth client, minimum scopes, token audience, endpoint, and account metadata.
Record any provider registration requirements discovered during verification.
Successful login alone does not establish inference compatibility.

### Local logout only

Logout removes the provider connection's stored OAuth credentials locally. It
does not request provider-side revocation, recall access tokens already supplied
to workers, or cancel running tasks. After removal, new credential requests fail
through existing failure handling until a successful login restores the connection.
Workers may continue using already-issued tokens while the provider accepts them.

Coordinate logout with credential issuance and refresh so a concurrent refresh
cannot recreate the removed connection. Report persistence failure as command
failure rather than claiming logout succeeded. The initial implementation has
no provider-side revocation flow; the exact logout command syntax remains to be
defined.

## Deployment scope

Multiple orchestrator replicas are outside the initial file-store deployment.
Supporting them requires shared durable storage and distributed coordination,
not independent token-file copies.

## Implementation sequence

1. **Define the Codex credential contract.** Establish Codex OAuth parameters for
   developer verification and the worker access-credential contract under the
   trusted-environment assumption. Resolve the single
   installation connection by provider; do not add connection-reference or token
   fields to task YAML.
2. **Implement Rust provider authentication and storage.** Define interfaces for
   provider login/refresh and file persistence. Implement Codex first, including
   PKCE, callback validation, cancellation, and headless completion. Retain the
   agreed provider fields without introducing Pi dependencies in the orchestrator.
3. **Implement CLI commands.** Move startup behind `start`, show help with no
   command, and provide `login`. Save credentials to the same store used by the
   server. Report storage failures explicitly and coordinate CLI changes with
   server refresh. Add status/disconnect using the agreed command syntax.
4. **Provide worker credential access.** Resolve the selected connection without
   caller validation in the trusted environment, centrally refresh when needed, persist
   replacements, and return only request credentials and necessary metadata.
   Prevent refresh from recreating a connection removed concurrently.
5. **Wire request-time resolution into Pi workers.** Put network access behind a
   worker interface, implement provider translation, and preserve model, tools,
   sessions, and existing environment-key authentication. Do not perform login
   or token refresh in workers.
6. **Update deployment.** Add a writable OAuth directory to the orchestrator and
   set `CMD ["start"]` in both Dockerfiles. Update launch scripts and ensure the
   non-root orchestrator UID can write files and locks. Keep worker OAuth-store
   mounts unnecessary and existing string credentials read-only.
7. **Verify and document.** Apply the testing scope below and update affected specs and
   website credentials, Agent-task, and installation documentation. Document
   remote sign-in, connection selection, storage, and reconnect behavior.

## Testing scope

For now, integration tests requiring complex handshakes are performed manually
by the developer. This includes browser authorization, remote callback/manual
completion, and live provider-to-Pi execution before and after token renewal.
Do not build automated end-to-end handshake harnesses for this implementation.

Automated coverage focuses only on critical unit and component tests: credential
translation and provider selection, expiry and refresh coordination, durable
replacement and logout races, and propagation through existing failure handling.
Use fake provider/network interfaces and temporary storage to verify these
behaviors without live accounts or browser sessions. Avoid tests that merely
mirror implementation details or duplicate existing coverage.

The scenarios below describe expected behavior, not a requirement to automate
every scenario. Record manual verification results, including any scenarios not
yet exercised, when implementing the integration.

## Validation scenarios

- Bare `orchestrator` prints help without server initialization; `start` retains
  server configuration and behavior. Default containers start the server.
- Login completes without Node.js, Pi, a connected worker, or a running server,
  including when the browser is on another host.
- Existing API-key tasks and tool credentials retain their environment behavior.
  A workflow mixing providers resolves authentication independently for each task.
- Provider token responses map correctly to Pi request credentials, including
  required account metadata; incompatible clients/audiences are not assumed valid.
- Each provider requests only scopes required for inference, unattended renewal,
  and provider-required account authorization; no unrelated scopes are requested.
- Multiple workers share one connection without separate login or worker refresh.
  Concurrent credential requests cause coordinated renewal and durable updates.
- Tasks from different namespaces use installation-wide connections without
  namespace-specific login or user-specific credential ownership.
- Tasks select the single account for their provider through `model_id`. Failed
  or cancelled replacement login preserves the old connection; successful login
  replaces it durably without an old refresh overwriting the new credentials.
- A long-running task obtains fresh access credentials for later model requests.
- Refresh survives orchestrator restart. CLI login/disconnect and server refresh
  cannot overwrite newer credentials or recreate a disconnected connection.
- Local logout durably removes credentials and blocks subsequent credential
  requests without provider revocation, token recall, or task cancellation.
- Worker credential retrieval works without an API key in the initial trusted
  environment. Refresh tokens remain on the orchestrator; logs and task state
  contain no tokens.
- Invalid authorization, subscription limits, network errors, and persistence
  failures use existing failure results and retry behavior, without new workflow
  states or silent fallback to API billing.
- The developer verifies Codex end to end using a live account, including model
  execution after token renewal; a model catalog alone is not proof of account
  entitlement or OAuth compatibility.

## Consequences and alternatives

RelayFold maintains provider-specific OAuth protocols in Rust, while keeping Pi
schema translation and model protocols in workers. This avoids an orchestrator
Node.js dependency and centralizes secrets and refresh coordination, but provider
authentication changes become RelayFold's maintenance responsibility.

A Pi-based helper would reuse Pi's login adapters but require Node.js or another
runtime deployment. A shared worker token file would reuse Pi persistence but
spread renewable credentials and refresh responsibility across workers. An
inference proxy could retain all tokens centrally but would also handle model
traffic. The chosen boundary returns access credentials and keeps direct worker
model calls; those workers must be trusted with the access they receive.

Subscription access remains subject to each provider's permitted integration,
account entitlements, and limits. Preserve API-key execution alongside it. Codex
is the initial implementation; support for Claude, Gemini, or custom OAuth
providers is a separate implementation and compatibility decision.
