export function createIssueUpdater({ fetch = globalThis.fetch } = {}) {
  return async function run({ inputs, credentials }) {
    const issue = inputs.find((input) => typeof input?.issue_url === "string");
    const pr = inputs.find((input) => typeof input?.pr_created === "boolean");
    const implementation = inputs.find((input) =>
      Array.isArray(input?.tests_run),
    );
    if (!issue || !pr || !implementation)
      throw new Error(
        "Missing fetched issue, implementation, or publication result",
      );
    const { repository, issue_number } = issue;
    if (
      typeof repository !== "string" ||
      !/^[A-Za-z0-9_-]+\/[A-Za-z0-9_.-]+$/.test(repository) ||
      !Number.isSafeInteger(issue_number) ||
      issue_number < 1 ||
      [pr, implementation].some(
        (input) =>
          input.repository !== repository ||
          input.issue_number !== issue_number,
      )
    )
      throw new Error("Summary inputs must identify the same valid issue");
    if (
      !Array.isArray(implementation.open_questions) ||
      implementation.open_questions.length
    )
      throw new Error(
        "Resolve open questions through ask_user before completing the issue",
      );
    const hasPr = pr.pr_number !== undefined;
    if (
      (hasPr &&
        (!Number.isSafeInteger(pr.pr_number) ||
          pr.pr_number < 1 ||
          pr.pr_url !==
            `https://github.com/${repository}/pull/${pr.pr_number}`)) ||
      (!hasPr && (pr.pr_created || issue.pr || pr.pr_url !== undefined)) ||
      (issue.pr && (pr.pr_created || pr.pr_number !== issue.pr.pr_number))
    )
      throw new Error(
        "Publication must include a valid PR number and URL when a PR exists",
      );
    if (
      typeof implementation.summary !== "string" ||
      typeof pr.summary !== "string" ||
      !implementation.tests_run.every((entry) => typeof entry === "string")
    )
      throw new Error("Invalid implementation or publication summary");
    const heading = hasPr
      ? pr.pr_created
        ? "PR created"
        : "PR updated"
      : "No PR needed";
    const body = [
      `### RelayFold — ${heading}`,
      "<!-- relayfold:issue-to-pr -->",
      implementation.summary,
      pr.summary,
      ...(hasPr ? [`PR #${pr.pr_number}: ${pr.pr_url}`] : []),
      ...(issue.change_request?.url
        ? [`Request: ${issue.change_request.url}`]
        : []),
      ...(implementation.tests_run.length
        ? [
            `Checks:\n${implementation.tests_run.map((entry) => `- ${entry}`).join("\n")}`,
          ]
        : []),
    ].join("\n\n");
    // An uncertain POST may have succeeded; do not automatically retry it.
    const response = await fetch(
      `https://api.github.com/repos/${repository}/issues/${issue_number}/comments`,
      {
        method: "POST",
        headers: {
          Authorization: "Bearer " + credentials.gh_token,
          Accept: "application/vnd.github+json",
          "Content-Type": "application/json",
          "X-GitHub-Api-Version": "2022-11-28",
        },
        body: JSON.stringify({ body }),
        signal: AbortSignal.timeout(30000),
      },
    );
    if (!response.ok)
      throw new Error("GitHub request failed: " + response.status);
    const comment = await response.json();
    if (!Number.isSafeInteger(comment?.id) || comment.body !== body)
      throw new Error("GitHub did not confirm the issue summary comment");
    return {
      repository,
      issue_number,
      commented: true,
      comment_summary: implementation.summary,
      ...(hasPr ? { pr_number: pr.pr_number, pr_url: pr.pr_url } : {}),
    };
  };
}

export default createIssueUpdater();
