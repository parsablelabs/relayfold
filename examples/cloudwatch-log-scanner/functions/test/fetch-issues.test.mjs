import assert from "node:assert/strict";
import test from "node:test";
import { createIssueFetcher } from "../src/fetch-issues.mjs";
import { readFile } from "node:fs/promises";
const context = {
  inputs: [{ repository: "example/service", targets: [] }],
  credentials: { gh_token: "test-token" },
};
const reply = (json, status = 200) => ({
  ok: status < 400,
  status,
  json: async () => json,
});

test("fetches open relayfold issues across pages, excluding pull requests", async () => {
  const requests = [];
  const run = createIssueFetcher({
    fetch: async (url, options) => {
      requests.push({ url, options });
      return reply(
        requests.length === 1
          ? Array.from({ length: 100 }, () => ({
              pull_request: {},
              title: "PR",
            }))
          : [
              {
                number: 7,
                title: "Existing failure",
                body: null,
                html_url: "https://github.com/example/service/issues/7",
                extra: "unused",
              },
            ],
      );
    },
  });
  const result = await run(context);
  assert.equal(result.repository, "example/service");
  assert.deepEqual(result.issues, [
    {
      number: 7,
      title: "Existing failure",
      body: "",
      html_url: "https://github.com/example/service/issues/7",
    },
  ]);
  assert.equal(requests.length, 2);
  requests.forEach(({ url, options }, index) => {
    const parsed = new URL(url);
    assert.equal(parsed.pathname, "/repos/example/service/issues");
    assert.equal(parsed.searchParams.get("state"), "open");
    assert.equal(parsed.searchParams.get("labels"), "relayfold");
    assert.equal(parsed.searchParams.get("page"), String(index + 1));
    assert.equal(options.headers.Authorization, "Bearer test-token");
    assert.equal(options.method, undefined);
  });
});

test("returns an empty issue list when none match", async () => {
  const run = createIssueFetcher({ fetch: async () => reply([]) });
  assert.deepEqual(await run(context), {
    repository: "example/service",
    issues: [],
  });
});

test("fails on incomplete or invalid issue inspection", async () => {
  let calls = 0;
  const run = createIssueFetcher({
    fetch: async () =>
      ++calls === 1
        ? reply(Array.from({ length: 100 }, () => ({ pull_request: {} })))
        : reply(null, 403),
  });
  await assert.rejects(run(context), /403/);
  await assert.rejects(
    createIssueFetcher({ fetch: async () => reply({ message: "invalid" }) })(
      context,
    ),
    /Invalid GitHub/,
  );
  await assert.rejects(
    createIssueFetcher({
      fetch: async () =>
        reply(Array.from({ length: 100 }, () => ({ pull_request: {} }))),
    })(context),
    /pagination limit/,
  );
});

test("rejects invalid repositories before network requests", async () => {
  const run = createIssueFetcher({
    fetch: () => assert.fail("No request expected"),
  });
  for (const repository of [
    undefined,
    "../service",
    "example/service?other",
    "https://github.com/example/service",
  ]) {
    await assert.rejects(
      run({ ...context, inputs: [{ repository }] }),
      /GitHub owner\/repo/,
    );
  }
});

test("generated issue fetcher runs with injected GitHub responses", async () => {
  const artifact = JSON.parse(
    await readFile(
      new URL(
        "../dist/cloudwatch-log-scanner.fetch_issues.json",
        import.meta.url,
      ),
      "utf8",
    ),
  );
  assert.deepEqual(artifact.dependencies, []);
  const module = await import(
    `data:text/javascript;base64,${Buffer.from(artifact.code).toString("base64")}`
  );
  const run = module.createIssueFetcher({ fetch: async () => reply([]) });
  assert.deepEqual(await run(context), {
    repository: "example/service",
    issues: [],
  });
});
