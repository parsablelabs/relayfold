import assert from "node:assert/strict";
import test from "node:test";
import { readFile } from "node:fs/promises";
import { createLabeler } from "../src/apply-labels.mjs";
import { createPublisher } from "../../cloudwatch-log-scanner/functions/src/publish-issues.mjs";

const label = "relayfold:pr-created";
const issue = {
  repository: "example/service",
  issue_number: 7,
  issue_url: "https://github.com/example/service/issues/7",
};
const pr = {
  repository: issue.repository,
  issue_number: issue.issue_number,
  pr_created: true,
  pr_number: 12,
};
const context = {
  inputs: [issue, pr],
  credentials: { gh_token: "test-token" },
};
const reply = (json, status = 200) => ({
  ok: status < 400,
  status,
  json: async () => json,
});

test("adds the marker without replacing existing labels and supports repeated execution", async () => {
  const calls = [];
  const run = createLabeler({
    fetch: async (url, options) => {
      calls.push({ url, options });
      return reply([{ name: "relayfold" }, { name: "bug" }, { name: label }]);
    },
  });
  for (const inputs of [[issue, pr], [pr, issue]]) {
    assert.deepEqual(await run({ ...context, inputs }), {
      applied: [{ repository: issue.repository, issue_number: 7, labels: [label] }],
    });
  }
  assert.equal(calls.length, 4);
  const post = calls[1];
  assert.equal(
    post.url,
    "https://api.github.com/repos/example/service/issues/7/labels",
  );
  assert.equal(post.options.method, "POST");
  assert.deepEqual(JSON.parse(post.options.body), { labels: [label] });
  assert.equal(post.options.headers.Authorization, "Bearer test-token");
});

test("creates the repository label if missing, including concurrent creation", async () => {
  for (const creationStatus of [201, 422]) {
    const calls = [];
    const run = createLabeler({
      fetch: async (url, options) => {
        calls.push({ url, options });
        if (calls.length === 1) return reply(null, 404);
        if (url.endsWith("/service/labels")) return reply({}, creationStatus);
        return reply([{ name: label }]);
      },
    });
    assert.equal((await run(context)).applied[0].labels[0], label);
    assert.equal(calls[0].url.endsWith("/labels/relayfold%3Apr-created"), true);
    assert.equal(JSON.parse(calls[1].options.body).name, label);
    assert.equal(calls.length, creationStatus === 201 ? 3 : 4);
  }
});

test("no PR means no label and no GitHub requests", async () => {
  const run = createLabeler({
    fetch: () => assert.fail("No network request expected"),
  });
  assert.deepEqual(
    await run({
      ...context,
      inputs: [issue, { ...pr, pr_created: false, pr_number: undefined }],
    }),
    { applied: [] },
  );
});

test("rejects missing evidence or mismatched identifiers before mutations", async () => {
  const run = createLabeler({
    fetch: () => assert.fail("No network request expected"),
  });
  for (const inputs of [
    [],
    [issue],
    [pr],
    [issue, { ...pr, repository: "other/service" }],
    [issue, { ...pr, issue_number: 8 }],
    [issue, { ...pr, pr_number: undefined }],
    [{ ...issue, repository: "../service" }, { ...pr, repository: "../service" }],
  ])
    await assert.rejects(run({ ...context, inputs }));
});

test("API failures and missing label confirmation fail the task", async () => {
  for (const responses of [
    [reply(null, 403)],
    [reply(null, 404), reply(null, 403)],
    [reply(null, 404), reply(null, 422), reply(null, 404)],
    [reply({}), reply(null, 403)],
    [reply({}), reply([])],
  ]) {
    let index = 0;
    const run = createLabeler({ fetch: async () => responses[index++] });
    await assert.rejects(run(context), /GitHub/);
  }
});

