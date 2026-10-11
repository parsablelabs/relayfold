export function createIssueAndPrFetcher({ fetch = globalThis.fetch } = {}) {
  return async function run({ inputs, credentials }) {
    const { repository, issue_number } = inputs[0] ?? {};
    let { pr_number } = inputs[0] ?? {};
    if (
      typeof repository !== "string" ||
      !/^[A-Za-z0-9_-]+\/[A-Za-z0-9_.-]+$/.test(repository)
    )
      throw new Error("Specify repository as GitHub owner/repo");
    if (!Number.isSafeInteger(issue_number) || issue_number < 1)
      throw new Error("Specify a positive integer issue_number");

    if (
      pr_number !== undefined &&
      (!Number.isSafeInteger(pr_number) || pr_number < 1)
    )
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
      throw new Error(
        "Requested number identifies a pull request, not an issue",
      );
    if (
      !issue.labels.some(
        (label) => (label?.name ?? label) === "relayfold:agent-ready",
      )
    )
      return {
        _workflow_exit_reason:
          'Issue does not have the "relayfold:agent-ready" label.',
      };

    async function allComments(path) {
      const comments = [];
      for (let page = 1; ; page++) {
        if (page > 100)
          throw new Error("GitHub comment pagination limit reached");
        const batch = await api(`${path}?per_page=100&page=${page}`);
        if (!Array.isArray(batch))
          throw new Error("Invalid GitHub comments response");
        comments.push(...batch);
        if (batch.length < 100) return comments;
      }
    }
    const commentBodies = async (path) =>
      (await allComments(path)).map((comment) => comment.body ?? "");
    const comments = await commentBodies(`/issues/${issue_number}/comments`);
    if (pr_number === undefined) {
      // New summaries carry a stable marker; legacy summaries have no marker but link to the PR.
      const summary = comments.findLast((body) =>
        body.includes("<!-- relayfold:issue-to-pr -->"),
      );
      const candidates = summary ? [summary] : comments;
      const prefix = `https://github.com/${repository}/pull/`;
      const numbers = new Set(
        candidates.flatMap((body) =>
          body
            .split(prefix)
            .slice(1)
            .flatMap((suffix) => {
              const match = suffix.match(/^([1-9][0-9]*)(?![0-9A-Za-z_/])/);
              return match ? [Number(match[1])] : [];
            }),
        ),
      );
      if (numbers.size > 1)
        throw new Error("Multiple PR links on the issue; specify pr_number");
      pr_number = numbers.values().next().value;
      if (
        pr_number === undefined &&
        issue.labels.some(
          (label) => (label?.name ?? label) === "relayfold:pr-created",
        )
      )
        throw new Error(
          "Issue is marked pr-created but no PR link was found; specify pr_number",
        );
    }
    let pr;
    let change_request;
    if (pr_number !== undefined) {
      if (!Number.isSafeInteger(pr_number) || pr_number < 1)
        throw new Error("Invalid PR link; specify pr_number");
      const pull = await api(`/pulls/${pr_number}`);
      if (
        !pull ||
        pull.number !== pr_number ||
        typeof pull.title !== "string" ||
        typeof pull.html_url !== "string" ||
        !["open", "closed"].includes(pull.state) ||
        typeof pull.head?.ref !== "string"
      )
        throw new Error("Invalid GitHub pull request response");
      const discussion = await allComments(`/issues/${pr_number}/comments`);
      const latest = discussion.findLast(
        (comment) =>
          comment.user?.type !== "Bot" &&
          typeof comment.body === "string" &&
          comment.body.trim() &&
          !comment.body.includes("<!-- relayfold:issue-to-pr -->"),
      );
      if (!latest)
        throw new Error(
          "Existing PR has no developer conversation comment; add the requested changes to the PR before rerunning",
        );
      change_request = { body: latest.body, url: latest.html_url ?? "" };
      pr = {
        pr_number,
        pr_url: pull.html_url,
        title: pull.title,
        body: pull.body ?? "",
        state: pull.state.toUpperCase(),
        branch: pull.head.ref,
        comments: discussion.map((comment) => comment.body ?? ""),
        review_comments: await commentBodies(`/pulls/${pr_number}/comments`),
        reviews: await commentBodies(`/pulls/${pr_number}/reviews`),
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
      ...(change_request ? { change_request } : {}),
    };
  };
}

export default createIssueAndPrFetcher();
