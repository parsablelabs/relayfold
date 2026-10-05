export function createLabeler({ fetch = globalThis.fetch } = {}) {
  return async function run({ inputs, credentials }) {
    const publication = inputs.find((input) => Array.isArray(input?.created));
    let jobs;
    if (publication) {
      jobs = publication.created.map((issue) => ({
        ...issue,
        pr_created: false,
      }));
    } else {
      const issue = inputs.find((input) => typeof input?.issue_url === "string");
      const pr = inputs.find((input) => typeof input?.pr_created === "boolean");
      const implementation = inputs.find((input) =>
        Array.isArray(input?.open_questions),
      );
      if (!issue || !pr)
        throw new Error("Missing fetched issue or PR creation result");
      if (
        pr.repository !== issue.repository ||
        pr.issue_number !== issue.issue_number
      )
        throw new Error("Issue and PR result must identify the same valid issue");
      if (
        implementation &&
        (implementation.repository !== issue.repository ||
          implementation.issue_number !== issue.issue_number)
      )
        throw new Error("Implementation must identify the same issue");
      jobs = [
        {
          ...issue,
          pr_created: pr.pr_created,
          pr_number: pr.pr_number,
          human_input_needed: Boolean(implementation?.open_questions.length),
        },
      ];
    }
    for (const job of jobs) {
      if (
        typeof job.repository !== "string" ||
        !/^[A-Za-z0-9_-]+\/[A-Za-z0-9_.-]+$/.test(job.repository) ||
        !Number.isSafeInteger(job.issue_number) ||
        job.issue_number < 1 ||
        typeof job.human_input_needed !== "boolean"
      )
        throw new Error(
          "Label request must identify a valid issue and human-input decision",
        );
      if (
        job.pr_created &&
        (!Number.isSafeInteger(job.pr_number) || job.pr_number < 1)
      )
        throw new Error("Created PR result must include a valid pr_number");
    }
    const applied = [];
    for (const job of jobs) {
      const { repository, issue_number } = job;
      const requested = [];
      if (job.pr_created) requested.push("relayfold:pr-created");
      if (job.human_input_needed) requested.push("relayfold:human-input-needed");
      if (!requested.length) continue;
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
      for (const label of requested) {
        const labelPath = "/labels/" + encodeURIComponent(label);
        const existing = await request(labelPath);
        if (existing.status === 404) {
          const created = await request("/labels", {
            method: "POST",
            body: JSON.stringify({
              name: label,
              color: "5319e7",
              description:
                label === "relayfold:pr-created"
                  ? "A PR has been created; RelayFold must not pick up this issue."
                  : "Human decisions or information are needed before implementation.",
            }),
          });
          // Another run may have created the repository label concurrently.
          if (created.status === 422) requireSuccess(await request(labelPath));
          else requireSuccess(created);
        } else requireSuccess(existing);
      }
      const response = await request(`/issues/${issue_number}/labels`, {
        method: "POST",
        body: JSON.stringify({ labels: requested }),
      });
      requireSuccess(response);
      const labels = await response.json();
      if (
        !Array.isArray(labels) ||
        !requested.every((label) => labels.some((item) => item?.name === label))
      )
        throw new Error("GitHub did not confirm the requested labels on the issue");
      applied.push({ repository, issue_number, labels: requested });
    }
    return { applied };
  };
}

export default createLabeler();
