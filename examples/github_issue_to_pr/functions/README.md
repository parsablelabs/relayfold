# GitHub Issue to PR Functions

`github-issue-to-pr.fetch_issue_and_pr` selects one eligible issue or PR update
using the GitHub REST API. Pass `{ "repository": "owner/repo" }` to scan open
issues labeled `relayfold:agent-ready` or `relayfold:pr-created`, oldest first.
Use `issue_number` to select an issue or pass both `issue_number` and `pr_number`
to adopt an existing same-repository PR. Use `relayfold:agent-ready` to queue
new work. Keep `relayfold:pr-created` on the issue for follow-up polling.

The fetch Function reads the status comment authored by the account associated
with `gh_token`. It can also find the PR in an older RelayFold summary from that
account or recover it from the `relayfold/issue-<number>` branch. It gathers unseen
discussion, inline comment, and submitted review feedback on open PRs, identifying
each item by its source and GitHub ID. Polling accepts feedback from
owners, organization members, and collaborators, excluding the token account
and bots. An idle poll returns `_workflow_exit_reason` and completes successfully.

`github-issue-to-pr.update_issue_status` writes the PR number and processed
feedback IDs into a status comment. It runs after successful publication and
updates the same comment on follow-up runs. The comment includes the change
summary and checks. If the issue needed no PR, it
records that outcome. Both Functions require a GitHub user token as `gh_token`.
Use a dedicated account for the token and keep the status comment between runs.

The shared `github.apply_labels` Function lives in [examples/functions](../../functions/README.md).
Register it separately before executing the workflow.

## Build and register

Requires Node.js 24 or newer. Run from this directory:

```bash
npm test
export RELAYFOLD_URL=http://localhost:3000
curl -fsS -X POST "$RELAYFOLD_URL/function-def" \
  --data-binary @dist/github-issue-to-pr.fetch_issue_and_pr.json
curl -fsS -X POST "$RELAYFOLD_URL/function-def" \
  --data-binary @dist/github-issue-to-pr.update_issue_status.json
```

Tests use mocked HTTP responses. The build writes registration artifacts to
`dist/`.
