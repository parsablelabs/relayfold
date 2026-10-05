export function createIssueFetcher({ fetch = globalThis.fetch } = {}) {
  return async function run({ inputs, credentials }) {
    const repository = inputs[0]?.repository;
    if (
      typeof repository !== "string" ||
      !/^[A-Za-z0-9_-]+\/[A-Za-z0-9_.-]+$/.test(repository)
    )
      throw new Error("Specify repository as GitHub owner/repo");
    const issues = [];
    for (let page = 1; ; page++) {
      if (page > 100) throw new Error("GitHub issue pagination limit reached");
      const response = await fetch(
        `https://api.github.com/repos/${repository}/issues?state=open&labels=relayfold&per_page=100&page=${page}`,
        {
          headers: {
            Authorization: "Bearer " + credentials.gh_token,
            Accept: "application/vnd.github+json",
            "X-GitHub-Api-Version": "2022-11-28",
          },
          signal: AbortSignal.timeout(30000),
        },
      );
      if (!response.ok)
        throw new Error("GitHub request failed: " + response.status);
      const batch = await response.json();
      if (!Array.isArray(batch))
        throw new Error("Invalid GitHub issues response");
      issues.push(
        ...batch
          .filter((issue) => !issue.pull_request)
          .map(({ number, title, body, html_url }) => ({
            number,
            title,
            body: body ?? "",
            html_url,
          })),
      );
      if (batch.length < 100) break;
    }
    return { repository, issues };
  };
}

export default createIssueFetcher();
