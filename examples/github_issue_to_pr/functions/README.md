# GitHub Issue to PR Functions

`github-issue-to-pr.fetch_issue` fetches one issue using the GitHub REST API,
without an LLM or GitHub CLI. It requires `gh_token` and input
`{ "repository": "owner/repo", "issue_number": 7 }`.

The issue must have the exact `relayfold` label. Missing labels, pull requests,
GitHub errors, or incomplete comment pagination fail the task before implementation.
Issues labeled `relayfold:pr-created` also fail before comment fetching and implementation.
The labels are checked regardless of whether the issue is open or closed.

Output contains `repository`, `issue_number`, `issue_url`, `title`, uppercase
`state`, `body`, and `comments` (an array of comment bodies). The implementation
and review Agents interpret acceptance criteria from the original body and comments.
No generated summary or extracted acceptance-criteria field is needed.

`github-issue-to-pr.mark_pr_created` receives fetched issue data and the PR creation result in either input order. It requires matching repository and issue identifiers. When `pr_created` is true and `pr_number` is valid, it creates the repository label if needed, adds `relayfold:pr-created` without replacing existing labels, and confirms the marker in the response. With `pr_created: false`, it makes no GitHub calls. Output is `{ repository, issue_number, labeled }`. It requires `gh_token` with permission to create and apply labels.

The marker remains until manually removed. It prevents later pickups, but does not lock concurrent runs or detect unmarked existing PRs. If marking fails after PR creation, apply the label before retrying the full workflow. The final issue-comment task waits for this Function to succeed.

Requires Node.js 24 or newer. Build and test from this directory:

```bash
npm test
```

Tests use injected HTTP responses and never contact GitHub. Generated JSON and YAML
artifacts in `dist/` are ignored by Git. Register both Functions before the workflow:

```bash
export RELAYFOLD_URL=http://localhost:3000
curl -fsS -X POST "$RELAYFOLD_URL/function-def" \
  --data-binary @dist/github-issue-to-pr.fetch_issue.json
curl -fsS -X POST "$RELAYFOLD_URL/function-def" \
  --data-binary @dist/github-issue-to-pr.mark_pr_created.json
```
