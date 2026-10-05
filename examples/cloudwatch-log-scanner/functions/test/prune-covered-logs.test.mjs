import assert from "node:assert/strict";
import test from "node:test";
import { readFile } from "node:fs/promises";
import prune from "../src/prune-covered-logs.mjs";
import { createPublisher } from "../src/publish-issues.mjs";

const first = "a".repeat(64);
const second = "b".repeat(64);
const marker = (fingerprint) =>
  `<!-- cloudwatch-log-scanner-cloudwatch:${fingerprint} -->`;
const scan = {
  repository: "example/service",
  groups: [
    { fingerprint: first, count: 7, samples: [{ message: "First error" }] },
    { fingerprint: second, count: 2, samples: [{ message: "Second error" }] },
  ],
  total_events: 9,
  start_time: "2026-10-02T12:00:00Z",
  end_time: "2026-10-03T12:00:00Z",
  dry_run: true,
  truncated: true,
  truncation: [
    { reason: "page_limit", region: "us-east-1", log_group: "/test" },
  ],
};
const fetched = (issues) => ({ repository: scan.repository, issues });

test("prunes exact issue markers, preserving remaining evidence and scan metadata", () => {
  const issues = fetched([{ body: `Known failure\n${marker(first)}` }]);
  for (const inputs of [
    [scan, issues],
    [issues, scan],
  ]) {
    const result = prune({ inputs });
    assert.deepEqual(result, {
      ...scan,
      groups: [scan.groups[1]],
      workflow_exit_reason: null,
    });
    assert.equal(scan.groups.length, 2);
  }
});

test("retains unmatched groups, including text without a valid current marker", () => {
  for (const issues of [
    [],
    [{ body: null }],
    [{ body: first }],
    [{ body: `<!-- old-scanner-cloudwatch:${first} -->` }],
    [{ body: marker(first.slice(1)) }],
    [{ body: marker("c".repeat(64)) }],
  ])
    assert.deepEqual(prune({ inputs: [scan, fetched(issues)] }), {
      ...scan,
      workflow_exit_reason: null,
    });
});

test("all covered or empty scans produce no-work input for publishing", async () => {
  const publish = createPublisher({
    fetch: () => assert.fail("No publishing request expected"),
  });
  for (const input of [scan, { ...scan, groups: [] }]) {
    const result = prune({
      inputs: [
        input,
        fetched([{ body: `${marker(first)}\n${marker(second)}` }]),
      ],
    });
    assert.deepEqual(result.groups, []);
    assert.equal(
      result.workflow_exit_reason,
      "No uncovered log patterns remain.",
    );
    const published = await publish({
      inputs: [result, { decision: "complete", output: {} }],
    });
    assert.deepEqual(published.created, []);
    assert.match(published.summary, /No work to do/);
  }
});

test("recomputes exit reason from remaining groups instead of trusting the upstream reason", () => {
  assert.equal(
    prune({
      inputs: [
        { ...scan, workflow_exit_reason: "Upstream exit reason" },
        fetched([]),
      ],
    }).workflow_exit_reason,
    null,
  );
  assert.equal(
    prune({
      inputs: [
        { ...scan, workflow_exit_reason: null },
        fetched([{ body: `${marker(first)}\n${marker(second)}` }]),
      ],
    }).workflow_exit_reason,
    "No uncovered log patterns remain.",
  );
});

test("fails on missing inputs or a mismatched repository", () => {
  for (const inputs of [
    [],
    [scan],
    [fetched([])],
    [scan, { repository: "other/service", issues: [] }],
    [{ ...scan, repository: undefined }, { issues: [] }],
  ])
    assert.throws(() => prune({ inputs }), /Missing scan|same repository/);
});

test("generated pruning artifact runs without dependencies", async () => {
  const artifact = JSON.parse(
    await readFile(
      new URL(
        "../dist/cloudwatch-log-scanner.prune_covered_logs.json",
        import.meta.url,
      ),
      "utf8",
    ),
  );
  assert.deepEqual(artifact.dependencies, []);
  const module = await import(
    `data:text/javascript;base64,${Buffer.from(artifact.code).toString("base64")}`
  );
  assert.deepEqual(
    module.default({ inputs: [scan, fetched([{ body: marker(first) }])] }),
    { ...scan, groups: [scan.groups[1]], workflow_exit_reason: null },
  );
});
