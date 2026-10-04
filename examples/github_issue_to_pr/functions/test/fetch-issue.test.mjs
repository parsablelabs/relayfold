import assert from "node:assert/strict";
import test from "node:test";
import { readFile } from "node:fs/promises";
import { createIssueFetcher } from "../src/fetch-issue.mjs";

const context = {
  inputs: [{ repository: "example/service", issue_number: 7 }],
  credentials: { gh_token: "test-token" },
};
const issue = {
  number: 7,
  title: "Fix failure",
  body: "## Acceptance Criteria\n- Recover correctly",
  html_url: "https://github.com/example/service/issues/7",
  state: "open",
  labels: [{ name: "relayfold" }, { name: "bug" }],
};
const reply = (json, status = 200) => ({
  ok: status < 400,
  status,
  json: async () => json,
});

test("fetches a labeled issue and all comment pages without an LLM", async () => {
  const requests = [];
  const run = createIssueFetcher({
    fetch: async (url, options) => {
      requests.push({ url, options });
      if (!url.includes("/comments")) return reply(issue);
      return reply(
        url.endsWith("page=1")
          ? Array.from({ length: 100 }, () => ({ body: "Discussion" }))
          : [{ body: "Final clarification" }],
      );
    },
  });
  const result = await run(context);
  assert.deepEqual(result, {
    repository: "example/service",
    issue_number: 7,
    issue_url: issue.html_url,
    title: issue.title,
    state: "OPEN",
    body: issue.body,
    comments: [...Array(100).fill("Discussion"), "Final clarification"],
  });
  assert.equal(requests.length, 3);
  assert.equal(
    requests[0].url,
    "https://api.github.com/repos/example/service/issues/7",
  );
  assert(requests[2].url.endsWith("/comments?per_page=100&page=2"));
  for (const { options } of requests) {
    assert.equal(options.headers.Authorization, "Bearer test-token");
    assert.equal(options.method, undefined);
  }
});

test("rejects unlabeled issues and pull requests before fetching comments", async () => {
  for (const changed of [
    { labels: [] },
    { labels: [{ name: "bug" }] },
    { labels: [{ name: "RelayFold" }] },
    { labels: [{ name: "relayfold" }, { name: "relayfold:pr-created" }] },
    { labels: [{ name: "relayfold" }, { name: "relayfold:human-input-needed" }] },
    { pull_request: {} },
  ]) {
    let calls = 0;
    const run = createIssueFetcher({
      fetch: async () => {
        calls++;
        return reply({ ...issue, ...changed });
      },
    });
    await assert.rejects(run(context), /relayfold|pull request/);
    assert.equal(calls, 1);
  }
});

test("validates input before making GitHub requests", async () => {
  const run = createIssueFetcher({
    fetch: () => assert.fail("No network call expected"),
  });
  for (const input of [
    {},
    { repository: "example/service?other", issue_number: 7 },
    { repository: "example/service", issue_number: 0 },
    { repository: "example/service", issue_number: "7" },
    { repository: "example/service", issue_number: 1.5 },
  ])
    await assert.rejects(run({ ...context, inputs: [input] }));
});

test("fails rather than returning incomplete or invalid GitHub data", async () => {
  await assert.rejects(
    createIssueFetcher({ fetch: async () => reply(null, 404) })(context),
    /404/,
  );
  await assert.rejects(
    createIssueFetcher({ fetch: async () => reply({}) })(context),
    /Invalid GitHub issue/,
  );
  for (const comments of [reply(null, 403), reply({ message: "invalid" })])
    await assert.rejects(
      createIssueFetcher({
        fetch: async (url) =>
          url.includes("/comments") ? comments : reply(issue),
      })(context),
      /403|Invalid GitHub comments/,
    );
  await assert.rejects(
    createIssueFetcher({
      fetch: async (url) =>
        reply(
          url.includes("/comments")
            ? Array.from({ length: 100 }, () => ({ body: "Comment" }))
            : issue,
        ),
    })(context),
    /pagination limit/,
  );
});

test("generated Function preserves empty bodies and comments without dependencies", async () => {
  const artifact = JSON.parse(
    await readFile(
      new URL("../dist/github-issue-to-pr.fetch_issue.json", import.meta.url),
      "utf8",
    ),
  );
  assert.deepEqual(artifact.dependencies, []);
  const module = await import(
    `data:text/javascript;base64,${Buffer.from(artifact.code).toString("base64")}`
  );
  const result = await module.createIssueFetcher({
    fetch: async (url) =>
      reply(
        url.includes("/comments")
          ? [{ body: null }]
          : { ...issue, body: null, state: "closed" },
      ),
  })(context);
  assert.equal(result.body, "");
  assert.equal(result.state, "CLOSED");
  assert.deepEqual(result.comments, [""]);
});
