# GitHub Issue to PR Functions

`github-issue-to-pr.fetch_issue_and_pr` fetches one issue using the GitHub REST API,
without an LLM or GitHub CLI. It requires `gh_token` and input
`{ "repository": "owner/repo", "issue_number": 7 }`.

The issue must have the exact `relayfold` label. Missing labels, pull requests,
GitHub errors, or incomplete comment pagination fail the task before implementation.
Issues labeled `relayfold:pr-created` or `relayfold:human-input-needed` also fail before comment fetching and implementation.
The labels are checked regardless of whether the issue is open or closed.

Output contains `repository`, `issue_number`, `issue_url`, `title`, uppercase
`state`, `body`, and `comments` (an array of comment bodies). The implementation
and review Agents interpret acceptance criteria from the original body and comments.
No generated summary or extracted acceptance-criteria field is needed.

The shared `github.apply_labels` Function lives in [`examples/functions`](../../functions/README.md). Build and register it separately before executing the workflow.

Requires Node.js 24 or newer. Build and test from this directory:

```bash
npm test
```

Tests use injected HTTP responses and never contact GitHub. Generated JSON and YAML
artifacts in `dist/` are ignored by Git. Register the fetch Function before the workflow:

```bash
export RELAYFOLD_URL=http://localhost:3000
curl -fsS -X POST "$RELAYFOLD_URL/function-def" \
  --data-binary @dist/github-issue-to-pr.fetch_issue_and_pr.json
```

### Continue an existing pull request

Add optional `pr_number` to the input, for example
`{ "repository": "owner/repo", "issue_number": 7, "pr_number": 12 }`.
The PR must belong to the input repository. The fetch Function includes an optional
`pr` object with `pr_number`, `pr_url`, `title`, `body`, uppercase `state`,
`branch`, and all pages of `comments` (conversation bodies), `review_comments`
(inline review bodies), and `reviews` (review summary bodies). GitHub errors or
incomplete pagination fail the task rather than passing partial context.
See GitHub's [review comments API](https://docs.github.com/en/rest/pulls/comments)
and [issue comments API](https://docs.github.com/en/rest/issues/comments).

Implementation checks out the supplied PR and uses its discussion and reviews as
context. Review receives the same context, and publication pushes to the existing
branch without creating a duplicate PR. The final issue comment links to the reused
PR. `pr_created` remains false for a reused PR, so reuse does not add the
`relayfold:pr-created` label. Existing issue-label guards still apply: remove a
blocking label deliberately before rerunning. Closed or merged PR details can be
fetched; the Agents must inspect their state before deciding how to proceed.
