# Early Adoption Security Plan

## Context

RelayFold should begin attracting real users and learning from their workflows
without attempting to implement every security feature expected by a large
enterprise. The near-term goal is a credible, secure-by-default deployment for
small teams running trusted workflows in a controlled environment.

This is not an enterprise-security roadmap. It identifies inexpensive controls
that avoid obvious risks while leaving larger investments to be guided by
adopter needs.

## Recommended Initial Security Baseline

### Public API authentication

Implement real API-key authentication in place of the deferred namespace
resolver. API keys should map requests to the correct namespace and should be
revocable and handled without appearing in logs or errors.

Unauthenticated global-namespace operation should be clearly designated as a
development mode. Production-oriented documentation and configuration should
not enable it by default.

Full OIDC, SAML, enterprise RBAC, and identity-provider integrations should be
deferred until real deployments establish their requirements.

### Secure network defaults

RelayFold services should expose only the interfaces that an operator
intentionally configures:

- Bind to localhost or a private interface by default where practical.
- Do not publish the worker API port in the default deployment unless it is
  required by the selected topology.
- Document the worker API as a trusted internal boundary that must not be
  exposed to the public internet.
- Consider a startup warning when the worker API is broadly exposed without
  authentication.

### Worker authentication

Shared-token authentication between the orchestrator and workers is not a
critical requirement for the initial release when both services are reliably
deployed on the same isolated network.

A shared token would protect against accidental exposure and unrelated
workloads reaching the worker API, but it would not provide strong isolation
between workers because every worker would hold the same credential.

Worker authentication can therefore be deferred while all of the following are
true:

- The worker API is reachable only through a trusted private network.
- Workers are operated by the same trusted organization.
- Untrusted workloads do not share that network boundary.
- RelayFold clearly documents that the worker API is unauthenticated and must
  not be exposed publicly.

Revisit service authentication, preferably with distinct worker identities or
mTLS rather than only a shared secret, when RelayFold supports workers across
networks, shared Kubernetes clusters, untrusted neighboring workloads, remote
workers, or deployments with explicit service-authentication requirements.

### Credential and log safety

Apply inexpensive protections around the existing credential model:

- Validate credential-file permissions and warn or fail when they are unsafe.
- Redact known credentials from logs, errors, workflow events, and task results.
- Encourage narrowly scoped credentials for every task.
- Add request-size limits and sensible task timeouts.
- Avoid claiming that current credential handling is suitable for hostile task
  code.

Vault, cloud secret-manager integrations, dynamic credentials, and detailed
secret-access policy should be driven by adopter demand rather than built
preemptively.

### Accurate execution trust boundaries

RelayFold currently provides a selected workspace as the intended location for
task work but does not guarantee filesystem containment for arbitrary Function
or Agent code. Early documentation should state this plainly:

- Workflow authors and task code are trusted.
- Workers can execute arbitrary code.
- Workers should run in a private, controlled environment.
- Task credentials should have the minimum necessary scope.
- RelayFold does not yet provide hostile-code or per-task sandbox isolation.

Container hardening, network egress policies, dependency controls, and stronger
per-task isolation remain important future work, but should be prioritized
against concrete adopter scenarios.

## Community Adoption Focus

### A single flagship use case

Position RelayFold around one clear initial value proposition:

> Build reliable, observable Agent workflows in YAML, with resumable execution
> and human approval.

Polish one end-to-end developer workflow rather than adding many shallow
examples. A strong candidate is:

1. Analyze a GitHub issue.
2. Modify a repository.
3. Run validation.
4. Pause for human approval.
5. Open a pull request.

The target should be a first successful execution within approximately ten
minutes:

1. Install RelayFold.
2. Configure one model credential.
3. Run the flagship workflow.
4. Inspect task results and workflow events.
5. Modify one task and execute it again.

### Early-adopter expectations

Publish a short security and deployment status that explains:

- RelayFold is suitable for evaluation and trusted-code deployments.
- It should run inside a private network.
- Workflow authors must be trusted.
- Workers can execute arbitrary code.
- Credentials should be narrowly scoped.
- Strong multi-user authorization and hostile-code isolation are not yet
  available.

Also provide a `SECURITY.md`, a private vulnerability-reporting path, focused
issue templates, and a short public roadmap without promised delivery dates.

## Suggested Release Scope

Prioritize the following work:

1. Implement public API-key authentication and namespace resolution.
2. Establish private network and service-binding defaults.
3. Add credential redaction and credential-file permission checks.
4. Publish `SECURITY.md` and clear deployment trust-boundary documentation.
5. Produce one polished ten-minute quickstart.
6. Produce one flagship GitHub-to-pull-request workflow.
7. Recruit a small group of design partners and use their experience to select
   the next investment.

Worker shared-token authentication is intentionally not part of this minimum
scope when the worker API remains on a trusted isolated network.

## Learning Goals

Early adoption should answer:

- Can a new user complete a workflow without maintainer assistance?
- Do users run or create a second workflow?
- Which workflows do they want to automate?
- Where do executions fail or become difficult to understand?
- Which security requirements arise from actual deployment environments?
- Do users return to RelayFold after their first evaluation?

These findings should determine whether subsequent work emphasizes
integrations, authoring, observability, reliability, identity, secrets
management, or stronger workload isolation.