test("applies human-input alone or alongside a PR marker from unresolved questions", async () => {
  const human = "relayfold:human-input-needed";
  for (const pr_created of [false, true]) {
    const expected = pr_created ? [label, human] : [human];
    const posted = [];
    const run = createLabeler({
      fetch: async (url, options) => {
        if (options.method === "POST") posted.push(JSON.parse(options.body));
        return reply(expected.map((name) => ({ name })));
      },
    });
    const result = await run({
      ...context,
      inputs: [issue, { ...pr, pr_created }, {
        repository: issue.repository,
        issue_number: 7,
        open_questions: ["Which retry behavior should be used?"],
      }],
    });
    assert.deepEqual(posted, [{ labels: expected }]);
    assert.deepEqual(result.applied[0].labels, expected);
  }
});

test("CloudWatch publication applies human-input only to flagged new issues", async () => {
  const human = "relayfold:human-input-needed";
  const fingerprint = "a".repeat(64);
  const publish = createPublisher({
    fetch: async (url, options) => reply(options.method === "POST"
      ? { number: 7, html_url: issue.issue_url } : []),
  });
  for (const needed of [false, true]) {
    const publication = await publish({
      credentials: context.credentials,
      inputs: [{
        repository: issue.repository,
        groups: [{ fingerprint, count: 1 }],
        dry_run: false,
      }, { decision: "complete", output: {
        commit_sha: "b".repeat(40),
        summary: "One finding",
        findings: [{
          fingerprint,
          title: "Failure",
          body: "## Problem\nFailure\n## Goal\nFix\n## Acceptance Criteria\nConfirm behavior\n## Notes\nWhich behavior is intended?",
          labels: needed ? [human] : [],
        }],
      } }],
    });
    const calls = [];
    const apply = createLabeler({
      fetch: async (url, options) => {
        calls.push({ url, options });
        return reply([{ name: human }]);
      },
    });
    const result = await apply({ ...context, inputs: [publication] });
    assert.equal(result.applied.length, needed ? 1 : 0);
    assert.equal(calls.length, needed ? 2 : 0);
    if (needed) assert.deepEqual(JSON.parse(calls[1].options.body), { labels: [human] });
  }
  const apply = createLabeler({ fetch: () => assert.fail("No network call") });
  assert.deepEqual(await apply({ inputs: [{ created: [], dry_run: true }] }), { applied: [] });
});

test("human-input marking blocks fetch until a human removes the label", async () => {
  const human = "relayfold:human-input-needed";
  const { createIssueFetcher } = await import("../../github_issue_to_pr/functions/src/fetch-issue.mjs");
  let blocked = true;
  const run = createIssueFetcher({
    fetch: async (url) => reply(url.includes("/comments")
      ? [{ body: "Use retries with a limit of three." }]
      : {
        number: 7, title: "Fix", html_url: issue.issue_url, state: "open",
        labels: [{ name: "relayfold" }, ...(blocked ? [{ name: human }] : [])],
      }),
  });
  await assert.rejects(run({ ...context, inputs: [issue] }), /human-input-needed/);
  blocked = false;
  assert.deepEqual((await run({ ...context, inputs: [issue] })).comments,
    ["Use retries with a limit of three."]);
});

test("labeling prevents subsequent pickup through the generated artifacts", async () => {
  async function artifact(name) {
    const definition = JSON.parse(
      await readFile(
        new URL(
          name === "apply_labels"
            ? "../dist/github.apply_labels.json"
            : "../../github_issue_to_pr/functions/dist/github-issue-to-pr.fetch_issue.json",
          import.meta.url,
        ),
        "utf8",
      ),
    );
    assert.deepEqual(definition.dependencies, []);
    return import(
      `data:text/javascript;base64,${Buffer.from(definition.code).toString("base64")}`
    );
  }
  const labeler = await artifact("apply_labels");
  const fetcher = await artifact("fetch_issue");
  const labels = [{ name: "relayfold" }];
  await labeler.createLabeler({
    fetch: async (url, options) => {
      if (options.method === "POST") labels.push({ name: label });
      return reply(labels);
    },
  })(context);
  const run = fetcher.createIssueFetcher({
    fetch: async (url) => {
      assert(!url.includes("/comments"));
      return reply({
        number: 7,
        title: "Fix",
        html_url: issue.issue_url,
        state: "open",
        labels,
      });
    },
  });
  await assert.rejects(
    run({ ...context, inputs: [issue] }),
    /relayfold:pr-created/,
  );
});
