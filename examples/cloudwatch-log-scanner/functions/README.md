# CloudWatch Log Scanner Functions

Standalone Functions for `examples/cloudwatch-log-scanner/example_cloudwatch_log_scanner.yaml`.
The workflow references these registry IDs:

| Function ID                    | Source                    | Credentials                                                                  | Runtime dependencies                       |
| ------------------------------ | ------------------------- | ---------------------------------------------------------------------------- | ------------------------------------------ |
| `cloudwatch-log-scanner.scan_cloudwatch` | `src/scan-cloudwatch.mjs` | `aws_access_key_id`, `aws_secret_access_key`, optionally `aws_session_token` | `@aws-sdk/client-cloudwatch-logs@3.1146.0` |
| `cloudwatch-log-scanner.prune_covered_logs` | `src/prune-covered-logs.mjs` | None | None |
| `cloudwatch-log-scanner.fetch_issues` | `src/fetch-issues.mjs` | `gh_token` | None |
| `cloudwatch-log-scanner.publish_issues`  | `src/publish-issues.mjs`  | `gh_token`                                                                   | None                                       |

## Build and test

Requires Node.js 24 or newer. From this directory:

```bash
npm ci
npm test
```

Run `npm run format` to format JavaScript in `src/`, `scripts/`, and `test/`.
Generated artifacts in `dist/` are excluded.

`npm test` builds registry artifacts, then runs the tests. Tests use injected
CloudWatch clients, a clock, and GitHub HTTP responses. They assert returned
counts, timestamps, grouped samples, original message preservation, empty results, pagination,
failures, dry runs, duplicate prevention, and issue POST contents. A separate
test imports the generated scanner artifact and uses the **real AWS SDK** against
a localhost CloudWatch protocol server, verifying it returns warning/error data
through the SDK's signed request and paginated response path. Allow localhost
socket binding when running these tests in a sandbox.

No automated test contacts AWS or GitHub. All four generated Function entry points
are exercised. The runtime exports default functions with RelayFold's normal
`{ inputs, credentials, workspacePath }` context. The named `createScanner` and
`createPublisher`, and `createIssueFetcher` factories expose side effects for tests; workflows use the
normal default exports.

## Check against real CloudWatch

Use existing read-only AWS credentials in your environment with
`logs:FilterLogEvents` permission on the selected groups:

```bash
export SCAN_REPOSITORY=example/service
export SCAN_TARGETS_JSON='[{"region":"us-east-1","log_group":"/aws/lambda/phonecallfetcher_prod"}]'
export SCAN_LOOKBACK_HOURS=24
npm run test:live
```

Replace the example group with an actual deployed group known to contain warnings
or errors. Set `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, and optionally
`AWS_SESSION_TOKEN` through your usual credential mechanism before running.

This invokes the actual scanner against AWS and **fails unless it returns events,
groups, and samples**. It prints only the time window and result counts, keeping
log excerpts and credentials out of the output. The check never publishes issues.
A zero-result failure can mean the selected window has no matches; it is not
by itself evidence of a scanner bug. The window ends five minutes ago and includes
ten minutes of overlap. Increase the lookback (maximum 168 hours) if needed.

## Register

Build without running tests with `npm run build`. The shared builder emits JSON
and YAML artifacts under `dist/`. Register the JSON definitions from this directory:

```bash
export RELAYFOLD_URL=http://localhost:3000
curl -fsS -X POST "$RELAYFOLD_URL/function-def" \
  --data-binary @dist/cloudwatch-log-scanner.scan_cloudwatch.json
curl -fsS -X POST "$RELAYFOLD_URL/function-def" \
  --data-binary @dist/cloudwatch-log-scanner.fetch_issues.json
curl -fsS -X POST "$RELAYFOLD_URL/function-def" \
  --data-binary @dist/cloudwatch-log-scanner.prune_covered_logs.json
curl -fsS -X POST "$RELAYFOLD_URL/function-def" \
  --data-binary @dist/cloudwatch-log-scanner.publish_issues.json
