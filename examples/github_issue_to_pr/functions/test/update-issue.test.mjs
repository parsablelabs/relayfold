import assert from "node:assert/strict";
import test from "node:test";
import { createIssueUpdater } from "../src/update-issue.mjs";
import { readFile } from "node:fs/promises";

const identifiers = { repository: "example/service", issue_number: 7 };
const issue = {
  ...identifiers,
  issue_url: "https://github.com/example/service/issues/7",
};
const pr = {
  ...identifiers,
  pr_created: true,
  pr_number: 12,
  pr_url: "https://github.com/example/service/pull/12",
  summary: "Published the changes.",
};
const implementation = {
  ...identifiers,
  summary: "Added validation.",
  tests_run: ["npm test: passed"],
  open_questions: [],
};
const context = {
  inputs: [issue, pr, implementation],
  credentials: { gh_token: "test-token" },
};
const reply = (json, status = 201) => ({
  ok: status < 400,
  status,
  json: async () => json,
});

test("publishes identifiable summaries for new and reused PRs with requested changes and checks", async () => {
  for (const pr_created of [true, false]) {
    let posts = 0;
    const run = createIssueUpdater({
      fetch: async (url, options) => {
        posts++;
        assert.equal(
          url,
          issue.issue_url.replace("github.com", "api.github.com/repos") +
            "/comments",
        );
        assert.equal(options.method, "POST");
        assert.equal(options.headers.Authorization, "Bearer test-token");
        const { body } = JSON.parse(options.body);
        assert(body.includes(pr_created ? "PR created" : "PR updated"));
        for (const text of [
          "<!-- relayfold:issue-to-pr -->",
          implementation.summary,
          pr.pr_url,
          "npm test: passed",
          "Request: https://github.com/example/service/pull/12#issuecomment-10",
        ])
          assert(body.includes(text));
        return reply({ id: 101, body });
      },
    });
    const result = await run({
      ...context,
      inputs: [
        implementation,
        { ...pr, pr_created },
        {
          ...issue,
          change_request: {
            body: "Add validation",
            url: pr.pr_url + "#issuecomment-10",
          },
        },
      ],
    });
    assert.equal(result.commented, true);
    assert.equal(result.pr_number, 12);
    assert.equal(posts, 1);
  }
});

test("acknowledges successful no-work without inventing a PR", async () => {
  const result = await createIssueUpdater({
    fetch: async (url, options) => {
      const { body } = JSON.parse(options.body);
      assert(body.includes("No PR needed"));
      return reply({ id: 1, body });
    },
  })({
    ...context,
    inputs: [
      issue,
      { ...identifiers, pr_created: false, summary: "Already implemented." },
      implementation,
    ],
  });
  assert.equal("pr_number" in result, false);
});

test("unresolved questions, missing publication, and inconsistent identifiers never post", async () => {
  const run = createIssueUpdater({
    fetch: () => assert.fail("No request expected"),
  });
  for (const inputs of [
    [issue, implementation],
    [issue, pr, { ...implementation, open_questions: ["Which behavior?"] }],
    [issue, { ...pr, issue_number: 8 }, implementation],
    [
      issue,
      { ...pr, pr_url: "https://github.com/other/repo/pull/12" },
      implementation,
    ],
    [issue, { ...pr, pr_number: undefined, pr_url: undefined }, implementation],
    [
      { ...issue, pr: { pr_number: 99 } },
      { ...pr, pr_created: false },
      implementation,
    ],
    [
      { ...issue, pr: {} },
      { ...identifiers, pr_created: false, summary: "No PR" },
      implementation,
    ],
  ]) {
    await assert.rejects(run({ ...context, inputs }));
  }
});

test("failed or uncertain publication never returns success and is not retried", async () => {
  for (const response of [
    reply(null, 403),
    reply({ id: 1, body: "wrong" }),
    new Error("Connection lost"),
  ]) {
    let calls = 0;
    const run = createIssueUpdater({
      fetch: async () => {
        calls++;
        if (response instanceof Error) throw response;
        return response;
      },
    });
    await assert.rejects(run(context));
    assert.equal(calls, 1);
  }
});

test("generated summary Function has no dependencies and publishes a marked comment", async () => {
  const artifact = JSON.parse(
    await readFile(
      new URL("../dist/github-issue-to-pr.update_issue.json", import.meta.url),
      "utf8",
    ),
  );
  assert.deepEqual(artifact.dependencies, []);
  const module = await import(
    `data:text/javascript;base64,${Buffer.from(artifact.code).toString("base64")}`
  );
  assert.equal(
    (
      await module.createIssueUpdater({
        fetch: async (url, options) =>
          reply({ id: 1, body: JSON.parse(options.body).body }),
      })(context)
    ).commented,
    true,
  );
});
