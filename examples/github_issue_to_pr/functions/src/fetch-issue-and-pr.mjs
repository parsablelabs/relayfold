export function createIssueAndPrFetcher({ fetch = globalThis.fetch } = {}) {
  return async function run({ inputs, credentials }) {
    const { repository, issue_number, pr_number: explicitPr } = inputs[0] ?? {};
    if (
      typeof repository !== "string" ||
      !/^[A-Za-z0-9_-]+\/[A-Za-z0-9_.-]+$/.test(repository)
    )
      throw new Error("Specify repository as GitHub owner/repo");
    if (issue_number !== undefined && (!Number.isSafeInteger(issue_number) || issue_number < 1))
      throw new Error("Specify a positive integer issue_number");

    if (explicitPr !== undefined && (!Number.isSafeInteger(explicitPr) || explicitPr < 1))
      throw new Error("Specify a positive integer pr_number");

    if (explicitPr !== undefined && issue_number === undefined)
      throw new Error("Specify issue_number when supplying pr_number");

    const base = `/repos/${repository}`;
    async function api(path) {
      const response = await fetch(`https://api.github.com${path}`, {
        headers: {
          Authorization: "Bearer " + credentials.gh_token,
          Accept: "application/vnd.github+json",
          "X-GitHub-Api-Version": "2022-11-28",
        },
        signal: AbortSignal.timeout(30000),
      });
      if (!response.ok) throw new Error("GitHub request failed: " + response.status);
      return response.json();
    }

    async function list(path, kind) {
      const entries = [];
      const separator = path.includes("?") ? "&" : "?";
      for (let page = 1; page <= 100; page++) {
        const batch = await api(`${base}${path}${separator}per_page=100&page=${page}`);
        if (!Array.isArray(batch)) throw new Error(`Invalid GitHub ${kind} response`);
        entries.push(...batch);
        if (batch.length < 100) return entries;
      }
      throw new Error(`GitHub ${kind} pagination limit reached`);
    }

    let accountId;
    async function authenticatedAccount() {
      if (accountId === undefined) {
        const account = await api("/user");
        if (!Number.isSafeInteger(account?.id) || account.id < 1)
          throw new Error("Invalid authenticated GitHub account");
        accountId = account.id;
      }
      return accountId;
    }

    const statusPrefix = "<!-- relayfold:issue-to-pr ";
    const legacyMarker = "<!-- relayfold:issue-to-pr -->";
    const ready = "relayfold:agent-ready";
    const created = "relayfold:pr-created";
    const hasLabel = (issue, name) => issue.labels.some((label) => (label?.name ?? label) === name);
    function validateIssue(issue) {
      if (!issue || !Number.isSafeInteger(issue.number) || issue.number < 1 ||
          typeof issue.title !== "string" || typeof issue.html_url !== "string" ||
          !["open", "closed"].includes(issue.state) || !Array.isArray(issue.labels))
        throw new Error("Invalid GitHub issue response");
    }

    const automatic = issue_number === undefined;
    const candidates = [];
    if (automatic) {
      const listings = await Promise.all([ready, created].map((label) =>
        list(`/issues?state=open&labels=${encodeURIComponent(label)}&sort=created&direction=asc`, "issue")));
      const issues = new Map();
      for (const [index, label] of [ready, created].entries()) {
        for (const candidate of listings[index]) {
          validateIssue(candidate);
          if (candidate.state === "open" && !candidate.pull_request && hasLabel(candidate, label))
            issues.set(candidate.number, candidate);
        }
      }
      candidates.push(...[...issues.values()].sort((first, second) =>
        Date.parse(first.created_at) - Date.parse(second.created_at) || first.number - second.number));
    } else {
      candidates.push({ number: issue_number });
    }

    for (const candidate of candidates) {
      const issue = await api(`${base}/issues/${candidate.number}`);
      validateIssue(issue);
      if (issue.number !== candidate.number) throw new Error("Invalid GitHub issue response");
      if (automatic && issue.state !== "open") continue;
      if (issue.pull_request) {
        if (automatic) continue;
        throw new Error("Requested number identifies a pull request, not an issue");
      }
      const isReady = hasLabel(issue, ready);
      const hasPublishedPr = hasLabel(issue, created);
      if (!isReady && !hasPublishedPr) continue;

      const comments = await list(`/issues/${issue.number}/comments`, "comments");
      const possibleStatus = comments.filter(({ body }) => typeof body === "string" &&
        (body.startsWith(statusPrefix) || body.split("\n").includes(legacyMarker)));
      const statusAuthor = possibleStatus.length > 0 ? await authenticatedAccount() : undefined;
      const trustedComments = possibleStatus.filter(({ user }) => user?.id === statusAuthor);
      const statusComments = trustedComments.filter(({ body }) =>
        body.startsWith(statusPrefix) && !body.startsWith(legacyMarker));
      if (statusComments.length > 1) throw new Error("Multiple RelayFold status comments on issue");
      const statusComment = statusComments[0];
      let status;
      if (statusComment) {
        const end = statusComment.body.indexOf(" -->", statusPrefix.length);
        if (end < 0) throw new Error("Invalid RelayFold status comment");
        status = JSON.parse(statusComment.body.slice(statusPrefix.length, end));
        if (!Number.isSafeInteger(statusComment.id) || statusComment.id < 1 ||
            !(status?.pr === null || Number.isSafeInteger(status?.pr) && status.pr > 0) ||
            !Array.isArray(status.handled) ||
            !status.handled.every((id) => typeof id === "string" && /^(discussion|inline|review):[1-9][0-9]*$/.test(id)))
          throw new Error("Invalid RelayFold status comment");
        if (explicitPr !== undefined && status.pr !== explicitPr)
          throw new Error("Explicit PR does not match the recorded RelayFold status");
      }

      const legacySummary = trustedComments.findLast(({ body }) => body.split("\n").includes(legacyMarker));
      const prUrlPrefix = `https://github.com/${repository}/pull/`;
      const legacyPrLink = legacySummary?.body.split("\n")
        .map((line) => /^PR #([1-9][0-9]*): (\S+)$/.exec(line))
        .find((link) => link && Number.isSafeInteger(Number(link[1])) &&
          link[2].toLowerCase() === `${prUrlPrefix}${link[1]}`.toLowerCase());
      const legacyPrNumber = legacyPrLink ? Number(legacyPrLink[1]) : undefined;
      const result = {
        repository,
        issue_number: issue.number,
        issue_url: issue.html_url,
        title: issue.title,
        state: issue.state.toUpperCase(),
        body: issue.body ?? "",
        comments: comments.filter((comment) => !trustedComments.includes(comment)).map((comment) => comment.body ?? ""),
      };
      let prNumber = status ? status.pr : explicitPr ?? legacyPrNumber;
      if (prNumber === null ||
          (prNumber === undefined && legacySummary?.body.startsWith("### RelayFold — No PR needed\n"))) continue;

      if (prNumber === undefined) {
        // Recover if publication succeeded but recording the status comment failed.
        const owner = repository.split("/")[0];
        const head = encodeURIComponent(`${owner}:relayfold/issue-${issue.number}`);
        const matches = await list(`/pulls?state=all&head=${head}`, "pull requests");
        if (matches.length > 1) throw new Error("Multiple PRs found for the RelayFold issue branch");
        if (matches.length) {
          prNumber = matches[0]?.number;
          if (!Number.isSafeInteger(prNumber) || prNumber < 1)
            throw new Error("Invalid GitHub pull request response");
        }
      }

      if (prNumber === undefined) {
        if (isReady && !hasPublishedPr) return result;
        if (automatic) continue;
        throw new Error(`Issue has the "${created}" label but no recorded PR; supply pr_number`);
      }

      const pull = await api(`${base}/pulls/${prNumber}`);
      if (!pull || pull.number !== prNumber ||
          typeof pull.title !== "string" || typeof pull.html_url !== "string" ||
          !["open", "closed"].includes(pull.state) || typeof pull.head?.ref !== "string")
        throw new Error("Invalid GitHub pull request response");
      if (pull.state !== "open") continue;
      if (pull.head.repo?.full_name?.toLowerCase() !== repository.toLowerCase())
        throw new Error("Tracked PR must have a head branch in the configured repository");

      const discussion = await list(`/issues/${prNumber}/comments`, "comments");
      const inline = await list(`/pulls/${prNumber}/comments`, "comments");
      const reviews = await list(`/pulls/${prNumber}/reviews`, "comments");
      const actor = await authenticatedAccount();
      const handled = new Set(status?.handled);
      const feedback = [
        ...discussion.map((comment) => ({ comment, source: "discussion" })),
        ...inline.map((comment) => ({ comment, source: "inline" })),
        ...reviews.map((comment) => ({ comment, source: "review" })),
      ].filter(({ comment }) =>
        comment.state !== "PENDING" && comment.user?.type !== "Bot" &&
        comment.user?.id !== actor && !comment.performed_via_github_app &&
        typeof comment.body === "string" && comment.body.trim() &&
        !comment.body.includes("<!-- relayfold:issue-to-pr") &&
          ["OWNER", "MEMBER", "COLLABORATOR"].includes(comment.author_association),
      ).sort((a, b) =>
        Date.parse(a.comment.submitted_at ?? a.comment.created_at ?? "1970-01-01") -
        Date.parse(b.comment.submitted_at ?? b.comment.created_at ?? "1970-01-01"),
      ).map(({ comment, source }) => {
        if (!Number.isSafeInteger(comment.id) || comment.id < 1)
          throw new Error("Invalid GitHub feedback ID");
        return { id: `${source}:${comment.id}`, body: comment.body, url: comment.html_url ?? "" };
      }).filter(({ id }) => !handled.has(id));

      if (!feedback.length) continue;
      return {
        ...result,
        pr: {
          pr_number: prNumber, pr_url: pull.html_url, title: pull.title, body: pull.body ?? "",
          state: pull.state.toUpperCase(), branch: pull.head.ref,
          comments: discussion.map((comment) => comment.body ?? ""),
          review_comments: inline.map((comment) => comment.body ?? ""),
          reviews: reviews.map((comment) => comment.body ?? ""),
        },
        feedback,
        handled_feedback: [...handled],
        ...(statusComment ? { status_comment_id: statusComment.id } : {}),
      };
    }

    return {
      repository,
      _workflow_exit_reason: "No eligible issue with relayfold:agent-ready or new PR feedback",
    };
  };
}

export default createIssueAndPrFetcher();
