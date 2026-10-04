import assert from "node:assert/strict";
import scan from "../src/scan-cloudwatch.mjs";

// Read-only, explicitly invoked integration check. Never publishes GitHub issues.
const targets = JSON.parse(process.env.SCAN_TARGETS_JSON ?? "null");
if (!Array.isArray(targets) || !targets.length) {
  throw new Error(
    'Set SCAN_TARGETS_JSON to [{"region":"us-east-1","log_group":"your-log-group"}]',
  );
}
const credentials = {
  aws_access_key_id: process.env.AWS_ACCESS_KEY_ID,
  aws_secret_access_key: process.env.AWS_SECRET_ACCESS_KEY,
  ...(process.env.AWS_SESSION_TOKEN
    ? { aws_session_token: process.env.AWS_SESSION_TOKEN }
    : {}),
};
if (!credentials.aws_access_key_id || !credentials.aws_secret_access_key) {
  throw new Error(
    "Set AWS_ACCESS_KEY_ID and AWS_SECRET_ACCESS_KEY (and AWS_SESSION_TOKEN for temporary credentials)",
  );
}
const result = await scan({
  inputs: [
    {
      repository: process.env.SCAN_REPOSITORY,
      targets,
      lookback_hours: Number(process.env.SCAN_LOOKBACK_HOURS ?? 24),
      dry_run: true,
    },
  ],
  credentials,
});
console.log(
  JSON.stringify(
    {
      start_time: result.start_time,
      end_time: result.end_time,
      total_events: result.total_events,
      patterns: result.groups.length,
      truncated: result.truncated,
      truncation: result.truncation,
    },
    null,
    2,
  ),
);
assert(
  result.total_events > 0 && result.groups.length > 0,
  "No WARN/ERROR results: choose a log group and lookback with known matching events; the scan ends five minutes ago",
);
assert(
  result.groups.every((group) => group.count > 0 && group.samples.length > 0),
);
console.log(
  "PASS: CloudWatch returned matching events and the scanner returned nonempty groups/samples.",
);
