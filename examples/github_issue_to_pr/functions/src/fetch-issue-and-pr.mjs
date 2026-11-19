export function createIssueAndPrFetcher({ fetch = globalThis.fetch } = {}) {
  return async function run({ inputs, credentials }) {
    const { repository, issue_number } = inputs[0] ?? {};
    let { pr_number } = inputs[0] ?? {};
    if (
      typeof repository !== "string" ||
      !/^[A-Za-z0-9_-]+\/[A-Za-z0-9_.-]+$/.test(repository)
    )
      throw new Error("Specify repository as GitHub owner/repo");
    if (issue_number !== undefined && (!Number.isSafeInteger(issue_number) || issue_number < 1))
      throw new Error("Specify a positive integer issue_number");

    if (
      pr_number !== undefined &&
      (!Number.isSafeInteger(pr_number) || pr_number < 1)
    )
      throw new Error("Specify a positive integer pr_number");

    if (pr_number !== undefined && issue_number === undefined)
      throw new Error("Specify issue_number when supplying pr_number");

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

    function hasLabel(issue, name) {
      return issue.labels.some((label) => (label?.name ?? label) === name);
    }
    function validateIssue(issue) {
      if (!issue || !Number.isSafeInteger(issue.number) || issue.number < 1 ||
          typeof issue.title !== "string" || typeof issue.html_url !== "string" ||
          !["open", "closed"].includes(issue.state) || !Array.isArray(issue.labels))
        throw new Error("Invalid GitHub issue response");
    }

    let selectedNumber = issue_number;
    if (selectedNumber === undefined) {
      for (let page = 1; ; page++) {
        if (page > 100) throw new Error("GitHub issue pagination limit reached");
        const batch = await api(`/issues?state=open&labels=relayfold:agent-ready&sort=created&direction=asc&per_page=100&page=${page}`);
        if (!Array.isArray(batch)) throw new Error("Invalid GitHub issues response");
        for (const candidate of batch) {
          validateIssue(candidate);
          if (selectedNumber === undefined && candidate.state === "open" &&
              !candidate.pull_request && hasLabel(candidate, "relayfold:agent-ready"))
            selectedNumber = candidate.number;
        }
        if (batch.length < 100) break;
      }
      if (selectedNumber === undefined)
        return {
          repository,
          _workflow_exit_reason: `No eligible open issues labeled "relayfold:agent-ready" in ${repository}`,
        };
    }

    const issue = await api(`/issues/${selectedNumber}`);
    validateIssue(issue);
    if (issue.number !== selectedNumber) throw new Error("Invalid GitHub issue response");
    if (issue_number === undefined && issue.state !== "open")
      throw new Error("Selected issue is no longer open; rerun to select another issue");
    if (issue.pull_request)
      throw new Error(
        "Requested number identifies a pull request, not an issue",
      );
    const issueOutput = {
      repository,
      issue_number: issue.number,
      issue_url: issue.html_url,
      title: issue.title,
      state: issue.state.toUpperCase(),
      body: issue.body ?? "",
      comments: [],
    };
    if (
      !issue.labels.some(
        (label) => (label?.name ?? label) === "relayfold:agent-ready",
      )
    )
      return {
        ...issueOutput,
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
    const comments = await commentBodies(`/issues/${selectedNumber}/comments`);
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
      pr = {
        pr_number,
        pr_url: pull.html_url,
        title: pull.title,
        body: pull.body ?? "",
        state: pull.state.toUpperCase(),
        branch: pull.head.ref,
        comments: discussion.map((comment) => comment.body ?? ""),
        review_comments: reviewComments.map((comment) => comment.body ?? ""),
        reviews: reviews.map((comment) => comment.body ?? ""),
      };
    }

    return {
      ...issueOutput,
      comments,
      ...(pr ? { pr, change_request } : {}),
    };
  };
}

export default createIssueAndPrFetcher();
