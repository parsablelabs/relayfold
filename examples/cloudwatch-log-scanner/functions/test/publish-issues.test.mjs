import assert from "node:assert/strict";
import test from "node:test";
import { readFile } from "node:fs/promises";
import { createPublisher } from "../src/publish-issues.mjs";
import { loadFunctionArtifact } from "../../../../functions/test-helpers/load-function-artifact.mjs";

const fingerprint = "b".repeat(64);
const marker = "<!-- cloudwatch-log-scanner-cloudwatch:" + fingerprint + " -->";
const scan = {
  repository: "example/service",
  groups: [{ fingerprint, count: 7 }],
  start_time: "2026-10-02T12:00:00Z",
  end_time: "2026-10-03T12:00:00Z",
  dry_run: false,
};
const finding = {
  fingerprint,
  title: "Handle queued lead failure",
  body: "## Problem\nObserved failure\n## Goal\nRecover\n## Acceptance Criteria\nRegression test\n## Notes\nCode evidence",
};
const analysis = {
  commit_sha: "a".repeat(40),
  findings: [finding],
  summary: "One improvement",
};
const accepted = (output) => ({ decision: "complete", output });
const context = {
  inputs: [accepted(analysis), scan],
  credentials: { gh_token: "test-token" },
};
const reply = (json, status = 200) => ({
  ok: status < 400,
  status,
  json: async () => json,
});

test("publishes an issue with evidence and fingerprint using the write credential", async () => {
  const requests = [];
  const run = createPublisher({
    fetch: async (url, options) => {
      requests.push({ url, options });
      return reply(
        options.method === "POST"
          ? {
              html_url: "https://github.com/example/service/issues/1",
              body: JSON.parse(options.body).body,
            }
          : [],
      );
    },
  });
  const result = await run(context);
  assert.equal(
    result.created[0].issue_url,
    "https://github.com/example/service/issues/1",
  );
  const posted = requests.find((r) => r.options.method === "POST");
  assert.equal(
    posted.url,
    "https://api.github.com/repos/example/service/issues",
  );
  assert.equal(posted.options.headers.Authorization, "Bearer test-token");
  const body = JSON.parse(posted.options.body);
  assert.equal(body.title, finding.title);
  assert.deepEqual(body.labels, ["relayfold"]);
  assert(body.body.includes(marker));
  assert(body.body.includes("7 matching events"));
  assert(body.body.includes(analysis.commit_sha));
});

test("dry-run returns a draft and never posts", async () => {
  const run = createPublisher({
    fetch: async (url, options) => {
      assert.equal(options.method, undefined);
      return reply([]);
    },
  });
  const result = await run({
    ...context,
    inputs: [{ ...scan, dry_run: true }, accepted(analysis)],
  });
  assert.equal(result.drafts.length, 1);
  assert.deepEqual(result.drafts[0].labels, ["relayfold"]);
  assert.deepEqual(result.created, []);
});

test("optional bug label is included in published issues and dry-run drafts", async () => {
  for (const labels of [["bug"], []]) {
    for (const dry_run of [false, true]) {
      const posted = [];
      const run = createPublisher({
        fetch: async (url, options) => {
          if (options.method === "POST") {
            posted.push(JSON.parse(options.body));
            return reply({ html_url: "created" });
          }
          return reply([]);
        },
      });
      const result = await run({
        ...context,
        inputs: [
          { ...scan, dry_run },
          accepted({ ...analysis, findings: [{ ...finding, labels }] }),
        ],
      });
      const issue = dry_run ? result.drafts[0] : posted[0];
      assert.deepEqual(issue.labels, ["relayfold", ...labels]);
      assert.equal(posted.length, dry_run ? 0 : 1);
    }
  }
});

test("rejects unsupported or malformed labels before network calls", async () => {
  const run = createPublisher({
    fetch: () => assert.fail("No network request expected"),
  });
  for (const labels of [null, "bug", ["enhancement"], ["bug", "bug"]]) {
    await assert.rejects(
      run({
        ...context,
        inputs: [
          scan,
          accepted({ ...analysis, findings: [{ ...finding, labels }] }),
        ],
      }),
      /Optional finding labels/,
    );
  }
});

