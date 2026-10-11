# GitHub Issue to PR Functions

`github-issue-to-pr.fetch_issue_and_pr` fetches one issue using the GitHub REST API,
without an LLM or GitHub CLI. It requires `gh_token` and input
`{ "repository": "owner/repo", "issue_number": 7 }`.

The issue must have the exact `relayfold:agent-ready` label. Without it, the
Function returns `_workflow_exit_reason`, completing the workflow before implementation.
Pull request numbers, GitHub errors, or incomplete pagination fail the task.
Open and closed issues are accepted. Neither `relayfold` nor
`relayfold:human-input-needed` is required or checked; clarification uses `ask_user`
inside the implementation workflow. `relayfold:pr-created` is informational.

Successful fetch output contains `repository`, `issue_number`, `issue_url`,
`title`, uppercase `state`, `body`, and `comments` (all issue comment bodies).
Implementation and review interpret the issue's acceptance criteria.

## Continue an existing pull request

The Function resolves a PR from its same-repository URL in the latest issue
summary marked `<!-- relayfold:issue-to-pr -->`. Older issues can use an
unambiguous PR link in their comments or an explicit input:
`{ "repository": "owner/repo", "issue_number": 7, "pr_number": 12 }`.
Multiple candidate links, or a `relayfold:pr-created` issue without a link,
fail with instructions to supply `pr_number`; no replacement PR is created.
PRs without a summary link must be supplied explicitly.

The optional `pr` object contains `pr_number`, `pr_url`, `title`, `body`, uppercase
`state`, `branch`, and all pages of conversation `comments`, inline
`review_comments`, and `reviews`. The separate `change_request` object contains
`body` and `url` from the latest nonempty developer conversation comment on the PR.
Bot comments and marked RelayFold summaries are excluded. A PR without developer
feedback fails fetching; post the request before starting the run. Inline review
comments and reviews provide context but are not the change-request trigger.

Implementation checks out the existing PR branch and focuses on `change_request`,
preserving existing work and using older discussion as context. Missing information
or a closed/merged PR that prevents continuation is resolved through `ask_user`.
Publication pushes to the same PR and returns `pr_created: false` for reuse.

## Summary and readiness

`github-issue-to-pr.update_issue` receives fetched issue, implementation, and
publication outputs. It posts one issue comment headed **RelayFold — PR created**,
**RelayFold — PR updated**, or **RelayFold — No PR needed**, with the stable marker,
change summary, checks, PR link, and request link when available. It rejects
unresolved `open_questions` and inconsistent identifiers before posting.

The shared `github.apply_labels` Function in
[`examples/functions`](../../functions/README.md) runs only after summary success.
It retains/adds `relayfold:pr-created` for a new or reused PR, then removes
`relayfold:agent-ready`. Successful no-work runs also consume readiness. Failures
before completion leave readiness present. To request another update, post a new
PR conversation comment, then reapply `relayfold:agent-ready` to the issue and
start the workflow. Avoid overlapping runs for the same issue; labels are not locks.

## Build and register

Requires Node.js 24 or newer. Build and test from this directory:

```bash
npm test
```

Tests use injected HTTP responses and never contact GitHub. Generated JSON and YAML
artifacts in `dist/` are ignored by Git. Register both Functions and the shared
label Function before registering the updated workflow:

```bash
export RELAYFOLD_URL=http://localhost:3000
curl -fsS -X POST "$RELAYFOLD_URL/function-def" \
  --data-binary @dist/github-issue-to-pr.fetch_issue_and_pr.json
curl -fsS -X POST "$RELAYFOLD_URL/function-def" \
  --data-binary @dist/github-issue-to-pr.update_issue.json
```

Previously labeled issues need `relayfold:agent-ready` to run. Legacy
`relayfold:human-input-needed` labels can be removed manually; they have no effect
on this workflow.

If PR publication succeeds but summary posting fails, supply that PR’s `pr_number`
when starting another run so it continues the published branch.
