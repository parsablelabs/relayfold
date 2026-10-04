---
title: GitHub Issue to Pull Request Workflow
description: Example agentic workflow that fetches a GitHub issue, implements a change, verifies it, and creates a pull request.
---

[`examples/example_github_issue_pr_workflow.yaml`](https://github.com/parsablelabs/relayfold/blob/main/examples/example_github_issue_pr_workflow.yaml)
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

The token must be able to read the target repository issue, push a branch, and create a pull request.

## Inputs

The first task requires one input object with the issue identifiers:

```json
{
  "repository": "parsablelabs/relayfold",
  "issue_number": 46
}
```

## Flow

<pre class="mermaid">
flowchart TD
    Input["Issue input: repository + issue_number"]
    Fetch["fetch-issue: read GitHub issue with gh"]
    Implement["implement-change: edit shared repo workspace"]
    Review{"review-implementation accepts?"}
    PR["create-pull-request: commit, push, open PR"]
    Done["Pull request ready"]
    Clarify["InputNeeded: ask for clarification"]
    Failed["Failed: missing credentials or unrecoverable error"]
    Input --> Fetch
    Fetch --> Implement
    Implement --> Review
    Review -->|continue with feedback, bounded| Implement
    Review -->|accepted| PR
    PR --> Done
    Implement -. underspecified .-> Clarify
    Fetch -. missing gh_token or gemini_api_key .-> Failed
    Implement -. task failure .-> Failed
    Review -. verifier failure .-> Failed
    PR -. gh failure .-> Failed
</pre>

1. `fetch-issue` uses `gh issue view` through the agent `bash` tool and returns structured issue details.
2. `implement-change` receives the issue details, updates the checkout in the shared `repo` workspace, runs relevant checks, and can pause for clarification when the issue is underspecified.
3. `review-implementation` checks the implementation against the issue criteria and test results. It can return `continue` with feedback, causing RelayFold to rerun from `implement-change` up to the bounded loop limit.
4. `create-pull-request` runs after the verifier accepts the implementation, commits and pushes the branch, and creates the PR with `gh pr create`.

All tasks use the same `workspace.group_name: repo` so files produced or edited by one step are visible to later steps.

## Register the workflow

Download and register the ready-to-run example directly from GitHub:

```bash
export RELAYFOLD_URL=http://localhost:3000

curl -fsSL https://raw.githubusercontent.com/parsablelabs/relayfold/main/examples/example_github_issue_pr_workflow.yaml \
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
