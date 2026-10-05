import {
  CloudWatchLogsClient,
  FilterLogEventsCommand,
} from "@aws-sdk/client-cloudwatch-logs";
import { createHash } from "node:crypto";

export function createScanner({
  createClient = (options) => new CloudWatchLogsClient(options),
  now = Date.now,
} = {}) {
  return async function run({ inputs, credentials }) {
    const config = inputs[0];
    if (
      !config?.targets?.length ||
      config.targets.some(
        (t) => !t.region || !t.log_group || t.log_group.includes("<"),
      )
    ) {
      throw new Error(
        "Specify explicit AWS region and CloudWatch log_group targets",
      );
    }
    if (
      typeof config.repository !== "string" ||
      !/^[A-Za-z0-9_-]+\/[A-Za-z0-9_.-]+$/.test(config.repository)
    )
      throw new Error("Specify repository as GitHub owner/repo");
    const hours = config.lookback_hours ?? 24;
    if (!Number.isInteger(hours) || hours < 1 || hours > 168)
      throw new Error("lookback_hours must be 1..168");
    const end = now() - 5 * 60_000; // Allow ingestion to settle.
    const start = end - hours * 3_600_000 - 10 * 60_000; // Overlap for late events.
    const groups = new Map();
    let total = 0;
    const truncation = [];
    scanTargets: for (const target of config.targets) {
      const client = createClient({
        region: target.region,
        credentials: {
          accessKeyId: credentials.aws_access_key_id,
          secretAccessKey: credentials.aws_secret_access_key,
          ...(credentials.aws_session_token
            ? { sessionToken: credentials.aws_session_token }
            : {}),
        },
      });
      try {
        let nextToken;
        let pages = 0;
        const tokens = new Set();
        do {
          if (pages >= 200) {
            truncation.push({ ...target, reason: "page_limit" });
            break;
          }
          pages++;
          const page = await client.send(
            new FilterLogEventsCommand({
              logGroupName: target.log_group,
              startTime: start,
              endTime: end,
              filterPattern: "%[Ww][Aa][Rr][Nn]|[Ee][Rr][Rr][Oo][Rr]%",
              nextToken,
            }),
          );
          for (const event of page.events ?? []) {
            if (total >= 50000) {
              truncation.push({ ...target, reason: "event_limit" });
              break scanTargets;
            }
            const message = event.message ?? "";
            // Normalize only the grouping signature; samples retain original values.
            const signature = message
              .replace(/\b[0-9a-f]{8}-[0-9a-f-]{27,}\b/gi, "<id>")
              .replace(/\b[0-9a-f]{24,}\b/gi, "<id>")
              .replace(/\b\d+(?:\.\d+)?\b/g, "<n>")
              .replace(/\s+/g, " ")
              .trim();
            const fingerprint = createHash("sha256")
              .update(target.log_group + "\n" + signature)
              .digest("hex");
            let group = groups.get(fingerprint);
            if (!group) {
              if (groups.size >= 200) {
                truncation.push({ ...target, reason: "pattern_limit" });
                break scanTargets;
              }
              group = {
                fingerprint,
                log_group: target.log_group,
                regions: [],
                count: 0,
                first_seen: event.timestamp,
                last_seen: event.timestamp,
                samples: [],
              };
              groups.set(fingerprint, group);
            }
            total++;
            group.count++;
            group.first_seen = Math.min(group.first_seen, event.timestamp);
            group.last_seen = Math.max(group.last_seen, event.timestamp);
            if (!group.regions.includes(target.region))
              group.regions.push(target.region);
            if (group.samples.length < 3)
              group.samples.push({
                timestamp: event.timestamp,
                message: message.slice(0, 6000),
              });
          }
          nextToken = page.nextToken;
          if (nextToken && tokens.has(nextToken))
            throw new Error("Repeated CloudWatch pagination token");
          if (nextToken) tokens.add(nextToken);
        } while (nextToken); // Empty pages can still have a next token.
      } finally {
        client.destroy();
      }
    }
    return {
      repository: config.repository,
      branch: "main",
      start_time: new Date(start).toISOString(),
      end_time: new Date(end).toISOString(),
      dry_run: config.dry_run ?? false,
      total_events: total,
      workflow_exit_reason:
        groups.size === 0 ? "No matching log patterns were found." : null,
      truncated: truncation.length > 0,
      truncation,
      groups: [...groups.values()].sort((a, b) => b.count - a.count),
    };
  };
}

export default createScanner();
