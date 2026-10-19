export function createLabeler({ fetch = globalThis.fetch } = {}) {
  return async function run({ inputs, credentials }) {
    const issue = inputs.find((input) => typeof input?.issue_url === "string");
    const pr = inputs.find((input) => typeof input?.pr_created === "boolean");
    const summary = inputs.find((input) => input?.commented === true);
    if (!issue || !pr || !summary)
      throw new Error(
        "Missing fetched issue, PR result, or successful issue summary",
      );
    const { repository, issue_number } = issue;
    if (
      typeof repository !== "string" ||
      !/^[A-Za-z0-9_-]+\/[A-Za-z0-9_.-]+$/.test(repository) ||
      !Number.isSafeInteger(issue_number) ||
      issue_number < 1 ||
      [pr, summary].some(
        (input) =>
          input.repository !== repository ||
          input.issue_number !== issue_number,
      )
    )
      throw new Error("Label inputs must identify the same valid issue");
    const hasPr = pr.pr_number !== undefined;
    if (
      (hasPr &&
        (!Number.isSafeInteger(pr.pr_number) ||
          pr.pr_number < 1 ||
          summary.pr_number !== pr.pr_number)) ||
      (!hasPr &&
        (pr.pr_created || issue.pr || summary.pr_number !== undefined)) ||
      (issue.pr && (pr.pr_created || pr.pr_number !== issue.pr.pr_number))
    )
      throw new Error("PR result and summary must identify the same valid PR");
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
    const applied = [];
    if (hasPr) {
      const label = "relayfold:pr-created";
      const labelPath = "/labels/" + encodeURIComponent(label);
      const existing = await request(labelPath);
      if (existing.status === 404) {
        const created = await request("/labels", {
          method: "POST",
          body: JSON.stringify({
            name: label,
            color: "5319e7",
            description: "RelayFold has a PR for this issue.",
          }),
        });
        if (created.status === 422) requireSuccess(await request(labelPath));
        else requireSuccess(created);
      } else requireSuccess(existing);
      const response = await request(`/issues/${issue_number}/labels`, {
        method: "POST",
        body: JSON.stringify({ labels: [label] }),
      });
      requireSuccess(response);
      const labels = await response.json();
      if (
        !Array.isArray(labels) ||
        !labels.some((item) => item?.name === label)
      )
        throw new Error("GitHub did not confirm the PR marker on the issue");
      applied.push({ repository, issue_number, labels: [label] });
    }
    const ready = "relayfold:agent-ready";
    const removal = await request(
      `/issues/${issue_number}/labels/${encodeURIComponent(ready)}`,
      { method: "DELETE" },
    );
    // A retry may find the label already removed; confirm that the issue still exists.
    const confirmation =
      removal.status === 404
        ? await request(`/issues/${issue_number}/labels`)
        : removal;
    requireSuccess(confirmation);
    const remaining = await confirmation.json();
    if (
      !Array.isArray(remaining) ||
      remaining.some((item) => (item?.name ?? item) === ready)
    )
      throw new Error("GitHub did not confirm removal of agent-ready");
    return {
      applied,
      removed: [{ repository, issue_number, labels: [ready] }],
    };
  };
}

export default createLabeler();
