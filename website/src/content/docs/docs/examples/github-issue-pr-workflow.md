---
title: GitHub Issue to Pull Request Workflow
description: Example agentic workflow that fetches a GitHub issue, implements a change, verifies it, and creates a pull request.
---

[`examples/github_issue_to_pr/example_github_issue_pr_workflow.yaml`](https://github.com/parsablelabs/relayfold/blob/main/examples/github_issue_to_pr/example_github_issue_pr_workflow.yaml)
demonstrates an agentic workflow that fetches a GitHub issue, implements the requested change, uses a verifier loop to review the implementation, and creates a pull request.

## Worker image tooling

The production and development worker images install the GitHub CLI package (`gh`) alongside `bash`, `curl`, and `git`.

Tasks that call `gh` list `gh_token` in `required_credentials`:

```yaml
required_credentials:
  - gemini_api_key
  - gh_token
```

Add `gh_token` to the worker credential file together with `gemini_api_key`:

```json
{
  "gemini_api_key": "...",
  "gh_token": "github_pat_..."
}
```

The token must be able to read the target repository issue and comments, push a branch, create a pull request, comment on the issue, and create and apply repository labels. The `fetch-issue-and-pr` Function requires only `gh_token`; it does not use an LLM or the GitHub CLI.

## Inputs

The workflow's `example_input` prefills the UI JSON trigger input with
`repository: parsablelabs/relayfold` and `issue_number: 46`. Replace these with
the repository and issue you want to implement before starting a run.

The first task requires one input object with the issue identifiers:

```json
{
  "repository": "parsablelabs/relayfold",
  "issue_number": 46
}
```

Apply `relayfold:agent-ready` to the issue before starting the workflow. Without that label, fetching returns an exit reason and the workflow completes before implementation. Pull request numbers fail fetching. Open and closed issues are accepted. The old `relayfold` and `relayfold:human-input-needed` labels have no effect on eligibility. `relayfold:pr-created` remains an informational status label.

## Flow

<pre class="mermaid">
flowchart TD
    Input["Issue input: repository + issue_number"]
    Fetch["fetch-issue-and-pr: Function checks agent-ready and resolves PR"]
    Implement["implement-change: edit shared repo workspace"]
    Review{"review-implementation accepts?"}
    PR["create-pull-request: commit, push, open PR"]
    Mark["apply-labels: Function marks PR and removes agent-ready"]
    Comment["update-github-issue: Function posts marked change summary"]
    Done["Completed"]
    Clarify["InputNeeded: ask for clarification"]
    Failed["Failed: missing credentials or unrecoverable error"]
    Input --> Fetch
    Fetch --> Implement
    Implement --> Review
    Review -->|continue with feedback, bounded| Implement
    Review -->|accepted| PR
    PR --> Comment
    Comment --> Mark
    Mark --> Done
    Fetch -->|no agent-ready: exit| Done
    Implement -. underspecified .-> Clarify
    Fetch -. invalid input, missing gh_token, or API failure .-> Failed
    Implement -. task failure .-> Failed
    Review -. verifier failure .-> Failed
    PR -. gh failure .-> Failed
</pre>

1. `fetch-issue-and-pr` calls `github-issue-to-pr.fetch_issue_and_pr`, a deterministic Function that checks `relayfold:agent-ready`, reads all issue comments, and resolves an existing PR from the issue summary link or explicit `pr_number`. For an existing PR, it fetches its discussion and reviews and selects the latest developer conversation comment as `change_request`. It makes no LLM calls.
2. `implement-change` edits the shared `repo` workspace. For fresh issues it implements the issue criteria; for existing PRs it checks out the PR branch and focuses on `change_request`, preserving existing work. Missing information pauses the run through `ask_user` and resumes with the human's answer.
3. `review-implementation` checks the issue criteria, requested update, and test results. It can return `continue` with feedback, rerunning from `implement-change` within the bounded loop limit.
4. `create-pull-request` commits and pushes after verification. It creates a PR for a fresh issue or updates the existing PR branch. New PR bodies include the issue URL, change summary, and tests; the Agent verifies the published issue link.
5. `update-github-issue` calls `github-issue-to-pr.update_issue` to post a summary headed **RelayFold — PR created**, **RelayFold — PR updated**, or **RelayFold — No PR needed**. It includes a stable hidden `<!-- relayfold:issue-to-pr -->` marker, the change summary, tests, PR link, and instruction-comment link when available. This distinguishes automation summaries from developer requests. Unresolved questions prevent completion.
6. `apply-labels` calls `github.apply_labels` only after the summary succeeds. It adds or retains `relayfold:pr-created` for a new or reused PR, then removes `relayfold:agent-ready` without replacing other labels. Successful no-work runs also consume readiness. Failures before completion retain the trigger for retry.

If review finds unsupported assumptions about critical behavior or scope, it rejects the implementation with feedback naming the assumptions, affected files and behavior, missing requirements, and specific questions for the human. On retry, `implement-change` calls `ask_user` with those questions before revising the affected code. The workflow pauses for human input, then implementation uses the answers while preserving earlier clarifications and accepted corrections. For example, if an issue asks for retries without specifying which failures are retryable, review can ask implementation to clarify that decision with the human instead of choosing a retry policy itself. These retries use the existing bounded verifier loop.

To request another update, post the requested changes in a PR conversation comment, reapply `relayfold:agent-ready` to the issue, and start the workflow. Bot comments and marked RelayFold summaries are excluded when selecting feedback. Inline comments and review summaries provide context but do not select the request. Labels do not lock concurrent runs; finish the current run before requesting the next update. Existing human-input labels can be removed manually, but they no longer block the workflow.

`implement-change` and `review-implementation` use `thinking_level: medium`; `create-pull-request` uses `low`. Fetching, summary publication, and labeling are Functions. See [Agent reasoning level](/relayfold/docs/concepts/tasks/agents/#reasoning-level) for model support and session behavior.

All Agent tasks use the same `workspace.group_name: repo` so files produced or edited by one step are visible to later steps.

## Register the workflow

First build, test, and register the fetch and summary Functions and shared labeling Function from a local checkout (Node.js 24 or newer; no npm dependencies are required):

```bash
cd examples/github_issue_to_pr/functions
npm test
export RELAYFOLD_URL=http://localhost:3000
curl -fsS -X POST "$RELAYFOLD_URL/function-def" \
  --data-binary @dist/github-issue-to-pr.fetch_issue_and_pr.json
curl -fsS -X POST "$RELAYFOLD_URL/function-def" \
  --data-binary @dist/github-issue-to-pr.update_issue.json
cd ../../functions
npm test
curl -fsS -X POST "$RELAYFOLD_URL/function-def" \
  --data-binary @dist/github.apply_labels.json
cd ../..
```

Then download and register the workflow directly from GitHub:

```bash
export RELAYFOLD_URL=http://localhost:3000

curl -fsSL https://raw.githubusercontent.com/parsablelabs/relayfold/main/examples/github_issue_to_pr/example_github_issue_pr_workflow.yaml \
  | curl -fsS -X POST "$RELAYFOLD_URL/workflow-def" \
      --data-binary @-
```

## Execute the workflow

Replace the repository and issue number with the issue you want the workflow to
implement:

```bash
curl -fsS -X POST "$RELAYFOLD_URL/workflow-def/github-issue-to-pr" \
  -H 'content-type: application/json' \
  -d '{
    "repository": "parsablelabs/relayfold",
    "issue_number": 46
  }'
```

## Check the output

After the workflow completes, replace `<workflow_id>` with the `id` returned
when you executed it and read the final issue-update result:

```bash
curl -fsS "$RELAYFOLD_URL/workflows/<workflow_id>/tasks/update-github-issue"
```

For a recurring workflow that analyzes production logs and files issues, see
[FillMyFunnel CloudWatch Review](/relayfold/docs/examples/fillmyfunnel-cloudwatch-workflow/).

### Continue an existing pull request

Apply `relayfold:agent-ready` to the issue after posting feedback on its PR. The
fetch Function resolves the PR from its same-repository link in the latest marked
RelayFold issue summary. For legacy issues, an unambiguous PR link in issue
comments is accepted. You can supply `pr_number` explicitly:

```json
{
  "repository": "owner/repo",
  "issue_number": 7,
  "pr_number": 12
}
```

Multiple candidate links, or a `relayfold:pr-created` issue without a resolvable
link, fail with instructions to supply `pr_number`. PRs without an issue summary
link must be supplied explicitly. A PR without a nonempty developer conversation
comment also fails; post the requested changes before starting the run.

The fetch result includes `pr` with its identifiers, body, branch, state, and all
conversation comments, inline comments, and reviews. `change_request` contains the
latest developer conversation comment's body and URL. Implementation and review
focus on that request while using the issue criteria and older discussion as
context. Verifier retries preserve ongoing work on the same PR branch. Publication
pushes to that PR and returns `pr_created: false` for reuse. A closed or merged PR
that prevents continuation requires clarification through `ask_user`.

If PR publication succeeds but summary posting fails, supply that PR’s `pr_number`
when starting another run so it continues the published branch.
