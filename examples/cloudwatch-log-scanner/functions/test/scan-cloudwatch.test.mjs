import assert from "node:assert/strict";
import test from "node:test";
import { createServer } from "node:http";
import { once } from "node:events";
import { readFile, writeFile, rm } from "node:fs/promises";
import { CloudWatchLogsClient } from "@aws-sdk/client-cloudwatch-logs";
import { createScanner } from "../src/scan-cloudwatch.mjs";

const now = Date.UTC(2026, 9, 3, 12);
const context = {
  inputs: [
    {
      repository: "example/service",
      targets: [{ region: "us-east-1", log_group: "/test" }],
      lookback_hours: 1,
      dry_run: true,
    },
  ],
  credentials: {
    aws_access_key_id: "test-key",
    aws_secret_access_key: "test-secret",
    aws_session_token: "test-session",
  },
  workspacePath: "/tmp/unused",
};
function fixture(pages) {
  const requests = [],
    options = [];
  let destroyed = 0;
  const scan = createScanner({
    now: () => now,
    createClient: (config) => {
      options.push(config);
      return {
        send: async (command) => {
          requests.push(command.input);
          const page = pages.shift();
          if (page instanceof Error) throw page;
          assert(page, "Unexpected CloudWatch request");
          return page;
        },
        destroy: () => {
          destroyed++;
        },
      };
    },
  });
  return { scan, requests, options, destroyed: () => destroyed };
}

test("returns grouped, original samples across empty pages and both regions", async () => {
  const a = now - 600_000,
    b = now - 400_000;
  const f = fixture([
    { events: [], nextToken: "page2" },
    {
      events: [
        {
          timestamp: a,
          message:
            "WARN call 123 failed user@example.com Bearer secret +15551234567",
        },
      ],
    },
    {
      events: [
        {
          timestamp: b,
          message:
            "WARN call 456 failed user@example.com Bearer secret +15559876543",
        },
      ],
    },
  ]);
  const result = await f.scan({
    ...context,
    inputs: [
      {
        ...context.inputs[0],
        targets: [
          ...context.inputs[0].targets,
          { region: "us-west-2", log_group: "/test" },
        ],
      },
    ],
  });
  assert.equal(result.total_events, 2);
  assert.equal(result.no_work, false);
  assert.equal(result.groups.length, 1);
  const group = result.groups[0];
  assert.equal(group.count, 2);
  assert.deepEqual(group.regions, ["us-east-1", "us-west-2"]);
  assert.equal(group.first_seen, a);
  assert.equal(group.last_seen, b);
  assert.equal(group.samples.length, 2);
  assert.equal(
    group.samples[0].message,
    "WARN call 123 failed user@example.com Bearer secret +15551234567",
  );
  assert.match(group.fingerprint, /^[a-f0-9]{64}$/);
  assert.equal(f.requests[1].nextToken, "page2");
  assert.equal(f.requests[0].startTime, now - 75 * 60_000);
  assert.equal(f.requests[0].endTime, now - 5 * 60_000);
  assert.equal(
    result.start_time,
    new Date(f.requests[0].startTime).toISOString(),
  );
  assert.equal(result.dry_run, true);
  assert.equal(f.options[0].credentials.sessionToken, "test-session");
  assert.equal(f.destroyed(), 2);
  assert.equal(result.truncated, false);
  assert.deepEqual(result.truncation, []);
});

test("returns no findings data when CloudWatch has no matching events", async () => {
  const f = fixture([{ events: [] }]);
  const result = await f.scan(context);
  assert.deepEqual(result.groups, []);
  assert.equal(result.total_events, 0);
});

test("limits sample size without losing occurrence counts and sorts by frequency", async () => {
  const events = Array.from({ length: 5 }, (_, i) => ({
    timestamp: now + i,
    message: "ERROR queue unavailable",
  }));
  events.push({ timestamp: now, message: "WARN distinct issue" });
  const result = await fixture([{ events }]).scan(context);
  assert.deepEqual(
    result.groups.map((g) => g.count),
    [5, 1],
  );
  assert.equal(result.groups[0].samples.length, 3);
});

test("rejects invalid input, AWS failures and repeated tokens", async () => {
  await assert.rejects(
    fixture([]).scan({ ...context, inputs: [{ targets: [] }] }),
    /Specify explicit/,
  );
  await assert.rejects(
    fixture([]).scan({
      ...context,
      inputs: [{ ...context.inputs[0], lookback_hours: 0 }],
    }),
    /1..168/,
  );
  const denied = fixture([new Error("AccessDenied")]);
  await assert.rejects(denied.scan(context), /AccessDenied/);
  assert.equal(denied.destroyed(), 1);
  await assert.rejects(
    fixture([{ nextToken: "x" }, { nextToken: "x" }]).scan(context),
    /Repeated/,
  );
});

test("preserves message values and formatting without masking", async () => {
  const message =
    '\x1b[31mERROR email=user@example.com token="secret" phone=+15551234567 https://example.com/path?token=secret\x1b[0m';
  const result = await fixture([
    { events: [{ timestamp: now, message }] },
  ]).scan(context);
  assert.equal(result.groups[0].samples[0].message, message);
});

