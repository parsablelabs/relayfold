const positiveInteger = (value) => Number.isSafeInteger(value) && value > 0;
const feedbackId = (value) => typeof value === "string" &&
  /^(discussion|inline|review):[1-9][0-9]*$/.test(value);

export function createIssueStatusUpdater({ fetch = globalThis.fetch } = {}) {
  return async function run({ inputs, credentials }) {
    const issue = inputs.find((input) => typeof input?.issue_url === "string");
    const publication = inputs.find((input) => typeof input?.pr_created === "boolean");
    const implementation = inputs.find((input) => Array.isArray(input?.tests_run));
    if (!issue || !publication || !implementation ||
        !/^[A-Za-z0-9_-]+\/[A-Za-z0-9_.-]+$/.test(issue.repository) ||
        !positiveInteger(issue.issue_number) || [publication, implementation].some((input) =>
          input.repository !== issue.repository || input.issue_number !== issue.issue_number))
      throw new Error("Issue, implementation, and publication must identify the same GitHub issue");
    if (!Array.isArray(implementation.open_questions) || implementation.open_questions.length)
      throw new Error("Resolve open questions through ask_user before completing the issue");
    if (typeof implementation.summary !== "string" || typeof publication.summary !== "string" ||
        !implementation.tests_run.every((entry) => typeof entry === "string"))
      throw new Error("Invalid implementation or publication summary");

    const { repository, issue_number } = issue;
    const commentId = issue.status_comment_id;
    const pr = publication.pr_number ?? null;
    const prUrl = pr === null ? undefined : `https://github.com/${repository}/pull/${pr}`;
    if ((pr !== null && !positiveInteger(pr)) ||
        publication.pr_url !== prUrl ||
        (issue.pr && (pr !== issue.pr.pr_number || publication.pr_created)) ||
        (!issue.pr && publication.pr_created !== (pr !== null)))
      throw new Error("Publication did not identify the expected PR");
    if (commentId !== undefined && !positiveInteger(commentId))
      throw new Error("Invalid RelayFold status comment ID");

    const handled = [...new Set([
      ...(issue.handled_feedback ?? []),
      ...(issue.feedback?.map(({ id }) => id) ?? []),
    ])];
    if (!handled.every(feedbackId)) throw new Error("Invalid RelayFold feedback ID");

    const prefix = "<!-- relayfold:issue-to-pr {";
    const marker = `<!-- relayfold:issue-to-pr ${JSON.stringify({ pr, handled })} -->`;
    const heading = pr === null ? "No PR needed" : publication.pr_created ? "PR created" : "PR updated";
    const summary = [
      `### RelayFold — ${heading}`,
      implementation.summary,
      publication.summary,
      ...(prUrl ? [`PR: [#${pr}](${prUrl})`] : []),
      ...(issue.feedback?.length ? [`Processed ${issue.feedback.length} new feedback item(s).`] : []),
      ...(implementation.tests_run.length
        ? [`Checks:\n${implementation.tests_run.map((entry) => `- ${entry}`).join("\n")}`]
        : []),
    ].join("\n\n");
    const result = (id) => ({
      repository,
      issue_number,
      commented: true,
      status_comment_id: id,
      comment_summary: implementation.summary,
      ...(prUrl ? { pr_number: pr, pr_url: prUrl } : {}),
    });
    const apiRoot = "https://api.github.com";
    const issueComments = `/repos/${repository}/issues/${issue_number}/comments`;
    async function request(path, options = {}) {
      const response = await fetch(apiRoot + path, {
        ...options,
        headers: {
          Authorization: "Bearer " + credentials.gh_token,
          Accept: "application/vnd.github+json",
          "Content-Type": "application/json",
          "X-GitHub-Api-Version": "2022-11-28",
        },
        signal: AbortSignal.timeout(30000),
      });
      if (!response.ok) throw new Error("GitHub status comment request failed: " + response.status);
      return response.json();
    }

    // A retry may follow a successful POST whose response never reached the worker.
    if (commentId === undefined) {
      const actor = await request("/user");
      if (!positiveInteger(actor?.id)) throw new Error("Invalid authenticated GitHub account");
      for (let page = 1; ; page++) {
        if (page > 100) throw new Error("GitHub comment pagination limit reached");
        const comments = await request(`${issueComments}?per_page=100&page=${page}`);
        if (!Array.isArray(comments)) throw new Error("Invalid GitHub comments response");
        for (const comment of comments) {
          if (comment.user?.id !== actor.id || !comment.body?.startsWith(prefix)) continue;
          if (comment.body.startsWith(marker) && positiveInteger(comment.id))
            return result(comment.id);
          throw new Error("Issue status changed since fetching; start a fresh run");
        }
        if (comments.length < 100) break;
      }
    }

    const path = commentId === undefined
      ? issueComments
      : `/repos/${repository}/issues/comments/${commentId}`;
    const comment = await request(path, {
      method: commentId === undefined ? "POST" : "PATCH",
      body: JSON.stringify({ body: `${marker}\n${summary}` }),
    });
    if (!positiveInteger(comment?.id) || (commentId !== undefined && comment.id !== commentId) ||
        !comment.body?.startsWith(marker))
      throw new Error("GitHub did not confirm the RelayFold status comment");
    return result(comment.id);
  };
}

export default createIssueStatusUpdater();
