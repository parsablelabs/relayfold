---
title: CloudWatch Log Scanner
description: Periodically inspect warnings and errors against main and file actionable GitHub issues without editing application code.
---

[`examples/cloudwatch-log-scanner/example_cloudwatch_log_scanner.yaml`](https://github.com/parsablelabs/relayfold/blob/main/examples/cloudwatch-log-scanner/example_cloudwatch_log_scanner.yaml)
scans selected CloudWatch log groups, clones the input `repository` at current
`main`, and submits detailed issues when logs reveal a concrete improvement.
It makes no application code changes and creates no branches, commits, or pull requests.

## Flow

<pre class="mermaid">
flowchart TD
    Input["Input: repository + CloudWatch targets"]
    Scan["scan-cloudwatch: collect log evidence"]
    Issues["fetch-issues: open issues labeled relayfold"]
    Prune["prune-covered-logs: remove fingerprints covered by open issues"]
    Select["select-group: choose one group by severity and impact"]
    Prepare["prepare-selected-group: retain original selected evidence"]
    Analyze["analyze-main: draft findings or return empty object"]
    Verify{"verify-analysis: accept or retry?"}
    Publish["publish-issues: create labeled issues or finish with no work"]
    Labels["apply-labels: mark issues needing human input"]
    Done["Completed"]
    Failed["Failed: final generation rejected"]

    Input --> Scan
    Input --> Issues
    Scan -->|"no patterns: exit reason returned"| Done
    Scan -->|"patterns remain"| Prune
    Issues --> Prune
    Prune -->|"all covered: exit reason returned"| Done
    Prune -->|"patterns remain"| Select
    Prune --> Prepare
    Select --> Prepare
    Prepare --> Analyze
    Issues --> Analyze
    Prepare --> Verify
    Issues --> Verify
    Analyze --> Verify
    Verify -. "continue + feedback; up to 3 generations" .-> Analyze
    Verify -->|"complete + unchanged analysis or empty object"| Publish
    Prepare --> Publish
    Verify -->|"continue at retry limit"| Failed
    Publish --> Labels
    Labels --> Done
</pre>

1. `scan-cloudwatch` uses the AWS SDK to fetch paginated WARN/WARNING/ERROR keyword matches from each explicit region and log group. It normalizes recurring messages, counts occurrences, and retains up to three original samples per pattern for selection and analysis. It returns `_workflow_exit_reason: "No matching log patterns were found."` when no patterns were collected, requesting successful early completion.
2. `fetch-issues` runs independently of scanning, fetching all open issues labeled `relayfold` in the input repository. It excludes pull requests and fails if it cannot retrieve the complete list.
3. `prune-covered-logs` deterministically removes log patterns whose exact fingerprints appear in the fetched open issues, before any LLM analysis. It uses the existing issue markers and needs no separate persistent state. All unmatched patterns reach selection; only the selected pattern and its samples reach analysis, verification, and publishing. The scan window and truncation information are preserved; `total_events` still counts the original scan before pruning. Unmarked or closed issues do not suppress patterns at this stage. It recomputes `_workflow_exit_reason` from the remaining groups and requests successful early completion with "No uncovered log patterns remain." when they are empty.
4. `select-group` uses an Agent without tools to choose exactly one uncovered pattern by actual severity and likely user impact. Recurrence and recency break ties; keyword matches alone do not establish severity. It requires a nonempty pruned scan and returns the original non-null fingerprint and a short selection reason; empty scans terminate upstream. `prepare-selected-group` validates that fingerprint and extracts the original group unchanged, preserving the scan window, truncation, and original `total_events`. No unselected group is passed downstream.
5. `analyze-main` first inspects the scan. When `groups` is empty, it returns `{}` without tools or repository inspection. Otherwise, it clones the repository, records the main commit, and traces symptoms through its application services. It checks actual severity, current code, and the supplied open issues labeled `relayfold`, comparing root causes rather than titles. It waits for both scanning and issue fetching. It investigates only the selected group and drafts at most one actionable issue, or returns no findings with an explanation. Every log claim must come from that group; unrelated symptoms must not be combined.
6. `verify-analysis` is an Agent that reviews the analysis against scan evidence and relevant repository context. It accepts the unchanged analysis or returns actionable feedback to retry `analyze-main`. The loop permits three generations and fails if the last is rejected; scanning is not repeated.
7. `publish-issues` consumes the accepted verifier output, validates the drafts and checks all existing issues for stable fingerprint markers before submitting them. Each issue includes **Problem**, **Goal**, **Acceptance Criteria**, and **Notes**, with log evidence, code permalinks, root-cause reasoning, and regression-test criteria.

8. `apply-labels` uses the shared `github.apply_labels` Function to add `relayfold:human-input-needed` to newly published issues whose verified findings require human decisions or missing information. It creates the repository label if needed and confirms application. Dry runs and no-work results make no labeling requests.

Analysis and verification assess whether each finding is immediately implementable. Flagged findings must list specific questions under Notes and distinguish human prerequisites from implementation tasks in Acceptance Criteria. Routine implementation choices and code investigation do not require the label. A human supplies the answers in an issue comment and removes `relayfold:human-input-needed`; the issue-to-PR workflow rejects the issue until that label is removed.

New issues receive the `relayfold` label so later runs include them in semantic duplicate inspection. Analysis may also select the optional `bug` label when log and code evidence establish a clear bug, with the defect explained in the issue body and checked by the verifier. General improvements and uncertain hypotheses do not receive `bug`. Dry-run drafts include the labels that would be published. That inspection covers open labeled issues; the publisher also checks fingerprint markers across open and closed issues immediately before creating new ones.

An empty scan, or a scan whose patterns are all covered by open issues, completes
successfully before any Agent runs. Both `scan-cloudwatch` and
`prune-covered-logs` return the reserved `_workflow_exit_reason` field; no YAML
exit toggle is needed.

Their output schemas require `_workflow_exit_reason` as a string or `null`.
Nonempty scans return `null` and continue. A nonempty reason skips remaining
pending tasks, including analysis, verification, publishing, and labeling. The
triggering task records `early_exit: true`; its output explains whether the scan
was empty or its patterns were already covered by open issues. The triggering output is retained, including the scan
window, counts, and truncation information. Independent issue fetching may already
have run; already-running tasks finish normally. An exit reason means there are
no retained patterns to analyze, not that a truncated scan proves the absence of errors.

Neither `analyze-main` nor `verify-analysis` requests workflow exit: a conclusion
that there are no actionable findings still passes through verification. These
tasks retain their empty-scan handling for standalone use, and publishing retains
its no-op handling for accepted empty analyses and findings arrays.

RelayFold requires a verifier decision envelope. On acceptance, `verify-analysis` returns `{"decision":"complete","output":<unchanged analysis>}`; for no work, `output` is `{}`. On rejection it returns `{"decision":"continue","feedback":"..."}`. The publisher receives the scan and verifier envelope, extracts `output`, and completes without GitHub requests when that object is empty. See [Bounded Loops](/relayfold/docs/concepts/bounded-loops/).

CloudWatch keyword matching is a candidate filter; the agent confirms severity and
relevance. Pagination continues across empty pages as specified by the
[AWS FilterLogEvents API](https://docs.aws.amazon.com/AmazonCloudWatchLogs/latest/APIReference/API_FilterLogEvents.html).
When limits prevent further scanning (50,000 retained events, 200 pages per target,
or 200 distinct patterns), the scan completes with the data collected so far.
Output includes `truncated: true` and `truncation` entries identifying the target
and reason (`event_limit`, `page_limit`, or `pattern_limit`). Page limits continue
with the next target; global event or pattern limits stop scanning. Counts describe
retained events only. The analysis and issue text disclose incomplete coverage.
Complete scans return `truncated: false` and `truncation: []`.
AWS access errors and repeated pagination tokens still fail the task.

## Credentials and tooling

The worker needs Node.js, `git`, `bash`, `gh`, and npm access to install the pinned
`@aws-sdk/client-cloudwatch-logs` dependency. The AWS CLI is not required.
Add these credentials to the worker credential file:

```json
{
  "aws_access_key_id": "...",
  "aws_secret_access_key": "...",
  "gemini_api_key": "...",
  "gh_token": "..."
}
```

Scope AWS permissions to `logs:FilterLogEvents` on the intended log groups.
The GitHub token needs repository contents read access and issues read/write
access to the input repository; give it no contents write access. Both the
analysis and verifier Agents and publishing Function receive `gh_token`, exposed as `GH_TOKEN`.
The analysis Agent is instructed to inspect the repository and draft findings;
the publishing Function submits the issues.
Temporary AWS credentials also require `aws_session_token` in the credential file
and in `scan-cloudwatch.required_credentials`; the function already supports it.

Log excerpts are sent to the configured model provider and may be published in
issues. The scanner preserves message values and formatting, apart from sample
size limits; masking and sanitization are handled upstream before logs are scanned.
Grouping normalizes IDs and numbers only in an internal signature, not in samples.

## Build, test, and register Functions

The workflow references `cloudwatch-log-scanner.scan_cloudwatch`, `cloudwatch-log-scanner.fetch_issues`,
`cloudwatch-log-scanner.prune_covered_logs`, `cloudwatch-log-scanner.prepare_selected_group`, and `cloudwatch-log-scanner.publish_issues`. The shared label step references `github.apply_labels` from [`examples/functions`](https://github.com/parsablelabs/relayfold/tree/main/examples/functions). The scanner Functions' standalone source, manifest, tests, and
artifact build script live in
[`examples/cloudwatch-log-scanner/functions`](https://github.com/parsablelabs/relayfold/tree/main/examples/cloudwatch-log-scanner/functions).
Issue fingerprint markers use the `cloudwatch-log-scanner-cloudwatch` prefix.
Issues created with the previous prefix no longer match the publisher's exact
fingerprint check; analysis still compares the fetched open issues for duplicates.

Register the four scanner Functions and the shared labeling Function before executing the workflow:

```bash
cd examples/cloudwatch-log-scanner/functions
npm ci
npm test
export RELAYFOLD_URL=http://localhost:3000
curl -fsS -X POST "$RELAYFOLD_URL/function-def" \
  --data-binary @dist/cloudwatch-log-scanner.scan_cloudwatch.json
curl -fsS -X POST "$RELAYFOLD_URL/function-def" \
  --data-binary @dist/cloudwatch-log-scanner.fetch_issues.json
curl -fsS -X POST "$RELAYFOLD_URL/function-def" \
  --data-binary @dist/cloudwatch-log-scanner.prune_covered_logs.json
curl -fsS -X POST "$RELAYFOLD_URL/function-def" \
  --data-binary @dist/cloudwatch-log-scanner.prepare_selected_group.json
curl -fsS -X POST "$RELAYFOLD_URL/function-def" \
  --data-binary @dist/cloudwatch-log-scanner.publish_issues.json
cd ../../functions
npm test
curl -fsS -X POST "$RELAYFOLD_URL/function-def" \
  --data-binary @dist/github.apply_labels.json
cd ../..
```

Tests assert nonempty scanner results from known log fixtures, returned counts
and original samples, truncation at limits, empty pages, errors, and issue publishing behavior.
They also exercise the generated scanner with the real AWS SDK against a local
CloudWatch protocol server. Automated tests do not call AWS or GitHub.

To format the standalone JavaScript, run `npm run format` from
`examples/cloudwatch-log-scanner/functions`. It formats `src/`, `scripts/`, and `test/`, excluding
generated artifacts in `dist/`.

To verify results from your actual CloudWatch log groups, supply read-only AWS
credentials through `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, and optionally
`AWS_SESSION_TOKEN`, then explicitly run:

```bash
cd examples/cloudwatch-log-scanner/functions
export SCAN_REPOSITORY=markosski/fillmyfunnel
export SCAN_TARGETS_JSON='[{"region":"us-east-1","log_group":"/aws/lambda/phonecallfetcher_prod"}]'
export SCAN_LOOKBACK_HOURS=24
npm run test:live
cd ../../..
```

Replace the example group with a confirmed group containing WARN/ERROR events.
The live check fails if no matching events, groups, or samples are returned and
prints only counts and the scan window. It never publishes GitHub issues.
See the Function workspace README for contracts and test details.

## Register and preview

The workflow's `example_input` prefills the UI JSON trigger input with the
repository and log group shown below, a 24-hour lookback, and `dry_run: true`.
Edit the repository, region, and log group to match your deployment before
starting a run. Set `dry_run: false` when you want to publish issues.

```bash
export RELAYFOLD_URL=http://localhost:3000
curl -fsS -X POST "$RELAYFOLD_URL/workflow-def" \
  --data-binary @examples/cloudwatch-log-scanner/example_cloudwatch_log_scanner.yaml
```

Choose the actual log groups and regions from your deployed AWS environment;
the repository contains deployments in `us-east-1` and `us-west-2`, but this
workflow does not assume every service or environment is active.
For example, preview one known deployed group:

```bash
curl -fsS -X POST "$RELAYFOLD_URL/workflow-def/cloudwatch-review" \
  -H 'content-type: application/json' \
  -d '{
    "repository": "markosski/fillmyfunnel",
    "targets": [
      {"region": "us-east-1", "log_group": "/aws/lambda/phonecallfetcher_prod"}
    ],
    "lookback_hours": 24,
    "dry_run": false
  }'
```

Set `repository` to the GitHub `owner/repo` to inspect and publish issues to. Replace that example group with a confirmed deployed name. Add more target objects
for other services or regions. `lookback_hours` defaults to 24 (range 1–168).
`dry_run: true` returns issue drafts without submitting them; `false` submits
issues and is the default. After completion, inspect the publishing result:

```bash
curl -fsS "$RELAYFOLD_URL/workflows/<workflow_id>/tasks/publish-issues"
```

Each run investigates one group and creates at most one issue. Other groups are reconsidered on later scans; they are not queued persistently. A selected group that yields no actionable issue can be selected again and delay lower-priority groups. Inspect the `select-group` task output for its selection reason.
