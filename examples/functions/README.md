# Shared example Functions

Reusable Functions used by multiple example workflows.

`github.apply_labels` is shared by the issue-to-PR and CloudWatch workflows. In the PR workflow it receives fetched issue data, the PR result, and implementation output. It adds `relayfold:pr-created` when `pr_created` is true and `pr_number` is valid; unresolved `open_questions` add `relayfold:human-input-needed`. Repository and issue identifiers must match.

In the CloudWatch workflow it receives the publisher's `created` entries, each with `repository`, `issue_number`, and `human_input_needed`. It adds the human-input label only to flagged new issues. Empty publication results, dry runs, and requests needing neither label make no GitHub calls.

The Function creates missing repository labels, adds labels without replacing existing ones, and confirms the response. Output is `{ applied: [{ repository, issue_number, labels }] }`. It requires `gh_token` with permission to create and apply labels.

For human-input issues, add the missing information in an issue comment and remove `relayfold:human-input-needed`. Fetching then includes that comment for implementation and review.

The marker remains until manually removed. It prevents later pickups, but does not lock concurrent runs or detect unmarked existing PRs. If marking fails after PR creation, apply the label before retrying the full workflow. The final issue-comment task waits for this Function to succeed.

Requires Node.js 24 or newer; no npm dependencies are needed. From this directory:

```bash
npm test
```

Tests use injected GitHub responses and build the issue-to-PR fetch artifact for the integration checks. They never contact GitHub.
Generated JSON and YAML artifacts in `dist/` are ignored by Git.

Register the shared Function before executing either workflow:

```bash
export RELAYFOLD_URL=http://localhost:3000
curl -fsS -X POST "$RELAYFOLD_URL/function-def" \
  --data-binary @dist/github.apply_labels.json
```