```

Also build and register `github.apply_labels` from `examples/functions` before executing this workflow; see that directory's README.

Then register the updated workflow from the repository root. Artifacts are generated
and ignored by Git; build them from the committed sources and pinned lockfile.

## Input and output contracts

`scan_cloudwatch` reads `inputs[0]`:

```json
{
  "repository": "example/service",
  "targets": [
    { "region": "us-east-1", "log_group": "/aws/lambda/phonecallfetcher_prod" }
  ],
  "lookback_hours": 24,
  "dry_run": true
}
```

`repository` (GitHub `owner/repo`) and targets are required. The repository is passed through the scan to both analysis and publishing. `lookback_hours` defaults to 24 and must be an integer from
1 to 168. `dry_run` defaults to false and is passed to the publisher. Output
contains `repository`, `branch`, `start_time`, `end_time`, `dry_run`,
`total_events`, `truncated`, `truncation`, and `groups`. Each group includes its `fingerprint`, `log_group`,
`regions`, `count`, `first_seen`, `last_seen`, and up to three original
`{ timestamp, message }` samples. Samples are capped at 6,000 characters.
Counts are keyword matches; the workflow's agent evaluates actual severity.
Limits are 200 pages per target, 50,000 retained events, and 200 patterns.
When a limit prevents further scanning, the function returns collected data with
`truncated: true` and `truncation` entries containing `region`, `log_group`, and
`reason` (`page_limit`, `event_limit`, or `pattern_limit`). Page limits move on to
the next target; event and pattern limits stop the scan globally. Counts cover
retained events only. Complete scans return `truncated: false` and an empty list.
AWS errors and repeated pagination tokens still fail the task.

Samples preserve original values and formatting, apart from the size cap.
Masking and sanitization must happen upstream. IDs, numbers, and whitespace are
normalized only in the internal grouping signature; samples are not normalized.

`fetch_issues` reads `inputs[0].repository` and returns `{ repository, issues }`. It fetches every page of open GitHub issues labeled `relayfold`, excludes pull requests, and returns only `number`, `title`, `body`, and `html_url` per issue. GitHub failures or pagination-limit exhaustion fail the task rather than returning an incomplete list. It runs independently of scanning; analysis and verification receive both results. Their semantic duplicate checks cover this open labeled list; the publisher retains its final fingerprint check across open and closed issues. Published issues receive the `relayfold` label.

`prune_covered_logs` receives the scan and fetched open issues in either order, requires matching repositories, and removes groups whose fingerprints appear in exact current issue markers. It preserves all other scan metadata, including `total_events` (the count before pruning), samples for remaining groups, and truncation information. It has no credentials, network calls, or persistent cache. Only open issues labeled `relayfold` from `fetch_issues` participate; unmarked and closed issues do not suppress patterns at this stage.

Analysis, verification, and publishing receive this pruned scan. If every pattern is covered, both Agents return their no-work outputs without inspecting the repository. They still make LLM calls; pruning reduces input and inspection work rather than bypassing Agent execution.

`publish_issues` accepts the pruned scanner output and an accepted verifier envelope
`{"decision":"complete","output":<analysis>}` in `inputs` (in either order).
Nonempty analysis must contain `commit_sha`, `summary`, and `findings`.
Each finding may include `bug` for a clear code defect and `relayfold:human-input-needed` for critical decisions or missing information that require a human. Both classifications are checked by the verifier. Flagged issues list specific questions under Notes and human prerequisites in Acceptance Criteria. Both labels may be present; omission or `[]` means neither applies. The publisher adds `relayfold` and optional `bug` on creation; the shared `github.apply_labels` step applies the human-input label afterward. Dry-run drafts include all planned labels. Newly created entries include `repository`, `issue_number`, and `human_input_needed` for the shared step. A human adds the missing information in a comment and removes the human-input label to allow the issue-to-PR workflow to proceed.
Each finding has `fingerprint`, `title`, and `body` with Problem, Goal, Acceptance
Criteria, and Notes sections. At most three findings are accepted; fingerprints
must correspond to scanned groups. It checks open and closed issues and returns
`created`, `skipped`, `dry_run`, and `summary`, plus `drafts` when findings exist.
Fingerprint markers use the `cloudwatch-log-scanner-cloudwatch` prefix; older markers no longer match the exact duplicate check. Dry runs return drafts without creating issues. An empty findings array performs
no GitHub calls. GitHub failures propagate; issue POSTs are never retried automatically.

See the website's CloudWatch Log Scanner CloudWatch Review example for scheduling,
credential scopes, truncation, upstream masking, and duplicate handling.

The `verify-analysis` Agent accepts analysis unchanged or requests a retry with feedback. It reruns `analyze-main` for up to three generations without repeating the scan, failing the workflow if the final generation is rejected. When the scan has no groups, analysis returns `{}` and verification returns `{"decision":"complete","output":{}}` without tools. The publisher treats an accepted empty output as a successful no-work result without GitHub requests. Missing or rejected verifier output fails before publishing.
