import assert from "node:assert/strict";
import test from "node:test";
import prepare from "../src/prepare-selected-group.mjs";
import { loadFunctionArtifact } from "../../../../functions/test-helpers/load-function-artifact.mjs";

const first = "a".repeat(64);
const second = "b".repeat(64);
const scan = {
  repository: "example/service",
  groups: [
    { fingerprint: first, count: 20, samples: [{ message: "WARN retry" }] },
    {
      fingerprint: second,
      count: 1,
      samples: [{ message: "ERROR lost data" }],
    },
  ],
  total_events: 21,
  start_time: "2026-10-02T12:00:00Z",
  end_time: "2026-10-03T12:00:00Z",
  dry_run: true,
  truncated: true,
  truncation: [
    { reason: "page_limit", region: "us-east-1", log_group: "/test" },
  ],
  _workflow_exit_reason: null,
};
const selection = {
  fingerprint: second,
  reason: "Possible data loss outranks retries",
};

test("isolates the selected original evidence and preserves scan metadata", () => {
  for (const inputs of [
    [scan, selection],
    [selection, scan],
  ]) {
    const result = prepare({ inputs });
    assert.deepEqual(result, { ...scan, groups: [scan.groups[1]] });
    assert.equal(result.groups[0], scan.groups[1]);
    assert.equal(scan.groups.length, 2);
  }
});

test("rejects missing, unknown, or null selections for a nonempty scan", () => {
  for (const inputs of [
    [],
    [scan],
    [selection],
    [scan, { ...selection, fingerprint: "c".repeat(64) }],
    [scan, { ...selection, fingerprint: null }],
    [scan, { ...selection, reason: " " }],
  ]) {
    assert.throws(() => prepare({ inputs }));
  }
});

test("rejects empty scans because upstream pruning must terminate the workflow", () => {
  const empty = { ...scan, groups: [] };
  for (const fingerprint of [null, second]) {
    assert.throws(
      () => prepare({ inputs: [empty, { ...selection, fingerprint }] }),
      /Selected fingerprint must identify a pruned scan group/,
    );
  }
});

test("generated selection preparation artifact preserves the isolation boundary", async () => {
  const run = await loadFunctionArtifact(
    "../../examples/cloudwatch-log-scanner/functions/dist/cloudwatch-log-scanner.prepare_selected_group.json",
  );
  assert.deepEqual(await run({ inputs: [scan, selection] }), {
    ...scan,
    groups: [scan.groups[1]],
  });
});
