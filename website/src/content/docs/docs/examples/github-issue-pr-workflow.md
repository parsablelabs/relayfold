---
title: GitHub Issue to Pull Request Workflow
description: Example agentic workflow that fetches a GitHub issue, implements a change, verifies it, and creates a pull request.
---

[`examples/github_issue_to_pr/example_github_issue_pr_workflow.yaml`](https://github.com/parsablelabs/relayfold/blob/main/examples/github_issue_to_pr/example_github_issue_pr_workflow.yaml)
demonstrates an agentic workflow that fetches a GitHub issue, implements the requested change, uses a verifier loop to review the implementation, and creates a pull request.

## Worker setup

The production and development worker images install the GitHub CLI package (`gh`) alongside `bash`, `curl`, and `git`.

The implementation, review, and PR publication tasks use GPT-6.1 Sol through `openai-codex/gpt-6.1-sol`.
Connect your Codex account with `orchestrator login openai-codex`. See
[Codex subscription sign-in](/relayfold/docs/operations/credentials/#codex-subscription-sign-in).

Add `gh_token` for GitHub access and the example's required `gemini_api_key`
entry to the worker credential file:

```json
{
  "gemini_api_key": "...",
  "gh_token": "github_pat_..."
}
```

Use a GitHub user token that can read the authenticated account, issues, PRs, and comments, push a branch, create a PR, post and edit its issue status comment, and create and apply repository labels. The `fetch-issue-and-pr` Function requires only `gh_token`; it does not use an LLM or the GitHub CLI.

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

With repository-only input, RelayFold scans open issues labeled
`relayfold:agent-ready` or `relayfold:pr-created` from oldest to newest and
starts work on at most one issue or PR per run. Apply `relayfold:agent-ready`
when a new issue is ready for implementation. To request follow-up work, post
feedback on the PR. The next run can pick it up from the issue's
`relayfold:pr-created` label without needing `relayfold:agent-ready`.
If there is no eligible work, the run completes successfully before implementation.
GitHub API errors fail the run.

Add `"issue_number": 46` to target a specific issue with either label.
To adopt an existing PR, also supply `"pr_number": 51` and post feedback on
the PR before running. The PR must have its head branch in the configured
repository. The supplied number must match any PR recorded in the issue's status comment.

## How polling and PR follow-up work

1. The fetch Function reads the issue and looks for a RelayFold status comment
   written by the GitHub account used for `gh_token`. It can also read the PR link
   from an older RelayFold summary written by the same account. If it finds no
   record, it checks the issue branch, `relayfold/issue-<issue_number>`. This
   lets polling recover if publishing the PR succeeded but writing the issue
   comment failed. An issue with `relayfold:pr-created` and no discoverable PR
   is skipped. You can adopt an older PR on another branch by supplying its
   number explicitly.
2. For an issue without a PR, the implementation and review Agents prepare and
   verify the changes, then the publication Agent opens a PR from the issue
   branch. For an open PR, the fetch Function reads discussion, inline comments,
   and submitted reviews. Feedback from owners, organization
   members, and collaborators can trigger work. Empty feedback, bots, and
   feedback from the token's account are ignored. Use a dedicated GitHub account
   for the token so developers can comment from their own accounts.
3. The implementation Agent checks out the existing PR branch and addresses all
   new feedback in order. It uses the issue and earlier PR discussion as context.
   The review Agent verifies the changes and the publication Agent pushes them to
   the same PR.
4. After publication succeeds, a Function posts or updates the issue's status
   comment with the PR link and records which feedback it handled. A later poll
   skips handled feedback and ignores closed or merged PRs. If no PR was needed
   for the initial issue, the comment records that result so the next poll skips
   the issue. The labeling Function then adds or retains `relayfold:pr-created`
   when there is a PR and removes `relayfold:agent-ready`.

Leave `relayfold:pr-created` on the issue while the PR is open. It does not
prevent feedback monitoring. Keep the status comment and use the same GitHub
account across runs so RelayFold can find its previous work.
The first poll of a PR tracked by an older issue summary can include comments
that were already addressed. The implementation Agent checks them against the
current branch before making changes.

All Agent tasks share a workspace within the run. A subsequent poll uses the
remote PR branch and GitHub discussion for context. Verifier retries within one
run can reuse the implementation Agent's session.

## Register the workflow

First build, test, and register the fetch, status, and shared labeling Functions from a local checkout (Node.js 24 or newer; no npm dependencies are required):

```bash
cd examples/github_issue_to_pr/functions
npm test
export RELAYFOLD_URL=http://localhost:3000
curl -fsS -X POST "$RELAYFOLD_URL/function-def" \
  --data-binary @dist/github-issue-to-pr.fetch_issue_and_pr.json
curl -fsS -X POST "$RELAYFOLD_URL/function-def" \
  --data-binary @dist/github-issue-to-pr.update_issue_status.json
cd ../../functions
npm test
curl -fsS -X POST "$RELAYFOLD_URL/function-def" \
  --data-binary @dist/github.apply_labels.json
cd ../..
```

Then register the workflow from the same local checkout:

```bash
export RELAYFOLD_URL=http://localhost:3000

curl -fsS -X POST "$RELAYFOLD_URL/workflow-def" \
  --data-binary @examples/github_issue_to_pr/example_github_issue_pr_workflow.yaml
```

If you already registered this workflow or its Functions, wait for active runs
to finish before updating the definitions. See [registration and overwrites](/relayfold/docs/guides/register-and-run-workflow/).

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

## Run on a schedule

Add an entry to your [scheduler configuration](/relayfold/docs/operations/scheduler/).
This example polls every five minutes.

```yaml
scheduler:
  - namespace: global-namespace
    workflow_def_id: github-issue-to-pr
    cron: "*/5 * * * *"
    input:
      repository: parsablelabs/relayfold
```

The scheduler skips a poll while an instance of the same workflow with the same
input is active. A run waiting for clarification can block that repository's
next scheduled poll until you respond and it finishes. Avoid concurrent manual
runs for the same repository. If a poll finds no work, inspect the
`fetch-issue-and-pr` result for the exit reason.