test("returns retained groups when the pattern limit is exceeded", async () => {
  const events = Array.from({ length: 201 }, (_, i) => ({
    timestamp: now,
    message:
      "WARN pattern " +
      String.fromCharCode(65 + Math.floor(i / 26)) +
      String.fromCharCode(65 + (i % 26)),
  }));
  const f = fixture([{ events }]);
  const result = await f.scan(context);
  assert.equal(result.groups.length, 200);
  assert.equal(result.total_events, 200);
  assert.equal(result.truncated, true);
  assert.deepEqual(result.truncation, [
    { region: "us-east-1", log_group: "/test", reason: "pattern_limit" },
  ]);
  assert.equal(f.destroyed(), 1);
});

test("returns exactly 50,000 retained events and stops scanning remaining targets", async () => {
  const events = Array.from({ length: 50001 }, () => ({
    timestamp: now,
    message: "ERROR queue unavailable",
  }));
  const f = fixture([{ events }]);
  const result = await f.scan({
    ...context,
    inputs: [
      {
        ...context.inputs[0],
        targets: [
          ...context.inputs[0].targets,
          { region: "us-west-2", log_group: "/other" },
        ],
      },
    ],
  });
  assert.equal(result.total_events, 50000);
  assert.equal(result.groups[0].count, 50000);
  assert.equal(result.truncated, true);
  assert.equal(result.truncation[0].reason, "event_limit");
  assert.equal(f.requests.length, 1);
  assert.equal(f.destroyed(), 1);
});

test("page limit retains events and continues with the next target", async () => {
  const pages = Array.from({ length: 200 }, (_, i) => ({
    events: [{ timestamp: now, message: "WARN first target" }],
    nextToken: "page-" + i,
  }));
  pages.push({ events: [{ timestamp: now, message: "ERROR second target" }] });
  const f = fixture(pages);
  const result = await f.scan({
    ...context,
    inputs: [
      {
        ...context.inputs[0],
        targets: [
          ...context.inputs[0].targets,
          { region: "us-west-2", log_group: "/other" },
        ],
      },
    ],
  });
  assert.equal(result.total_events, 201);
  assert.equal(result.groups.length, 2);
  assert.equal(result.truncated, true);
  assert.deepEqual(result.truncation, [
    { region: "us-east-1", log_group: "/test", reason: "page_limit" },
  ]);
  assert.equal(f.requests.length, 201);
  assert.equal(f.destroyed(), 2);
});

test("generated scanner uses the real AWS SDK to return results from a local CloudWatch protocol server", async (t) => {
  // No mocked SDK client: this exercises SDK serialization, signing, pagination,
  // deserialization, and the registry artifact's actual exported implementation.
  const artifact = JSON.parse(
    await readFile(
      new URL(
        "../dist/cloudwatch-log-scanner.scan_cloudwatch.json",
        import.meta.url,
      ),
    ),
  );
  assert.deepEqual(artifact.dependencies, [
    { name: "@aws-sdk/client-cloudwatch-logs", version: "3.1146.0" },
  ]);
  const runtime = new URL("../dist/scan-runtime.mjs", import.meta.url);
  await writeFile(runtime, artifact.code);
  t.after(() => rm(runtime, { force: true }));
  const { createScanner: builtScanner } = await import(runtime.href);
  const requests = [];
  const server = createServer(async (req, res) => {
    let body = "";
    for await (const chunk of req) body += chunk;
    requests.push({ headers: req.headers, body: JSON.parse(body) });
    res.writeHead(200, { "content-type": "application/x-amz-json-1.1" });
    res.end(
      JSON.stringify(
        requests.length === 1
          ? { events: [], nextToken: "next-page" }
          : {
              events: [
                {
                  timestamp: now - 600_000,
                  message: "ERROR failed to publish queued lead",
                  eventId: "e1",
                  logStreamName: "service",
                },
              ],
            },
      ),
    );
  });
  t.after(() => new Promise((resolve) => server.close(resolve)));
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  const scan = builtScanner({
    now: () => now,
    createClient: (options) =>
      new CloudWatchLogsClient({
        ...options,
        endpoint: `http://127.0.0.1:${server.address().port}`,
        maxAttempts: 1,
      }),
  });
  const result = await scan(context);
  assert.equal(result.total_events, 1);
  assert.equal(
    result.groups[0].samples[0].message,
    "ERROR failed to publish queued lead",
  );
  assert.equal(requests.length, 2);
  assert.equal(requests[1].body.nextToken, "next-page");
  assert.equal(requests[0].body.logGroupName, "/test");
  assert.equal(
    requests[0].body.filterPattern,
    "%[Ww][Aa][Rr][Nn]|[Ee][Rr][Rr][Oo][Rr]%",
  );
  assert.match(requests[0].headers.authorization, /^AWS4-HMAC-SHA256 /);
});

test("requires a valid repository and passes it through the scan", async () => {
  for (const repository of [
    undefined,
    "https://github.com/example/service",
    "../service",
    "example/service?other",
  ]) {
    await assert.rejects(
      fixture([]).scan({
        ...context,
        inputs: [{ ...context.inputs[0], repository }],
      }),
      /GitHub owner\/repo/,
    );
  }
  const result = await fixture([{ events: [] }]).scan(context);
  assert.equal(result.repository, "example/service");
});

test("requests early exit only after finishing an empty scan", async () => {
  const f = fixture([{ events: [], nextToken: "next-page" }, { events: [] }]);
  const result = await f.scan(context);
  assert.equal(f.requests.length, 2);
  assert.equal(result.no_work, true);
  assert.deepEqual(result.groups, []);
  assert.equal(result.total_events, 0);
  assert.equal(result.truncated, false);
});