test("checks subsequent pages and skips closed duplicate issues", async () => {
  let calls = 0;
  const run = createPublisher({
    fetch: async (url, options) => {
      assert.equal(options.method, undefined);
      calls++;
      if (calls === 1)
        return reply(
          Array.from({ length: 100 }, () => ({
            pull_request: {},
            body: marker,
          })),
        );
      assert(url.endsWith("page=2"));
      return reply([{ body: marker, state: "closed", html_url: "existing" }]);
    },
  });
  const result = await run(context);
  assert.deepEqual(result.skipped, [{ fingerprint, issue_url: "existing" }]);
  assert.equal(calls, 2);
});

test("rejects ungrounded, duplicate and malformed drafts before network calls", async () => {
  const run = createPublisher({
    fetch: () => assert.fail("No network request expected"),
  });
  for (const findings of [
    [{ ...finding, fingerprint: "c".repeat(64) }],
    [finding, finding],
    [{ ...finding, body: "Missing required sections" }],
    [finding, finding, finding, finding],
  ])
    await assert.rejects(
      run({ ...context, inputs: [scan, accepted({ ...analysis, findings })] }),
    );
});

test("failed duplicate inspection never creates an issue and uncertain POST is not retried", async () => {
  const run = createPublisher({ fetch: async () => reply(null, 403) });
  await assert.rejects(run(context), /403/);
  let posts = 0;
  const uncertain = createPublisher({
    fetch: async (url, options) => {
      if (options.method === "POST") {
        posts++;
        throw new Error("Connection lost");
      }
      return reply([]);
    },
  });
  await assert.rejects(uncertain(context), /Connection lost/);
  assert.equal(posts, 1);
});

test("generated publisher is dependency-free and returns the empty-findings result", async () => {
  const artifact = JSON.parse(
    await readFile(
      new URL(
        "../dist/cloudwatch-log-scanner.publish_issues.json",
        import.meta.url,
      ),
    ),
  );
  assert.deepEqual(artifact.dependencies, []);
  const run = await loadFunctionArtifact(
    "../../examples/cloudwatch-log-scanner/functions/dist/cloudwatch-log-scanner.publish_issues.json",
  );
  assert.deepEqual(
    await run({
      ...context,
      inputs: [scan, accepted({ ...analysis, findings: [] })],
    }),
    {
      created: [],
      skipped: [],
      dry_run: false,
      summary: "One improvement",
    },
  );
});

test("drafts explicitly disclose truncated scan evidence", async () => {
  const run = createPublisher({ fetch: async () => reply([]) });
  const result = await run({
    ...context,
    inputs: [
      accepted(analysis),
      {
        ...scan,
        dry_run: true,
        truncated: true,
        truncation: [
          { region: "us-east-1", log_group: "/test", reason: "event_limit" },
        ],
      },
    ],
  });
  assert.match(result.drafts[0].body, /counts cover retained events only/);
  assert.match(result.drafts[0].body, /event_limit/);
});

test("rejects missing or invalid repository before GitHub requests", async () => {
  const run = createPublisher({
    fetch: () => assert.fail("No request expected"),
  });
  for (const repository of [
    undefined,
    "https://github.com/example/service",
    "example/service?other",
  ]) {
    await assert.rejects(
      run({
        ...context,
        inputs: [{ ...scan, repository }, accepted(analysis)],
      }),
      /GitHub owner\/repo/,
    );
  }
});

test("accepted empty output completes without GitHub requests", async () => {
  const run = createPublisher({
    fetch: () => assert.fail("No GitHub request expected"),
  });
  for (const groups of [[], scan.groups]) {
    for (const inputs of [
      [{ ...scan, groups }, accepted({})],
      [accepted({}), { ...scan, groups }],
    ]) {
      const result = await run({ ...context, inputs });
      assert.deepEqual(result.created, []);
      assert.deepEqual(result.skipped, []);
      assert.equal(result.dry_run, false);
      assert.match(result.summary, /No work to do/);
    }
  }
});

test("rejects missing, rejected, or malformed verifier output before requests", async () => {
  const run = createPublisher({
    fetch: () => assert.fail("No GitHub request expected"),
  });
  for (const verification of [
    undefined,
    {},
    analysis,
    { decision: "continue", feedback: "Retry", output: analysis },
    accepted(null),
    accepted([]),
    accepted({ summary: "Incomplete" }),
  ]) {
    await assert.rejects(run({ ...context, inputs: [scan, verification] }));
  }
  await assert.rejects(run({ ...context, inputs: [accepted({})] }));
});
