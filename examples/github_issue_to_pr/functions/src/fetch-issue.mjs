export function createIssueFetcher({ fetch = globalThis.fetch } = {}) {
  return async function run({ inputs, credentials }) {
    const { repository, issue_number } = inputs[0] ?? {};
    if (
      typeof repository !== "string" ||
      !/^[A-Za-z0-9_-]+\/[A-Za-z0-9_.-]+$/.test(repository)
    )
      throw new Error("Specify repository as GitHub owner/repo");
    if (!Number.isSafeInteger(issue_number) || issue_number < 1)
      throw new Error("Specify a positive integer issue_number");

    const base = `https://api.github.com/repos/${repository}/issues/${issue_number}`;
    async function api(suffix = "") {
      const response = await fetch(base + suffix, {
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

    const issue = await api();
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

    const comments = [];
    for (let page = 1; ; page++) {
      if (page > 100) throw new Error("GitHub comment pagination limit reached");
      const batch = await api(`/comments?per_page=100&page=${page}`);
      if (!Array.isArray(batch))
        throw new Error("Invalid GitHub comments response");
      comments.push(...batch.map((comment) => comment.body ?? ""));
      if (batch.length < 100) break;
    }

    return {
      repository,
      issue_number: issue.number,
      issue_url: issue.html_url,
      title: issue.title,
      state: issue.state.toUpperCase(),
      body: issue.body ?? "",
      comments,
    };
  };
}

export default createIssueFetcher();
