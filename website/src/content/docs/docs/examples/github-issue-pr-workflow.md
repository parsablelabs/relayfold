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
`repository: parsablelabs/relayfold`. Replace this with your repository before
starting a run.

The first task requires only a repository:

```json
{
  "repository": "parsablelabs/relayfold"
}
```

When `issue_number` is omitted, the fetch Function reads all pages of open issues
with the `relayfold:agent-ready` label and selects the oldest eligible issue by
creation time, skipping pull requests. It rechecks the selected issue's state and
labels before fetching comments. If none qualify, the workflow completes
successfully through early exit without running implementation or publication.
The fetch result contains `repository` and `_workflow_exit_reason`, without
issue metadata. Listing errors and incomplete pagination still fail the fetch step. Selection uses the
[repository issues API](https://docs.github.com/en/rest/issues/issues#list-repository-issues).

Add `"issue_number": 46` to choose a specific issue. Supplying `pr_number` also
requires `issue_number`. Explicit selection accepts open and closed issues;
automatic selection accepts only open issues. Pull request numbers fail fetching.

Apply `relayfold:agent-ready` to the issue before starting the workflow. If the
selected issue no longer has that label, fetching returns an exit reason and the
workflow completes before implementation. `fetch-issue-and-pr` enables
`control.allow_early_exit: true`; RelayFold adds the optional reason field to its
output schema. When a selected issue loses readiness, the exit result retains issue metadata with an empty comments array,
and comments are not fetched. The old `relayfold` and
`relayfold:human-input-needed` labels have no effect on eligibility.
`relayfold:pr-created` identifies prior PR publication but does not block a ready
issue. Automatic selection does not claim or lock an issue; avoid concurrent runs
for the same repository to prevent duplicate work.

## Flow

The issue summary receives the fetched issue, implementation results, and PR
publication results. Labeling waits for the summary to be posted successfully.

Repository-only selection chooses the oldest open issue with
`relayfold:agent-ready`; old status labels do not exclude ready issues. Existing
PRs can be resolved from issue summary links. The latest developer feedback from
PR conversation comments, inline comments, or submitted reviews supplies
`change_request`; bots, pending reviews, and marked automation summaries are excluded.

<pre class="mermaid">
flowchart TD
    Input["Issue input: repository + optional issue_number"]
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
    Fetch -->|no eligible issue or no agent-ready: exit| Done
    Implement -. underspecified .-> Clarify
    Fetch -. invalid input, missing gh_token, or API failure .-> Failed
    Implement -. task failure .-> Failed
    Review -. verifier failure .-> Failed
    PR -. gh failure .-> Failed
</pre>

1. `fetch-issue-and-pr` calls `github-issue-to-pr.fetch_issue_and_pr`, a deterministic Function that checks `relayfold:agent-ready`, reads all issue comments, and resolves an existing PR from the issue summary link or explicit `pr_number`. For an existing PR, it fetches its discussion and reviews and selects the latest eligible developer feedback from conversation comments, inline comments, or submitted reviews as `change_request`. It makes no LLM calls.
2. `implement-change` edits the shared `repo` workspace. For fresh issues it implements the issue criteria; for existing PRs it checks out the PR branch and focuses on `change_request`, preserving existing work. Missing information pauses the run through `ask_user` and resumes with the human's answer.
3. `review-implementation` checks the issue criteria, requested update, and test results. It can return `continue` with feedback, rerunning from `implement-change` within the bounded loop limit.
4. `create-pull-request` commits and pushes after verification. It creates a PR for a fresh issue or updates the existing PR branch. New PR bodies include the issue URL, change summary, and tests; the Agent verifies the published issue link.
5. `update-github-issue` calls `github-issue-to-pr.update_issue` to post a summary headed **RelayFold — PR created**, **RelayFold — PR updated**, or **RelayFold — No PR needed**. It includes a stable hidden `<!-- relayfold:issue-to-pr -->` marker, the change summary, tests, PR link, and instruction-comment link when available. This distinguishes automation summaries from developer requests. Unresolved questions prevent completion.
6. `apply-labels` calls `github.apply_labels` only after the summary succeeds. It adds or retains `relayfold:pr-created` for a new or reused PR, then removes `relayfold:agent-ready` without replacing other labels. Successful no-work runs also consume readiness. Failures before completion retain the trigger for retry.

If review finds unsupported assumptions about critical behavior or scope, it rejects the implementation with feedback naming the assumptions, affected files and behavior, missing requirements, and specific questions for the human. On retry, `implement-change` calls `ask_user` with those questions before revising the affected code. The workflow pauses for human input, then implementation uses the answers while preserving earlier clarifications and accepted corrections. For example, if an issue asks for retries without specifying which failures are retryable, review can ask implementation to clarify that decision with the human instead of choosing a retry policy itself. These retries use the existing bounded verifier loop.

To request another update, post the requested changes in a PR conversation comment, inline comment, or submitted review, reapply `relayfold:agent-ready` to the issue, and start the workflow. Bot comments, pending reviews, and marked RelayFold summaries are excluded when selecting feedback. Labels do not lock concurrent runs; finish the current run before requesting the next update. Existing human-input labels can be removed manually, but they no longer block the workflow.

`implement-change` and `review-implementation` use `thinking_level: medium`; `create-pull-request` uses `low`. Fetching, summary publication, and labeling are Functions. See [Agent reasoning level](/relayfold/docs/concepts/tasks/agents/#reasoning-level) for model support and session behavior.

Agent output structure comes from the effective output schema supplied by the
worker. Prompts describe field meanings and task decisions without duplicating
JSON output examples.

All Agent tasks use the same `workspace.group_name: repo` so files produced or edited by one step are visible to later steps.

Only the three Agent tasks (`implement-change`, `review-implementation`, and
`create-pull-request`) enable `allow_agent_feedback: true` to report optional
environment and setup observations in `_agent_provided_feedback`. Inspect this
field in each task attempt's output; it is separate from verifier retry feedback
and does not affect workflow control. See
[Agent feedback](/relayfold/docs/concepts/workflow-yaml/#agent-feedback).

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

If `apply-labels` fails output schema validation, rebuild and replace the shared
label Function. Older artifacts return only `applied`; the current workflow also
requires `removed` to confirm consumption of `relayfold:agent-ready`. Updating
workflow YAML does not update registered Function code:

```bash
npm test --prefix examples/functions
curl -fsS -X POST "$RELAYFOLD_URL/function-def?overwrite=true" \
  --data-binary @examples/functions/dist/github.apply_labels.json
```

Run these commands from the repository root, then start a new workflow run.

## Execute the workflow

Replace the repository with the repository whose next eligible issue you want
the workflow to implement:

```bash
curl -fsS -X POST "$RELAYFOLD_URL/workflow-def/github-issue-to-pr" \
  -H 'content-type: application/json' \
  -d '{
    "repository": "parsablelabs/relayfold"
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
link must be supplied explicitly. A PR without nonempty developer feedback
also fails; post the requested changes before starting the run.

The fetch result includes `pr` with its identifiers, body, branch, state, and all
conversation comments, inline comments, and reviews. `change_request` contains the
latest nonempty developer feedback's body and URL. Selection uses comment
creation times and review submission times; bots, pending reviews, and marked
RelayFold summaries are excluded. Implementation and review
focus on that request while using the issue criteria and older discussion as
context, preserving work that already addresses earlier requests. Feedback with
no actionable request or conflicting instructions requiring a critical decision
triggers clarification through `ask_user`. Verifier retries preserve ongoing work on the same PR branch. Publication
pushes to that PR and returns `pr_created: false` for reuse. A closed or merged PR
that prevents continuation requires clarification through `ask_user`.

If PR publication succeeds but summary posting fails, supply that PR’s `pr_number`
when starting another run so it continues the published branch.
