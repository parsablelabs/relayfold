export function createPublisher({ fetch = globalThis.fetch } = {}) {
  return async function run({ inputs, credentials }) {
    const scan = inputs.find((i) => Array.isArray(i?.groups));
    const verification = inputs.find((i) => i?.decision === "complete");
    const analysis = verification?.output;
    if (
      !scan ||
      !analysis ||
      typeof analysis !== "object" ||
      Array.isArray(analysis)
    )
      throw new Error("Missing accepted verifier output or scan");
    if (scan.groups.length > 1)
      throw new Error("Publishing requires at most one selected group");
    if (Object.keys(analysis).length === 0)
      return {
        created: [],
        skipped: [],
        dry_run: scan.dry_run,
        summary: "No work to do; no issues needed.",
      };
    if (
      !Array.isArray(analysis.findings) ||
      !/^[0-9a-f]{40}$/.test(analysis.commit_sha)
    )
      throw new Error("Missing scan or main commit evidence");
    if (
      typeof scan.repository !== "string" ||
      !/^[A-Za-z0-9_-]+\/[A-Za-z0-9_.-]+$/.test(scan.repository)
    )
      throw new Error("Specify repository as GitHub owner/repo");
    const allowed = new Set(scan.groups.map((g) => g.fingerprint));
    if (analysis.findings.length > 1)
      throw new Error("At most one issue per run");
    const used = new Set();
    for (const finding of analysis.findings) {
      if (!allowed.has(finding.fingerprint) || used.has(finding.fingerprint))
        throw new Error("Unknown or repeated finding fingerprint");
      used.add(finding.fingerprint);
      if (
        finding.labels !== undefined &&
        (!Array.isArray(finding.labels) ||
          finding.labels.length > 2 ||
          new Set(finding.labels).size !== finding.labels.length ||
          finding.labels.some(
            (label) => !["bug", "relayfold:human-input-needed"].includes(label),
          ))
      )
        throw new Error(
          "Optional finding labels may contain only bug and relayfold:human-input-needed without duplicates",
        );
      if (
        !finding.title ||
        finding.title.length > 200 ||
        !finding.body ||
        !["Problem", "Goal", "Acceptance Criteria", "Notes"].every((h) =>
          finding.body.includes("## " + h),
        )
      ) {
        throw new Error(
          "Issue needs a title and Problem, Goal, Acceptance Criteria, Notes sections",
        );
      }
    }
    const base = "https://api.github.com/repos/" + scan.repository;
    async function api(path, options = {}) {
      const response = await fetch(base + path, {
        ...options,
        headers: {
          Authorization: "Bearer " + credentials.gh_token,
          Accept: "application/vnd.github+json",
          "Content-Type": "application/json",
          "X-GitHub-Api-Version": "2022-11-28",
        },
        signal: AbortSignal.timeout(30000),
      });
      if (!response.ok)
        throw new Error("GitHub request failed: " + response.status);
      return response.json();
    }
    if (!analysis.findings.length)
      return {
        created: [],
        skipped: [],
        dry_run: scan.dry_run,
        summary: analysis.summary,
      };
    // Inspect every issue page, including closed issues. Fail rather than publish with an incomplete duplicate check.
    const issues = [];
    for (let page = 1; ; page++) {
      if (page > 100) throw new Error("GitHub issue pagination limit reached");
      const batch = await api("/issues?state=all&per_page=100&page=" + page);
      issues.push(...batch.filter((i) => !i.pull_request));
      if (batch.length < 100) break;
    }
    const created = [],
      skipped = [],
      drafts = [];
    for (const finding of analysis.findings) {
      const marker =
        "<!-- cloudwatch-log-scanner-cloudwatch:" +
        finding.fingerprint +
        " -->";
      const duplicate = issues.find((i) => (i.body ?? "").includes(marker));
      if (duplicate) {
        skipped.push({
          fingerprint: finding.fingerprint,
          issue_url: duplicate.html_url,
        });
        continue;
      }
      const group = scan.groups.find(
        (g) => g.fingerprint === finding.fingerprint,
      );
      let body =
        finding.body +
        "\n\n" +
        marker +
        "\n\nScanned " +
        scan.start_time +
        " to " +
        scan.end_time +
        "; " +
        group.count +
        " matching events. Main commit: `" +
        analysis.commit_sha +
        "`.";
      if (scan.truncated) {
        body +=
          "\nScan was truncated; counts cover retained events only. Limits: " +
          scan.truncation
            .map((t) => `${t.reason} (${t.region}, ${t.log_group})`)
            .join(", ") +
          ".";
      }
      const labels = ["relayfold", ...(finding.labels ?? [])];
      if (scan.dry_run) {
        drafts.push({ title: finding.title, body, labels });
        continue;
      }
      // Do not automatically retry a POST: an uncertain response may already have created the issue.
      const issue = await api("/issues", {
        method: "POST",
        body: JSON.stringify({
          title: finding.title,
          body,
          labels: labels.filter(
            (label) => label !== "relayfold:human-input-needed",
          ),
        }),
      });
      issues.push(issue);
      created.push({
        repository: scan.repository,
        issue_number: issue.number,
        human_input_needed: labels.includes("relayfold:human-input-needed"),
        fingerprint: finding.fingerprint,
        issue_url: issue.html_url,
      });
    }
    return {
      created,
      skipped,
      drafts,
      dry_run: scan.dry_run,
      summary: analysis.summary,
    };
  };
}

export default createPublisher();
