export const statuses = ['Pending', 'Running', 'Paused', 'InputNeeded', 'Completed', 'Failed'] as const
export type Status = typeof statuses[number]
export type Connection = { host: string; apiKey: string }
export type Workflow = { id: string; description: string; created_at_epoch_ms: number; last_invoked_at_epoch_ms: number | null }
export type FunctionSummary = { id: string }
export type FunctionDefinition = { id: string; dependencies: { name: string; version: string }[]; code: string }
export type Instance = { id: string; workflow_def_id: string; status: Status; created_at_epoch_ms: number | null; modified_at_epoch_ms: number; completed_at_epoch_ms: number | null; completed_task_count: number; total_task_count: number }
export type Task = { task_attempt_id: string; task_def_id: string; generation_index: number; satisfaction: string; status: Exclude<Status, 'Paused' | 'InputNeeded'> | { InputNeeded: { input_request: string } } }
export type Report = { instance_id: string; workflow_def_id: string; status: Status; tasks: Task[]; verifier_states?: unknown[] }
export type EventRecord = { created_time: number; event: { type: string; [key: string]: unknown } }
export type InstancePage = { workflows: Instance[]; next_cursor: string | null }
export type EventPage = { events: EventRecord[]; next_sequence: number | null }

export function normalizeHost(host: string): string {
  const url = new URL(host.includes('://') ? host : `http://${host}`)
  if (!['http:', 'https:'].includes(url.protocol) || url.username || url.password || url.search || url.hash || url.pathname !== '/') {
    throw new Error('Enter an HTTP(S) host and port, for example http://localhost:3000.')
  }
  return url.origin
}

export function newestFirst(instances: Instance[]): Instance[] {
  return [...instances].sort((a, b) => (b.created_at_epoch_ms ?? -1) - (a.created_at_epoch_ms ?? -1) || b.id.localeCompare(a.id))
}

export function createApi(connection: Connection, transport: typeof fetch = fetch) {
  async function request<T>(path: string, signal?: AbortSignal, body?: unknown, format: 'json' | 'text' = 'json', bodyFormat: 'json' | 'yaml' = 'json'): Promise<T> {
    const response = await transport(`/api${path}`, {
      method: body === undefined ? 'GET' : 'POST',
      signal,
      headers: {
        'X-RelayFold-Host': normalizeHost(connection.host),
        ...(connection.apiKey ? { Authorization: `Bearer ${connection.apiKey}` } : {}),
        ...(body === undefined ? {} : { 'Content-Type': bodyFormat === 'yaml' ? 'application/yaml' : 'application/json' }),
      },
      ...(body === undefined ? {} : { body: bodyFormat === 'yaml' ? String(body) : JSON.stringify(body) }),
    })
    if (!response.ok) {
      const explanation = response.status === 401 ? 'Check your API key in Settings.'
        : response.status === 409 ? path === '/workflow-def' ? 'This workflow already has instances and cannot be overwritten. Register it under a new ID.' : 'The workflow state changed or this action is no longer available. Refresh and try again.'
        : response.status === 404 ? 'The workflow, function, or task could not be found.'
        : response.status === 503 ? 'No eligible worker is available. Start or reconnect a worker and try again.'
        : response.status === 502 ? 'Cannot reach the orchestrator. Check the host in Settings and ensure it is running.'
        : await response.text()
      throw new Error(`API request failed (${response.status}). ${explanation}`)
    }
    return (format === 'text' ? response.text() : response.json()) as Promise<T>
  }
  return {
    functions: (signal: AbortSignal) => request<{ function_defs: FunctionSummary[] }>('/function-def', signal),
    functionDefinition: (id: string, signal: AbortSignal) => request<FunctionDefinition>(`/function-def/${encodeURIComponent(id)}`, signal),
    registerFunction: (definition: string) => request<{ id: string }>('/function-def', undefined, definition, 'json', 'yaml'),
    registerWorkflow: (definition: string) => request<{ id: string }>('/workflow-def', undefined, definition, 'json', 'yaml'),
    startWorkflow: (id: string, input: unknown = null) => request<{ id: string }>(`/workflow-def/${encodeURIComponent(id)}`, undefined, input),
    definition: (id: string, signal: AbortSignal) => request<string>(`/workflow-def/${encodeURIComponent(id)}?format=yaml`, signal, undefined, 'text'),
    workflows: (signal: AbortSignal) => request<{ workflow_defs: Workflow[] }>('/workflow-def', signal),
    async instances(status: string, signal: AbortSignal): Promise<Instance[]> {
      const instances = new Map<string, Instance>()
      let cursor: string | null = null
      do {
        const query = new URLSearchParams({ limit: '100' })
        if (status) query.set('status', status)
        if (cursor) query.set('cursor', cursor)
        const page: InstancePage = await request(`/workflows?${query}`, signal)
        page.workflows.forEach(instance => instances.set(instance.id, instance))
        cursor = page.next_cursor
      } while (cursor !== null)
      return newestFirst([...instances.values()])
    },
    report: (id: string, signal: AbortSignal) => request<Report>(`/workflows/${encodeURIComponent(id)}`, signal),
    async events(id: string, signal: AbortSignal): Promise<EventRecord[]> {
      const events: EventRecord[] = []
      let sequence: number | null = null
      do {
        const query = new URLSearchParams({ limit: '100' })
        if (sequence !== null) query.set('after_sequence', String(sequence))
        const page: EventPage = await request(`/workflows/${encodeURIComponent(id)}/events?${query}`, signal)
        events.push(...page.events)
        sequence = page.next_sequence
      } while (sequence !== null)
      return events
    },
    action: (id: string, action: 'pause' | 'resume') => request(`/workflows/${encodeURIComponent(id)}/${action}`, undefined, {}),
    retryTask: (id: string, task: string) => request(`/workflows/${encodeURIComponent(id)}/tasks/${encodeURIComponent(task)}/retry`, undefined, {}),
    humanInput: (id: string, task: string, input: unknown) => request(`/workflows/${encodeURIComponent(id)}/tasks/${encodeURIComponent(task)}/human-input`, undefined, { input }),
  }
}
