export function createLabeler({ fetch = globalThis.fetch } = {}) {
  return async function run({ inputs, credentials }) {
    const issue = inputs.find((input) => typeof input?.issue_url === "string");
    const pr = inputs.find((input) => typeof input?.pr_created === "boolean");
    if (!issue || !pr)
      throw new Error("Missing fetched issue or PR creation result");
    const { repository, issue_number } = issue;
    if (
      typeof repository !== "string" ||
      !/^[A-Za-z0-9_-]+\/[A-Za-z0-9_.-]+$/.test(repository) ||
      !Number.isSafeInteger(issue_number) ||
      issue_number < 1 ||
      pr.repository !== repository ||
      pr.issue_number !== issue_number
    )
      throw new Error("Issue and PR result must identify the same valid issue");
    if (!pr.pr_created) return { repository, issue_number, labeled: false };
    if (!Number.isSafeInteger(pr.pr_number) || pr.pr_number < 1)
      throw new Error("Created PR result must include a valid pr_number");

    const label = "relayfold:pr-created";
    const base = `https://api.github.com/repos/${repository}`;
    async function request(path, options = {}) {
      return fetch(base + path, {
        ...options,
        headers: {
          Authorization: "Bearer " + credentials.gh_token,
          Accept: "application/vnd.github+json",
          "Content-Type": "application/json",
          "X-GitHub-Api-Version": "2022-11-28",
        },
        signal: AbortSignal.timeout(30000),
      });
    }
    function requireSuccess(response) {
      if (!response.ok)
        throw new Error("GitHub request failed: " + response.status);
    }
    const labelPath = "/labels/" + encodeURIComponent(label);
    const existing = await request(labelPath);
    if (existing.status === 404) {
      const created = await request("/labels", {
        method: "POST",
        body: JSON.stringify({
          name: label,
          color: "5319e7",
          description: "A PR has been created; RelayFold must not pick up this issue.",
        }),
      });
      // Another run may have created the repository label concurrently.
      if (created.status === 422) requireSuccess(await request(labelPath));
      else requireSuccess(created);
    } else requireSuccess(existing);

    const response = await request(`/issues/${issue_number}/labels`, {
      method: "POST",
      body: JSON.stringify({ labels: [label] }),
    });
    requireSuccess(response);
    const labels = await response.json();
    if (!Array.isArray(labels) || !labels.some((item) => item?.name === label))
      throw new Error("GitHub did not confirm the PR-created label on the issue");
    return { repository, issue_number, labeled: true };
  };
}

export default createLabeler();
