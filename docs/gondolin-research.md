# Gondolin as a RelayFold task sandbox

Research date: 2026-09-14

Scope: Gondolin's architecture, execution model, security boundaries, integration surface, limitations, and likely fit as a RelayFold sandbox. This note uses only Gondolin's first-party repository, documentation, source, package metadata, and releases. Statements about RelayFold integration are engineering inferences from those sources, not claims made by Gondolin.

## Executive summary

Gondolin is a local agent sandbox built from a TypeScript/Node.js host control plane, a small Linux guest runtime, and a VM backend. QEMU is the default and documented primary isolation boundary; `libkrun` is an experimental alternative. Its distinctive feature is not only VM isolation but host-mediated I/O: the host controls command execution, network egress, secret substitution, and optional filesystem mounts. [README](https://github.com/earendil-works/gondolin/blob/main/README.md) · [Architecture](https://earendil-works.github.io/gondolin/architecture/) · [Security design](https://github.com/earendil-works/gondolin/blob/main/docs/security.md)

For RelayFold, Gondolin looks like a **good fit for sandboxing untrusted task execution/tools**, especially shell commands, generated code, builds, and filesystem mutations. It is a less natural fit for putting RelayFold's whole orchestration/control plane inside the VM. Gondolin assumes its host Node.js process is trusted and deliberately makes that host the policy enforcement point. A clean boundary would therefore keep workflow scheduling, durable state, policy construction, model credentials, and observability in RelayFold, while routing an AI task's untrusted command/file operations to a task-scoped Gondolin VM. [Security design](https://github.com/earendil-works/gondolin/blob/main/docs/security.md) · [Pi integration example](https://github.com/earendil-works/gondolin/blob/main/host/examples/pi-gondolin.ts)

This should be treated as a promising backend to prototype, not an immediately production-complete containment layer. The repository describes itself as experimental; the current npm package is `0.12.0`; QEMU escape risk, same-user host attacks, side channels, and comprehensive denial-of-service isolation are explicitly outside its guarantees. [README](https://github.com/earendil-works/gondolin/blob/main/README.md) · [Package metadata](https://github.com/earendil-works/gondolin/blob/main/host/package.json) · [Security design](https://github.com/earendil-works/gondolin/blob/main/docs/security.md)

## How Gondolin works

### Components and trust boundary

The system has three main pieces:

- The **host control plane**, imported as `@earendil-works/gondolin` or used through its CLI, starts/stops VMs, sends execution requests, serves programmable filesystems, mediates network policy, and resolves guest assets.
- The **Linux guest** contains `sandboxd` for command execution, `sandboxfs` for FUSE-to-host filesystem RPC, `sandboxssh` for loopback-only host-to-guest forwarding, `sandboxingress` for inbound HTTP forwarding, and init scripts that set up tmpfs, networking, and services.
- The **VM backend** supplies compute isolation. QEMU is the default; `libkrun` is optional and experimental. Both use the same higher-level exec, VFS, network, SSH, and ingress control plane, though backend configuration and some runtime behavior differ. [Architecture](https://earendil-works.github.io/gondolin/architecture/) · [Backend matrix](https://earendil-works.github.io/gondolin/backends/)

The guest is considered adversarial; the host Node.js process, its policy and secrets, the guest image supply chain, and the VM boundary are trusted. With QEMU, the documented device surface is intentionally small (`virtio-serial`, `virtio-net`, `virtio-blk`, and `virtio-rng`), and rootfs writes normally go to a disposable qcow2 copy-on-write overlay. [Security design](https://github.com/earendil-works/gondolin/blob/main/docs/security.md)

```text
RelayFold host (trusted)
  policy, credentials, state, audit
             |
             | Node adapter / CLI
             v
Gondolin host control plane (trusted)
  exec RPC | VFS RPC | network mediation
             |
             | virtio channels
             v
Linux micro-VM (untrusted)
  agent tools / shell / generated code
```

### Task/command execution

The normal SDK lifecycle is `VM.create(...)`, one or more `vm.exec(...)` calls, and `vm.close()`. For each exec, the host sends a structured request over virtio-serial, `sandboxd` starts the process in the guest, stdio is streamed over the channel, and the host produces an `ExecResult`. A string command runs through `/bin/sh -lc`; an argv array executes an absolute executable directly. Non-zero guest exits return an `ExecResult` rather than throwing. Output may be buffered, streamed with credit-based backpressure, inherited, or dropped. [Architecture](https://earendil-works.github.io/gondolin/architecture/) · [VM SDK](https://earendil-works.github.io/gondolin/sdk-vm/)

Cancellation has an important limitation: aborting an exec rejects the local promise but does not currently guarantee termination of the process in the guest. The ingress documentation also states that the guest currently executes one command at a time, so one long-running `vm.exec()` blocks further exec requests. RelayFold would need to regard VM teardown as the reliable hard-stop mechanism and either serialize tool calls per VM or allocate separate VMs when task-level parallelism is required. [VM SDK](https://earendil-works.github.io/gondolin/sdk-vm/) · [Network SDK](https://earendil-works.github.io/gondolin/sdk-network/)

Gondolin's intended lifecycle closely matches a task-scoped sandbox: boot a VM, run commands, persist required outputs to an explicit mount or external service, then discard the VM. The project recommends aligning VM lifetime with an agent task or turn and reports sub-second boot, while warning that idle QEMU processes retain memory. Durable state belongs in VFS-backed paths; the ordinary rootfs and tmpfs paths are disposable. [Workloads and lifecycle](https://earendil-works.github.io/gondolin/workloads/)

## Isolation and security model

### Compute

Guest code executes against a guest Linux kernel rather than directly against the host kernel. The documented guarantee is that, absent a QEMU escape, guest processes cannot directly access host kernel memory or files that were not explicitly exposed. This is stronger separation than a same-kernel process or ordinary container boundary, but it inherits the VMM's attack surface and patching requirements. [Security design](https://github.com/earendil-works/gondolin/blob/main/docs/security.md)

CPU count and memory size are configurable for both backends, and a host runner PID is available for external metrics. However, Gondolin explicitly does not claim complete denial-of-service isolation: guest code may burn CPU, allocate memory, or induce host work. RelayFold would still need outer process/VM supervision, timeouts, quotas, concurrency limits, disk limits, and host-level monitoring. [Backend matrix](https://earendil-works.github.io/gondolin/backends/) · [VM SDK](https://earendil-works.github.io/gondolin/sdk-vm/) · [Security design](https://github.com/earendil-works/gondolin/blob/main/docs/security.md)

### Network

Gondolin does not attach the guest to generic host NAT. The guest emits Ethernet frames through virtio-net; the host runs a small userspace network stack, classifies TCP as mapped TCP, HTTP/1.x, TLS, or optionally SSH, and denies unknown protocols. Plain HTTP is parsed and replayed with host `fetch`; TLS is intercepted with a Gondolin-controlled CA so that the same HTTP policy can be applied. UDP is blocked except for DNS, whose default `synthetic` mode returns synthetic addresses without upstream DNS. HTTP `CONNECT` is denied. [Architecture](https://earendil-works.github.io/gondolin/architecture/) · [Security design](https://github.com/earendil-works/gondolin/blob/main/docs/security.md)

Network mediation is not automatically a destination-deny policy. `createHttpHooks()` documents that an omitted `allowedHosts` means allow-all and an explicit empty list means deny-all; its source maps omission to `['*']`. A RelayFold adapter must therefore construct hooks explicitly and should default to `createHttpHooks({ allowedHosts: [] })`, opening exact hosts per task capability. Merely using synthetic DNS is not an egress allowlist. [HTTP hooks source](https://github.com/earendil-works/gondolin/blob/main/host/src/http/hooks.ts)

For HTTP/TLS, hooks can allow/deny by hostname, resolved IP, request method/path/content, and can inspect or rewrite requests/responses. Internal address ranges are blocked by default when `createHttpHooks()` is used, and redirects are rechecked host-side. Explicit mapped TCP and SSH egress are narrower escape hatches but bypass the HTTP hooks and HTTP secret-substitution path. They should be exposed as separate high-risk RelayFold capabilities rather than folded into a general `network: true` flag. [Network SDK](https://earendil-works.github.io/gondolin/sdk-network/) · [Security design](https://github.com/earendil-works/gondolin/blob/main/docs/security.md)

WebSockets are supported after an HTTP/1.1 upgrade, but after the `101` response the connection becomes an opaque byte tunnel; only the handshake is hookable. They can be disabled. Custom `onRequest` hooks also require care: secrets may already be expanded when logging occurs, and a hook that performs its own host-side `fetch()` is outside the VM egress policy. [Network SDK](https://earendil-works.github.io/gondolin/sdk-network/)

### Secrets

Gondolin can give the guest a random placeholder environment value instead of a real credential. For HTTP/TLS-mediated traffic, the host substitutes the real value in outbound headers only when the destination matches that secret's host allowlist; Basic authentication is decoded, substituted, and re-encoded. Query-string substitution is opt-in because of reflection risk. The real secret therefore need not enter guest environment, disk, or memory. [README](https://github.com/earendil-works/gondolin/blob/main/README.md) · [HTTP hooks source](https://github.com/earendil-works/gondolin/blob/main/host/src/http/hooks.ts) · [Security design](https://github.com/earendil-works/gondolin/blob/main/docs/security.md)

This mechanism narrows credential theft but does not make an allowed service harmless. Malicious code may send any guest-readable data to any allowed host, and an allowed endpoint that reflects headers can reveal an injected secret back to the guest. Secret substitution does not apply to arbitrary request bodies or mapped TCP/SSH. RelayFold should issue short-lived least-privilege task credentials, keep host patterns exact, restrict methods and paths when feasible, and treat all mounted readable data as exfiltratable to allowed destinations. [Security design](https://github.com/earendil-works/gondolin/blob/main/docs/security.md)

### Filesystem and persistence

The guest root filesystem is image-backed and usually ephemeral. Host-visible paths are explicit VFS mounts served by guest FUSE and host JavaScript providers. Built-ins include in-memory storage, direct host directories, read-only wrappers, and shadow wrappers that can hide entries such as `.env`, `.npmrc`, `node_modules`, `.git`, or other sensitive/host-specific trees. `RealFSProvider` documents fail-closed handling of symlinks that escape the exposed directory. Custom providers and before/after VFS hooks can implement synthetic content, audit access, or enforce additional policy. [VFS providers](https://earendil-works.github.io/gondolin/vfs/) · [Security design](https://github.com/earendil-works/gondolin/blob/main/docs/security.md)

The safest RelayFold default would be an in-memory `/workspace`, with small read-only input mounts and a distinct writable `/out` provider for declared artifacts. If in-place repository editing is required, expose only the task checkout through a `RealFSProvider` wrapped to shadow credentials and irrelevant host directories; never mount the host home or filesystem root. VFS data is not included in VM disk checkpoints, so output persistence must be explicit. [VFS providers](https://earendil-works.github.io/gondolin/vfs/) · [Workloads and lifecycle](https://earendil-works.github.io/gondolin/workloads/)

## Integration surface

### JavaScript/TypeScript SDK

The primary API is the public npm package `@earendil-works/gondolin` (currently version `0.12.0`, Apache-2.0, Node.js `>=23.6.0`). The SDK exposes VM lifecycle, buffered/streaming exec, interactive PTY shells, guest filesystem helpers, configurable VFS providers, network and secret hooks, ingress, SSH access/egress, image resolution, and disk checkpoints. [Package metadata](https://github.com/earendil-works/gondolin/blob/main/host/package.json) · [SDK overview](https://earendil-works.github.io/gondolin/sdk/) · [VM SDK](https://earendil-works.github.io/gondolin/sdk-vm/) · [Network SDK](https://earendil-works.github.io/gondolin/sdk-network/)

For a non-Node RelayFold runtime, the most capable integration would be a small trusted Node adapter/sidecar with a narrow RelayFold-facing contract such as `createSandbox(policy)`, `exec(command, io, deadline)`, `collectArtifacts()`, and `close()`. This preserves streaming, hooks, VFS composition, and secret handling without embedding Node APIs into RelayFold's domain model. This is an inference from Gondolin's SDK-only programmatic interface.

### CLI

The `gondolin` CLI supports interactive `bash`, session `list`/`attach`, `snapshot`, non-interactive `exec`, guest image `build`, and local image management. It accepts host and memory mounts, allowed HTTP hosts, host secrets, DNS modes, mapped TCP, and SSH policy. A subprocess-based CLI adapter is therefore a reasonable spike, but it is less suitable as the final integration if RelayFold needs structured streaming, runtime secret rotation, fine-grained hooks, or rich audit events. [CLI](https://earendil-works.github.io/gondolin/cli/)

### Existing agent integration pattern

The repository's Pi example keeps the agent on the host, starts one VM for the session, mounts the working directory at `/workspace`, maps host paths to guest paths, and overrides the agent's read/write/edit/bash tool operations to execute in Gondolin. This is strong first-party evidence for the recommended RelayFold integration seam: a sandboxed tool executor beneath an otherwise trusted orchestration runtime. The example is a pattern, not a secure default policy; its source imports `RealFSProvider` and `VM` but does not construct an HTTP allowlist. [Pi integration example](https://github.com/earendil-works/gondolin/blob/main/host/examples/pi-gondolin.ts)

## Constraints and operational risks

- **Maturity:** the project and krun backend are described as experimental; pin the package and guest image manifests, qualify upgrades, and do not assume API or image stability from a pre-1.0 package. [README](https://github.com/earendil-works/gondolin/blob/main/README.md) · [Releases](https://github.com/earendil-works/gondolin/releases)
- **Host/platform requirements:** Linux and macOS are supported, Windows is not; QEMU and Node.js `>=23.6.0` are required for the standard path. ARM64 is the most tested runtime path. Initial guest assets are roughly 200 MB or more and are downloaded/cached on first use. [README](https://github.com/earendil-works/gondolin/blob/main/README.md) · [CLI](https://earendil-works.github.io/gondolin/cli/) · [Current limitations](https://earendil-works.github.io/gondolin/limitations/)
- **Image/tool availability:** the default image is intentionally minimal. The built-in custom image builder currently supports Alpine, though OCI root filesystems can be used as a base. Adding required compilers/runtimes/tools generally means producing a custom image. [Custom images](https://github.com/earendil-works/gondolin/blob/main/docs/custom-images.md) · [Current limitations](https://earendil-works.github.io/gondolin/limitations/)
- **Protocol compatibility:** HTTP/2, HTTP/3, QUIC, WebRTC, generic UDP, and arbitrary TCP are not available in the default mediated model. HTTP/1.x, HTTPS via interception, WebSocket upgrade, explicit SSH, and explicit mapped TCP cover many coding-agent use cases but not all task workloads. [Current limitations](https://earendil-works.github.io/gondolin/limitations/) · [Network SDK](https://earendil-works.github.io/gondolin/sdk-network/)
- **Persistence:** checkpoints capture disk state, not RAM or process state; several guest paths are tmpfs and excluded; VFS data is separately provider-backed. Restored checkpoints also depend on compatible image assets. [Current limitations](https://earendil-works.github.io/gondolin/limitations/) · [Workloads and lifecycle](https://earendil-works.github.io/gondolin/workloads/)
- **Concurrency and termination:** command execution is currently serialized within a guest, and cancellation alone does not guarantee guest process death. VM-level close/kill plus host deadlines are needed for enforcement. [VM SDK](https://earendil-works.github.io/gondolin/sdk-vm/) · [Network SDK](https://earendil-works.github.io/gondolin/sdk-network/)
- **Threat-model exclusions:** malicious host code, malicious users with the same host account, hypervisor escape, side channels, and full DoS containment are out of scope. Unix sockets and caches are accessible to same-account local attackers under the documented assumptions. [Security design](https://github.com/earendil-works/gondolin/blob/main/docs/security.md)
- **Supply chain and CA:** default assets come from GitHub releases; high-assurance deployments should build/verify images and checksums. The TLS interception CA private key is host-sensitive; use a per-run certificate directory when stronger separation is required and protect logs because request hooks may observe expanded secrets. [Security design](https://github.com/earendil-works/gondolin/blob/main/docs/security.md)

## Recommended RelayFold proof of concept

1. Introduce a sandbox-executor interface at the boundary where RelayFold currently performs command/file tool calls; keep workflow orchestration, event storage, and model calls on the trusted host.
2. Implement a Node-based Gondolin adapter first, with one disposable VM per RelayFold task and explicit cleanup on success, failure, cancellation, or worker loss. Use the CLI only for an initial feasibility spike.
3. Default to deny-all HTTP (`allowedHosts: []`), synthetic DNS, no SSH, no mapped TCP, no ingress, and no WebSockets. Grant exact network capabilities from task policy.
4. Start with an in-memory workspace plus explicit read-only inputs and writable artifact output. If RelayFold must edit a checkout in place, mount only that checkout and shadow `.env`, credential/config files, sockets, and host dependency trees.
5. Provide secrets only through Gondolin's placeholder substitution, scoped to exact hosts and preferably paths/methods via request hooks; never pass raw credentials in `VM.env` or mounted files.
6. Enforce hard wall-clock deadlines by tearing down the VM, and place outer CPU, memory, disk, and process-count controls around the Gondolin/QEMU process. Treat `vm.exec` cancellation as cooperative only.
7. Export structured lifecycle, exec, VFS, egress, resource, and teardown events into RelayFold observability. Redact HTTP-hook logs because they may contain expanded secrets.
8. Validate representative tasks against the protocol/image constraints: repository checkout, package install, build/test, artifact extraction, LLM API streaming, cancellation, runaway fork/memory/CPU behavior, and prompt-injection exfiltration attempts.

## Bottom line

Gondolin's design is closely aligned with the risky half of an agentic workflow orchestrator: executing model-directed code while strictly mediating filesystem, network, and credentials. Its host-side policy model and disposable task lifecycle make it a compelling RelayFold sandbox backend. The recommended seam is **RelayFold task/tool executor -> Gondolin VM**, not **RelayFold control plane -> inside Gondolin VM**. Adoption should proceed behind a backend interface and a security-focused proof of concept because fail-closed network configuration, outer resource enforcement, process termination, platform packaging, custom images, and pre-1.0 maturity remain RelayFold's responsibility.
