export default function run({ inputs }) {
  const scan = inputs.find((input) => Array.isArray(input?.groups));
  const fetched = inputs.find((input) => Array.isArray(input?.issues));
  if (!scan || !fetched) throw new Error("Missing scan or fetched open issues");
  if (!scan.repository || scan.repository !== fetched.repository)
    throw new Error(
      "Scan and fetched issues must identify the same repository",
    );

  const covered = new Set();
  for (const issue of fetched.issues) {
    for (const match of (issue.body ?? "").matchAll(
      /<!-- cloudwatch-log-scanner-cloudwatch:([0-9a-f]{64}) -->/g,
    ))
      covered.add(match[1]);
  }
  return {
    ...scan,
    groups: scan.groups.filter((group) => !covered.has(group.fingerprint)),
  };
}
