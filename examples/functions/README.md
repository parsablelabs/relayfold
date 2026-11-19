# Shared example Functions

`github.apply_labels` completes the issue-to-PR workflow's label lifecycle.
It receives fetched issue data, the PR publication result, and a successful issue
summary (`commented: true`). Repository, issue, and PR identifiers must match.

For a new or reused PR it ensures `relayfold:pr-created` exists and adds it without
replacing other issue labels. This label is informational. After that succeeds,
it removes `relayfold:agent-ready`, preventing further processing until a developer
reapplies it. Successful no-work runs remove readiness without adding a PR marker.
Missing summary evidence or marker failures cause no readiness removal; removal
failures fail the task. Repeated removal confirms that readiness is absent.

Output is `{ applied: [{ repository, issue_number, labels }], removed:
[{ repository, issue_number, labels }] }`. It requires `gh_token` with permission
to create, apply, and remove labels. Human clarification uses the workflow's
`ask_user` flow; this Function does not manage `relayfold:human-input-needed`.
The CloudWatch workflow publishes its origin and optional bug labels directly and
no longer calls this Function.

Requires Node.js 24 or newer; no npm dependencies are needed. From this directory:

```bash
npm test
```

Tests use injected GitHub responses and build the issue-to-PR fetch artifact for
integration checks. They never contact GitHub. Generated artifacts in `dist/`
are ignored by Git.

Register the Function before executing the issue-to-PR workflow:

```bash
export RELAYFOLD_URL=http://localhost:3000
curl -fsS -X POST "$RELAYFOLD_URL/function-def" \
  --data-binary @dist/github.apply_labels.json
```
