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

The token must be able to read the target repository issue and comments, push a branch, create a pull request, comment on the issue, and create and apply repository labels. The `fetch-issue` Function requires only `gh_token`; it does not use an LLM or the GitHub CLI.

## Inputs

The first task requires one input object with the issue identifiers:

```json
{
  "repository": "parsablelabs/relayfold",
  "issue_number": 46
}
```

The requested issue must have the exact `relayfold` label. Issues without that label and pull request numbers fail the fetch task before implementation begins. Issues labeled `relayfold:pr-created` also fail fetching before implementation begins. Open and closed issues are otherwise accepted.

## Flow

<pre class="mermaid">
flowchart TD
    Input["Issue input: repository + issue_number"]
    Fetch["fetch-issue: Function reads relayfold-labeled issue"]
    Implement["implement-change: edit shared repo workspace"]
    Review{"review-implementation accepts?"}
    PR["create-pull-request: commit, push, open PR"]
    Mark["mark-pr-created: Function adds relayfold:pr-created"]
    Comment["update-github-issue: final status comment"]
    Done["Completed"]
    Clarify["InputNeeded: ask for clarification"]
    Failed["Failed: missing credentials or unrecoverable error"]
    Input --> Fetch
    Fetch --> Implement
    Implement --> Review
    Review -->|continue with feedback, bounded| Implement
    Review -->|accepted| PR
    PR --> Mark
    Mark --> Comment
    Comment --> Done
    Implement -. underspecified .-> Clarify
    Fetch -. missing label, gh_token, or API failure .-> Failed
    Implement -. task failure .-> Failed
    Review -. verifier failure .-> Failed
    PR -. gh failure .-> Failed
</pre>

1. `fetch-issue` calls `github-issue-to-pr.fetch_issue`, a deterministic Function that reads the issue and all comment pages through the [GitHub REST API](https://docs.github.com/en/rest/issues/issues#get-an-issue). It requires `relayfold`, rejects `relayfold:pr-created`, and returns `repository`, `issue_number`, `issue_url`, `title`, `state`, `body`, and `comments` (comment bodies). It makes no LLM calls. The implementation and review Agents interpret acceptance criteria from the original body and comments.
2. `implement-change` receives the issue details, updates the checkout in the shared `repo` workspace, runs relevant checks, and can pause for clarification when the issue is underspecified.
3. `review-implementation` checks the implementation against the issue criteria and test results. It can return `continue` with feedback, causing RelayFold to rerun from `implement-change` up to the bounded loop limit.
4. `create-pull-request` runs after the verifier accepts the implementation, commits and pushes the branch, and creates the PR with `gh pr create`. The PR body includes a full link to the issue it addresses, a change summary, and test results. The Agent checks the published body and adds the issue link if missing, preserving existing content and avoiding duplicate links.
5. `mark-pr-created` calls `github-issue-to-pr.mark_pr_created`. When `pr_created` is true, it adds `relayfold:pr-created` to the issue, creating the repository label if missing. It uses the [add-labels endpoint](https://docs.github.com/en/rest/issues/labels#add-labels-to-an-issue) to preserve existing labels and verifies the response includes the marker. When no PR was created, it returns `labeled: false` without GitHub calls.
6. `update-github-issue` waits for labeling to finish and adds the final status comment. A labeling failure stops the workflow before this comment.

The marker remains after the PR is closed or merged. Remove it deliberately when you want another implementation attempt. This guard prevents later runs from picking up marked issues; it does not lock concurrent runs or discover existing PRs that have no marker. If labeling fails after PR creation, apply the marker before starting another full workflow run.

All Agent tasks use the same `workspace.group_name: repo` so files produced or edited by one step are visible to later steps.

## Register the workflow

First build, test, and register both Functions from a local checkout (Node.js 24 or newer; no npm dependencies are required):

```bash
cd examples/github_issue_to_pr/functions
npm test
export RELAYFOLD_URL=http://localhost:3000
curl -fsS -X POST "$RELAYFOLD_URL/function-def" \
  --data-binary @dist/github-issue-to-pr.fetch_issue.json
curl -fsS -X POST "$RELAYFOLD_URL/function-def" \
  --data-binary @dist/github-issue-to-pr.mark_pr_created.json
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
curl -fsS -X POST "$RELAYFOLD_URL/workflow-def/github-issue-pr-workflow" \
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
