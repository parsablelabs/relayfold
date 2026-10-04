import assert from "node:assert/strict";
import test from "node:test";
import { readFile } from "node:fs/promises";
import { createLabeler } from "../src/mark-pr-created.mjs";

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
      repository: issue.repository,
      issue_number: 7,
      labeled: true,
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
    assert.equal((await run(context)).labeled, true);
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
    { repository: issue.repository, issue_number: 7, labeled: false },
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

test("labeling prevents subsequent pickup through the generated artifacts", async () => {
  async function artifact(name) {
    const definition = JSON.parse(
      await readFile(
        new URL(`../dist/github-issue-to-pr.${name}.json`, import.meta.url),
        "utf8",
      ),
    );
    assert.deepEqual(definition.dependencies, []);
    return import(
      `data:text/javascript;base64,${Buffer.from(definition.code).toString("base64")}`
    );
  }
  const labeler = await artifact("mark_pr_created");
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
