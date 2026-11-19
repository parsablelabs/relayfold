import assert from "node:assert/strict";
import test from "node:test";
import { readFile } from "node:fs/promises";
import { createLabeler } from "../src/apply-labels.mjs";

const marker = "relayfold:pr-created";
const ready = "relayfold:agent-ready";
const issue = {
  repository: "example/service",
  issue_number: 7,
  issue_url: "https://github.com/example/service/issues/7",
};
const pr = {
  repository: issue.repository,
  issue_number: 7,
  pr_created: true,
  pr_number: 12,
};
const summary = {
  repository: issue.repository,
  issue_number: 7,
  commented: true,
  pr_number: 12,
};
const context = {
  inputs: [issue, pr, summary],
  credentials: { gh_token: "test-token" },
};
const reply = (json, status = 200) => ({
  ok: status < 400,
  status,
  json: async () => json,
});

test("marks new or reused PRs before consuming agent-ready without replacing other labels", async () => {
  for (const pr_created of [true, false]) {
    const calls = [];
    const run = createLabeler({
      fetch: async (url, options) => {
        calls.push({ url, options });
        return reply([{ name: "bug" }, { name: marker }]);
      },
    });
    assert.deepEqual(
      await run({
        ...context,
        inputs: [summary, issue, { ...pr, pr_created }],
      }),
      {
        applied: [
          { repository: issue.repository, issue_number: 7, labels: [marker] },
        ],
        removed: [
          { repository: issue.repository, issue_number: 7, labels: [ready] },
        ],
      },
    );
    assert.equal(calls.length, 3);
    assert.equal(calls[1].options.method, "POST");
    assert.deepEqual(JSON.parse(calls[1].options.body), { labels: [marker] });
    assert.equal(calls[2].options.method, "DELETE");
    assert(calls[2].url.endsWith("/issues/7/labels/relayfold%3Aagent-ready"));
    assert.equal(calls[2].options.headers.Authorization, "Bearer test-token");
  }
});

test("creates missing PR marker, including concurrent creation", async () => {
  for (const creationStatus of [201, 422]) {
    const calls = [];
    const run = createLabeler({
      fetch: async (url, options) => {
        calls.push({ url, options });
        if (calls.length === 1) return reply(null, 404);
        if (url.endsWith("/service/labels")) return reply({}, creationStatus);
        return reply([{ name: marker }]);
      },
    });
    await run(context);
    assert.equal(JSON.parse(calls[1].options.body).name, marker);
    assert.equal(calls.length, creationStatus === 201 ? 4 : 5);
  }
});

test("successful no-work result consumes readiness without marking a PR", async () => {
  const calls = [];
  const result = await createLabeler({
    fetch: async (url, options) => {
      calls.push(options.method);
      return reply([]);
    },
  })({
    ...context,
    inputs: [
      issue,
      { ...pr, pr_created: false, pr_number: undefined },
      { ...summary, pr_number: undefined },
    ],
  });
  assert.deepEqual(result.applied, []);
  assert.deepEqual(calls, ["DELETE"]);
});

test("requires a successful summary and consistent identifiers before mutations", async () => {
  const run = createLabeler({
    fetch: () => assert.fail("No request expected"),
  });
  for (const inputs of [
    [],
    [issue, pr],
    [issue, pr, { ...summary, commented: false }],
    [issue, { ...pr, issue_number: 8 }, summary],
    [issue, pr, { ...summary, repository: "other/repo" }],
    [issue, pr, { ...summary, pr_number: 99 }],
    [issue, { ...pr, pr_number: undefined }, summary],
    [
      { ...issue, pr: { pr_number: 99 } },
      { ...pr, pr_created: false },
      summary,
    ],
    [{ ...issue, repository: "../bad" }, pr, summary],
  ]) {
    await assert.rejects(run({ ...context, inputs }));
  }
});

test("marker failures never consume readiness and removal failures fail the task", async () => {
  for (const responses of [
    [reply(null, 403)],
    [reply(null, 404), reply(null, 403)],
    [reply({}), reply([])],
    [reply({}), reply([{ name: marker }]), reply(null, 403)],
    [reply({}), reply([{ name: marker }]), reply([{ name: ready }])],
  ]) {
    let index = 0;
    await assert.rejects(
      createLabeler({ fetch: async () => responses[index++] })(context),
      /GitHub/,
    );
    assert.equal(index, responses.length);
  }
});

test("generated functions consume readiness and the next fetch exits", async () => {
  async function artifact(path) {
    const definition = JSON.parse(
      await readFile(new URL(path, import.meta.url), "utf8"),
    );
    assert.deepEqual(definition.dependencies, []);
    return import(
      `data:text/javascript;base64,${Buffer.from(definition.code).toString("base64")}`
    );
  }
  const labeler = await artifact("../dist/github.apply_labels.json");
  const fetcher = await artifact(
    "../../github_issue_to_pr/functions/dist/github-issue-to-pr.fetch_issue_and_pr.json",
  );
  let labels = [{ name: ready }, { name: "bug" }];
  await labeler.createLabeler({
    fetch: async (url, options) => {
      if (options.method === "POST") labels.push({ name: marker });
      if (options.method === "DELETE")
        labels = labels.filter((label) => label.name !== ready);
      return reply(labels);
    },
  })(context);
  const result = await fetcher.createIssueAndPrFetcher({
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
  })({ ...context, inputs: [issue] });
  assert.match(result._workflow_exit_reason, /agent-ready/);
  assert.deepEqual(labels, [{ name: "bug" }, { name: marker }]);
});

test("repeated readiness removal confirms absence instead of hiding missing issues", async () => {
  for (const status of [200, 404]) {
    let calls = 0;
    const run = createLabeler({
      fetch: async () => {
        calls++;
        return calls === 1 ? reply(null, 404) : reply([], status);
      },
    });
    const inputs = [
      issue,
      { ...pr, pr_created: false, pr_number: undefined },
      { ...summary, pr_number: undefined },
    ];
    if (status === 200)
      assert.equal((await run({ ...context, inputs })).removed.length, 1);
    else await assert.rejects(run({ ...context, inputs }), /404/);
    assert.equal(calls, 2);
  }
});
