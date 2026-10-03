# Optional workflow sandboxing

Research date: 2026-10-01. Primary sources checked below. This comparison supplements [the earlier Gondolin investigation](gondolin-research.md); proposed RelayFold changes are engineering recommendations, not existing behavior.

## Candidates and naming ambiguity

**Gondolin** is [earendil-works/gondolin](https://github.com/earendil-works/gondolin). The host package currently declares `@earendil-works/gondolin` version `0.12.0`, Apache-2.0, and Node.js `>=23.6.0`. [Package metadata](https://github.com/earendil-works/gondolin/blob/main/host/package.json)

**pi-sandbox is ambiguous:** at least [carderne/pi-sandbox](https://github.com/carderne/pi-sandbox) and [code-yeongyu/pi-sandbox](https://github.com/code-yeongyu/pi-sandbox) use this name. Select the actual repository before choosing a dependency; their policies and backends differ.

| Candidate | Isolation and integration | Best RelayFold opportunity |
| --- | --- | --- |
| Gondolin | Linux VM controlled through a TypeScript SDK; host-mediated storage and networking | Backend for untrusted command execution and generated code |
| carderne/pi-sandbox | Pi extension checks read/write/edit and wraps bash using its sandbox-runtime fork | Reference for tool interception; underlying runtime is the reusable component |
| code-yeongyu/pi-sandbox | Pi policy manager with native, Docker, virtual-shell and QEMU backends | Reference for capability validation and backend contracts |

The first row is supported by [Gondolin security design](https://github.com/earendil-works/gondolin/blob/main/docs/security.md); the other rows by their respective [carderne README](https://github.com/carderne/pi-sandbox/blob/main/README.md) and [code-yeongyu README](https://github.com/code-yeongyu/pi-sandbox/blob/main/README.md).

## Gondolin

Gondolin trusts its host Node process and treats the guest as adversarial. QEMU supplies a separate guest kernel; explicit programmable VFS mounts expose host storage. Network traffic passes through a host stack, with HTTP/TLS mediation and optional SSH or mapped TCP exceptions. Secrets can remain on the host: the guest gets placeholders replaced in outbound headers only for matching destination hosts. This does not prevent misuse of allowed endpoints or reflection of credentials. Hypervisor escapes, malicious same-account host users, side channels and comprehensive DoS isolation are excluded from its guarantees. [Security design](https://github.com/earendil-works/gondolin/blob/main/docs/security.md)

**Network policy must be explicit:** `createHttpHooks()` omitting `allowedHosts` allows all hosts; an empty array denies all. Source also shows host callbacks run before the final secret injection, a detail that differs from the earlier note's logging warning. [HTTP hooks source](https://github.com/earendil-works/gondolin/blob/main/host/src/http/hooks.ts)

The SDK provides VM creation, execution, streaming and teardown. Abort stops waiting but does not guarantee guest process termination; enforcement needs VM teardown. [VM SDK](https://earendil-works.github.io/gondolin/sdk-vm/)

Operational gaps include disk-only checkpoints, no process/RAM restore, a minimal guest image and Alpine-only image builder, no HTTP/2 or HTTP/3, no generic UDP/QUIC/WebRTC, no Windows, and incomplete parity with experimental krun. [Limitations](https://earendil-works.github.io/gondolin/limitations/)

## Native sandbox / Pi extension path

The carderne extension wraps bash with macOS `sandbox-exec` or Linux `bubblewrap`; read/write/edit are checked in-process because those tools execute in the trusted Node process. It uses interactive grants, merges project/global permissions, and offers session disable controls. Those are local interactive product choices; unattended RelayFold execution should use a fixed host-owned policy and fail closed. [Extension README](https://github.com/carderne/pi-sandbox/blob/main/README.md)

The underlying [carderne/sandbox-runtime](https://github.com/carderne/sandbox-runtime) is independently usable as a CLI/library. Its native OS restrictions apply to spawned process trees, with HTTP/SOCKS proxies for domain filtering. Read access is broad unless explicitly denied; write and network access use allowlists. Per-session `createSandboxManager()` instances own proxy ports and permissions independently on macOS/Linux. It also documents experimental TLS termination, while some compatibility switches widen IPC or permit applications to launch outside the sandbox. Thus RelayFold should deliberately select a strict supported subset. [Runtime README](https://github.com/carderne/sandbox-runtime/blob/main/README.md)

The code-yeongyu project supports a different backend abstraction. Its feature matrix says restricted network mode is a configuration error across its listed backends; SSH is transport, not process isolation. Native execution persists host changes, while Docker/QEMU sessions are ephemeral. Environment scrubbing and structured denial attribution are useful design precedents. [README and matrix](https://github.com/code-yeongyu/pi-sandbox/blob/main/README.md)

## RelayFold integration recommendations

Local inspection by the parent investigation found that coding tools currently run against `process.cwd()` on the host; extension tools can override names. Resource extension loading wraps tools but does not visibly provide the lifecycle emissions and bindings these Pi extensions expect. Function subprocesses inherit `process.env`, and agent credential setup mutates that environment. Consequently installing a Pi extension alone cannot establish workflow containment. These are local code observations, not external project claims.

Recommended initial scope:

1. Define optional workflow policy, enforced by a trusted worker adapter. Keep orchestration, model calls and durable state outside the sandbox.
2. Apply the policy to all relevant command/file execution, including function subprocesses. Host HTTP/custom tools require explicit mediation or rejection; merely sandboxing bash leaves them privileged.
3. Allocate execution isolation per task attempt, with an explicit workspace persistence decision for retries. A workflow-level policy need not mean one shared VM for the whole workflow.
4. Start with one backend. Native runtime offers a smaller operational step for trusted developer workloads; Gondolin provides a stronger compute boundary and host-held credential mechanism for untrusted generated code.
5. Default sandboxed execution to explicit workspace paths, deny-all egress, a scrubbed environment, and no raw worker credentials. Validate backend capabilities before a run; never silently fall back to unsandboxed execution.
6. Treat policy as trusted configuration rather than agent-editable workspace content. Record policy/backend identity, denials and lifecycle events. Enforce deadlines and cleanup on completion, cancellation and worker loss.

Implementation should first settle whether the promise covers agent tools only or function execution too, and whether sandbox state survives retries. No code, public documentation or specification behavior was changed by this research.
