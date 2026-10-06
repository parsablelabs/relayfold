export default function run({ inputs }) {
  const scan = inputs.find((input) => Array.isArray(input?.groups));
  const selection = inputs.find((input) =>
    Object.hasOwn(input ?? {}, "fingerprint"),
  );
  if (!scan || !selection)
    throw new Error("Missing pruned scan or group selection");
  if (typeof selection.reason !== "string" || !selection.reason.trim())
    throw new Error("Group selection requires a reason");
  const group = scan.groups.find(
    (group) => group.fingerprint === selection.fingerprint,
  );
  if (!group)
    throw new Error("Selected fingerprint must identify a pruned scan group");
  return { ...scan, groups: [group], _workflow_exit_reason: null };
}
