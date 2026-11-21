import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import { createIssueAndPrFetcher } from "../src/fetch-issue-and-pr.mjs";

const repository = "example/service";
const credentials = { gh_token: "test-token" };
const polling = { inputs: [{ repository }], credentials };
const issue = {
  number: 7, title: "Fix failure", state: "open", labels: ["relayfold", "relayfold:pr-created"],
  html_url: `https://github.com/${repository}/issues/7`,
};
const pull = {
  number: 12, title: "Fix failure", state: "open", head: { ref: "relayfold/issue-7", repo: { full_name: repository } },
  html_url: `https://github.com/${repository}/pull/12`,
};
const actor = { id: 999 };
const feedback = (body, extra = {}) => ({
  id: 1, body, author_association: "COLLABORATOR", user: { id: 5, type: "User" }, ...extra,
});
const json = (value) => Response.json(value);
const base = `/repos/${repository}`;
const publication = (pr_created = false) => ({
  repository, issue_number: 7, pr_created, pr_number: 12, pr_url: pull.html_url, summary: "Published.",
});

const implementation = {
  repository, issue_number: 7, summary: "Updated validation.", tests_run: [], open_questions: [],
};

async function loadUpdater() {
  const { code } = JSON.parse(await readFile(new URL(
    "../dist/github-issue-to-pr.update_issue_status.json", import.meta.url,
  ), "utf8"));
  return (await import(`data:text/javascript;base64,${Buffer.from(code).toString("base64")}`)).createIssueStatusUpdater;
}

test("records the PR, batches unseen feedback, and acknowledges it only after a successful update", async () => {
  const comments = [];
  const discussion = [];
  const reviews = [];
  let writeFailure = false;
  const fetch = async (url, options = {}) => {
    const { pathname } = new URL(url);
    if (pathname === "/user") return json(actor);
    if (pathname === base + "/issues") return json([issue]);
    if (pathname === base + "/issues/7") return json(issue);
    if (pathname === base + "/pulls/12") return json(pull);
    if (pathname === base + "/issues/12/comments") return json(discussion);
    if (pathname === base + "/pulls/12/comments") return json([]);
    if (pathname === base + "/pulls/12/reviews") return json(reviews);
    if (pathname === base + "/issues/7/comments" && !options.method) return json(comments);
    if (writeFailure) return new Response(null, { status: 503 });
    assert.equal(pathname, base + (options.method === "PATCH" ? "/issues/comments/42" : "/issues/7/comments"));
    const { body } = JSON.parse(options.body);
    const comment = { id: 42, body, user: actor };
    if (options.method === "PATCH") comments[0] = comment;
    else comments.push(comment);
    return json(comment);
  };

  const update = (await loadUpdater())({ fetch });
  const poll = createIssueAndPrFetcher({ fetch });
  const initial = { inputs: [{ repository, issue_number: 7, issue_url: issue.html_url }, publication(true), implementation], credentials };
  await update(initial);
  await update(initial); // Retrying the creation does not post another comment.
  assert.equal(comments.length, 1);
  assert.match(comments[0].body, /\[#12\]/);
  discussion.push(feedback("Fix the edge case"), feedback("Bot", { id: 2, user: { type: "Bot" } }),
    feedback("Drive-by request", { id: 3, author_association: "NONE" }),
    feedback("Automation account", { id: 4, user: actor }));
  reviews.push(feedback("Add a regression test"));
  const job = await poll(polling);
  assert.equal(job.pr.pr_number, 12);
  assert.deepEqual(job.feedback.map(({ id }) => id), ["discussion:1", "review:1"]);
  const followUp = { inputs: [job, publication(), implementation], credentials };
  writeFailure = true;
  await assert.rejects(update(followUp), /503/);
  assert.deepEqual((await poll(polling)).feedback, job.feedback);
  writeFailure = false;
  discussion.push(feedback("Handle canceled jobs too", { id: 5 }));
  await update(followUp);
  const nextJob = await poll(polling);
  assert.deepEqual(nextJob.feedback.map(({ id }) => id), ["discussion:5"]);
  await update({ inputs: [nextJob, publication(), implementation], credentials });
  assert.ok((await poll(polling))._workflow_exit_reason);
  assert.equal(comments.length, 1);
});

test("recovers a PR from its issue branch and skips it when closed", async () => {
  let state = "open";
  const poll = createIssueAndPrFetcher({ fetch: async (url) => {
    const { pathname, searchParams } = new URL(url);
    if (pathname === "/user") return json(actor);
    if (pathname === base + "/issues") return json([issue]);
    if (pathname === base + "/issues/7") return json(issue);
    if (pathname === base + "/issues/7/comments")
      return json([{ id: 42, user: { id: 7 }, body: '<!-- relayfold:issue-to-pr {"pr":99,"handled":[]} -->' }]);
    if (pathname === base + "/pulls") {
      assert.equal(searchParams.get("head"), "example:relayfold/issue-7");
      return json([pull]);
    }
    if (pathname === base + "/pulls/12") return json({ ...pull, state });
    assert.equal(state, "open", "Closed PRs should not be polled for feedback");
    return json(pathname === base + "/issues/12/comments" ? [feedback("Adjust the tests")] : []);
  } });
  assert.equal((await poll(polling)).pr.pr_number, 12);
  state = "closed";
  assert.ok((await poll(polling))._workflow_exit_reason);
});

test("finds an older PR from a summary authored by the workflow account", async () => {
  const legacy = (number, user) => ({
    user,
    body: `### RelayFold — PR created\n\n<!-- relayfold:issue-to-pr -->\n\n` +
      `PR #${number}: https://github.com/${repository}/pull/${number}`,
  });
  const poll = createIssueAndPrFetcher({ fetch: async (url) => {
    const { pathname } = new URL(url);
    if (pathname === "/user") return json(actor);
    if (pathname === base + "/issues") return json([issue]);
    if (pathname === base + "/issues/7") return json(issue);
    if (pathname === base + "/issues/7/comments")
      return json([legacy(12, actor), legacy(99, { id: 5 })]);
    if (pathname === base + "/pulls/12") return json(pull);
    if (pathname === base + "/issues/12/comments") return json([feedback("Add a test")]);
    if ([base + "/pulls/12/comments", base + "/pulls/12/reviews"].includes(pathname)) return json([]);
    assert.fail(`Unexpected request to ${pathname}`);
  } });
  const job = await poll(polling);
  assert.equal(job.pr.pr_number, 12);
  assert.deepEqual(job.feedback.map(({ id }) => id), ["discussion:1"]);
});

test("skips completed issues that needed no PR and continues scanning", async () => {
  const nextIssue = { ...issue, number: 8, labels: ["relayfold:agent-ready"] };
  const poll = createIssueAndPrFetcher({ fetch: async (url) => {
    const { pathname } = new URL(url);
    if (pathname === "/user") return json(actor);
    if (pathname === base + "/issues") return json([issue, nextIssue]);
    if (pathname === base + "/issues/7") return json(issue);
    if (pathname === base + "/issues/8") return json(nextIssue);
    if (pathname === base + "/pulls") return json([]);
    if (pathname === base + "/issues/7/comments")
      return json([{ id: 42, user: actor, body: '<!-- relayfold:issue-to-pr {"pr":null,"handled":[]} -->' }]);
    return json([]);
  } });
  assert.equal((await poll(polling)).issue_number, 8);
});
