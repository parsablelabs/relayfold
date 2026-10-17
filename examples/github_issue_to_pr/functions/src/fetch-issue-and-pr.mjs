export function createIssueAndPrFetcher({ fetch = globalThis.fetch } = {}) {
  return async function run({ inputs, credentials }) {
    const { repository, issue_number, pr_number } = inputs[0] ?? {};
    if (
      typeof repository !== "string" ||
      !/^[A-Za-z0-9_-]+\/[A-Za-z0-9_.-]+$/.test(repository)
    )
      throw new Error("Specify repository as GitHub owner/repo");
    if (!Number.isSafeInteger(issue_number) || issue_number < 1)
      throw new Error("Specify a positive integer issue_number");

    if (pr_number !== undefined && (!Number.isSafeInteger(pr_number) || pr_number < 1))
      throw new Error("Specify a positive integer pr_number");

    const base = `https://api.github.com/repos/${repository}`;
    async function api(path) {
      const response = await fetch(base + path, {
        headers: {
          Authorization: "Bearer " + credentials.gh_token,
          Accept: "application/vnd.github+json",
          "X-GitHub-Api-Version": "2022-11-28",
        },
        signal: AbortSignal.timeout(30000),
      });
      if (!response.ok)
        throw new Error("GitHub request failed: " + response.status);
      return response.json();
    }

    const issue = await api(`/issues/${issue_number}`);
    if (
      !issue ||
      issue.number !== issue_number ||
      typeof issue.title !== "string" ||
      typeof issue.html_url !== "string" ||
      !["open", "closed"].includes(issue.state) ||
      !Array.isArray(issue.labels)
    )
      throw new Error("Invalid GitHub issue response");
    if (issue.pull_request)
      throw new Error("Requested number identifies a pull request, not an issue");
    if (!issue.labels.some((label) => (label?.name ?? label) === "relayfold"))
      throw new Error('Issue must have the "relayfold" label');
    for (const blocked of ["relayfold:pr-created", "relayfold:human-input-needed"])
      if (issue.labels.some((label) => (label?.name ?? label) === blocked))
        throw new Error(`Issue is excluded by the "${blocked}" label`);

    async function commentBodies(path) {
      const comments = [];
      for (let page = 1; ; page++) {
        if (page > 100) throw new Error("GitHub comment pagination limit reached");
        const batch = await api(`${path}?per_page=100&page=${page}`);
        if (!Array.isArray(batch))
          throw new Error("Invalid GitHub comments response");
        comments.push(...batch.map((comment) => comment.body ?? ""));
        if (batch.length < 100) return comments;
      }
    }
    const comments = await commentBodies(`/issues/${issue_number}/comments`);
    let pr;
    if (pr_number !== undefined) {
      const pull = await api(`/pulls/${pr_number}`);
      if (!pull || pull.number !== pr_number ||
          typeof pull.title !== "string" || typeof pull.html_url !== "string" ||
          !["open", "closed"].includes(pull.state) ||
          typeof pull.head?.ref !== "string")
        throw new Error("Invalid GitHub pull request response");
<<<<<<< Updated upstream
=======
      const discussion = await allComments(`/issues/${pr_number}/comments`);
      const reviewComments = await allComments(`/pulls/${pr_number}/comments`);
      const reviews = await allComments(`/pulls/${pr_number}/reviews`);
      const feedback = [...discussion, ...reviewComments, ...reviews]
        .filter((comment) => comment.state !== "PENDING")
        .sort((a, b) =>
          Date.parse(a.submitted_at ?? a.created_at ?? "1970-01-01") -
          Date.parse(b.submitted_at ?? b.created_at ?? "1970-01-01"),
        );
      const latest = feedback.findLast(
        (comment) =>
          comment.user?.type !== "Bot" &&
          typeof comment.body === "string" &&
          comment.body.trim() &&
          !comment.body.includes("<!-- relayfold:issue-to-pr -->"),
      );
      if (!latest)
        throw new Error(
          "Existing PR has no developer feedback comment or submitted review; add the requested changes to the PR before rerunning",
        );
      change_request = { body: latest.body, url: latest.html_url ?? "" };
>>>>>>> Stashed changes
      pr = {
        pr_number,
        pr_url: pull.html_url,
        title: pull.title,
        body: pull.body ?? "",
        state: pull.state.toUpperCase(),
        branch: pull.head.ref,
<<<<<<< Updated upstream
        comments: await commentBodies(`/issues/${pr_number}/comments`),
        review_comments: await commentBodies(`/pulls/${pr_number}/comments`),
        reviews: await commentBodies(`/pulls/${pr_number}/reviews`),
=======
        comments: discussion.map((comment) => comment.body ?? ""),
        review_comments: reviewComments.map((comment) => comment.body ?? ""),
        reviews: reviews.map((comment) => comment.body ?? ""),
>>>>>>> Stashed changes
      };
    }

    return {
      repository,
      issue_number: issue.number,
      issue_url: issue.html_url,
      title: issue.title,
      state: issue.state.toUpperCase(),
      body: issue.body ?? "",
      comments,
      ...(pr ? { pr } : {}),
    };
  };
}

export default createIssueAndPrFetcher();
