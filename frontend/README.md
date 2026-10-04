# RelayFold frontend

A React dashboard for the orchestrator's public API. Browse registered workflows,
open a workflow definition to see its Mermaid task diagram and registered YAML,
and start a new instance with optional JSON trigger input,
view instances newest first by creation time, filter by status, and open an
instance to inspect task attempts and event logs with the latest event at the top. Instance details
support plain-text or JSON human input, pause, resume, and restarting failed tasks. Views refresh every
five seconds, with a manual refresh button. Completed workflows are labeled
**Success**. Termination is not available.

## Start locally

```bash
cd frontend
npm install
npm run dev
```

Open the URL printed by Vite (usually http://localhost:5173). Start the
orchestrator separately; the frontend does not start it.

In **Settings**, enter the public API host, such as `http://localhost:3000`.
The host is saved in your browser. If namespace authentication is enabled,
enter a bearer API key; the key is kept in memory until the page reloads.

The default is localhost on the orchestrator's default public port, 3000.
To match a custom orchestrator port, pass its address when starting Vite:

```bash
RELAYFOLD_PUBLIC_HTTP_ADDR=0.0.0.0:4000 npm run dev
```

Or set `VITE_DEFAULT_API_HOST=http://localhost:4000` in `frontend/.env.local`.
The frontend cannot discover environment variables of an independently running
orchestrator; Settings overrides the default.

API requests go through the frontend server's `/api` proxy, so the orchestrator
needs no CORS changes. The configured host must be reachable from that server.
The proxy runs in both the Vite development and preview servers. Keep these
servers on a trusted local network.

## Checks and production preview

```bash
npm run lint
npm run build
npm run preview
```

For API integration tests (Node 22.18 or newer):

```bash
npm test
```

A static deployment needs a server implementing the same `/api` forwarding
contract (the `X-RelayFold-Host` request header selects the public API origin).
The `dist` files alone do not provide a proxy.

## Behavior to check

- Register workflows and trigger multiple instances through the orchestrator.
- Click a registered workflow name to inspect its diagram and YAML. Diagram
  arrows follow data bindings; isolated tasks stay independent. Diamonds mark
  verifiers and dashed arrows show explicit rerun targets. The YAML returned by
  the API represents the stored definition, without original file comments.
- Click **Start workflow** on a definition page to queue a new instance and open
  its details. Optionally expand **Trigger input** and enter valid JSON. An
  eligible worker must be connected; connection and worker errors are shown
  on the definition page.
- Check that all registered workflows appear and instances are ordered by creation
  time, even when an older instance was recently modified.
- Filter each status, open an instance, and verify its task attempts and event log.
- Pause a pending/running instance; resume a paused instance. In-flight tasks can
  finish while paused, but further execution waits for resume.
- For a workflow awaiting human input, submit a response to the specified task
  attempt; confirm its continuation appears after refresh.
- For a failed instance, use **Restart task** on a failed task attempt. The task
  is queued again on its existing worker host, preserving its local context.
  Restart controls are available only when both the instance and task are failed;
  force retry to another host is not exposed.
- Change the host or API key in Settings and verify connection errors are visible.

The instance list loads all API pages before sorting because the API itself
orders by modification time. Large histories require more requests per refresh.
Events are fetched in API sequence order, including every page, and displayed
newest first while retaining their original event numbers.
