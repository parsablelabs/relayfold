import test from 'node:test'
import assert from 'node:assert/strict'
import { createApi, normalizeHost } from '../src/api.ts'
import type { Instance } from '../src/api.ts'

function instance(id: string, created: number | null): Instance {
  return { id, workflow_def_id: 'workflow', status: 'Running', created_at_epoch_ms: created, modified_at_epoch_ms: 999, completed_at_epoch_ms: null, completed_task_count: 0, total_task_count: 1 }
}

test('fetches all instance pages, filters on each page and orders by creation rather than API modification order', async () => {
  const calls: string[] = []
  const api = createApi({ host: 'localhost:3000', apiKey: 'key' }, async (path, options) => {
    const url = String(path)
    calls.push(url)
    assert.equal(new Headers(options?.headers).get('Authorization'), 'Bearer key')
    assert.equal(new Headers(options?.headers).get('X-RelayFold-Host'), 'http://localhost:3000')
    const query = new URL(url, 'http://frontend').searchParams
    assert.equal(query.get('status'), 'Running')
    assert.equal(query.get('limit'), '100')
    return Response.json(query.has('cursor')
      ? { workflows: [instance('new', 300), instance('unknown', null)], next_cursor: null }
      : { workflows: [instance('old', 100)], next_cursor: 'next page' })
  })
  const result = await api.instances('Running', new AbortController().signal)
  assert.deepEqual(result.map(value => value.id), ['new', 'old', 'unknown'])
  assert.equal(calls.length, 2)
  assert.equal(new URL(calls[1], 'http://frontend').searchParams.get('cursor'), 'next page')
})

test('loads all events with sequence cursors, preserving API event order', async () => {
  const api = createApi({ host: 'localhost:3000', apiKey: '' }, async path => {
    const url = new URL(String(path), 'http://frontend')
    assert.equal(url.pathname, '/api/workflows/a%2Fb/events')
    return Response.json(url.searchParams.get('after_sequence') === '2'
      ? { events: [{ created_time: 10, event: { type: 'last' } }], next_sequence: null }
      : { events: [{ created_time: 10, event: { type: 'first' } }], next_sequence: 2 })
  })
  const result = await api.events('a/b', new AbortController().signal)
  assert.deepEqual(result.map(value => value.event.type), ['first', 'last'])
})

test('encodes task attempt IDs and posts exact human input and lifecycle payloads', async () => {
  const calls: { path: string; body: unknown }[] = []
  const api = createApi({ host: 'localhost:3000', apiKey: '' }, async (path, options) => {
    assert.equal(options?.method, 'POST')
    assert.equal(new Headers(options.headers).get('Content-Type'), 'application/json')
    calls.push({ path: String(path), body: JSON.parse(String(options.body)) })
    return Response.json({ status: 'queued' })
  })
  await api.humanInput('run/a', 'agent[2]', { approved: true })
  await api.humanInput('run/a', 'agent[2]', 'yes')
  await api.action('run/a', 'pause')
  await api.action('run/a', 'resume')
  assert.deepEqual(calls, [
    { path: '/api/workflows/run%2Fa/tasks/agent%5B2%5D/human-input', body: { input: { approved: true } } },
    { path: '/api/workflows/run%2Fa/tasks/agent%5B2%5D/human-input', body: { input: 'yes' } },
    { path: '/api/workflows/run%2Fa/pause', body: {} },
    { path: '/api/workflows/run%2Fa/resume', body: {} },
  ])
})

test('exposes authentication and conflict errors and propagates cancellation', async () => {
  for (const [status, pattern] of [[401, /API key/], [409, /workflow state changed/], [502, /Cannot reach/]] as const) {
    const api = createApi({ host: 'localhost:3000', apiKey: '' }, async () => new Response('', { status }))
    await assert.rejects(api.report('run', new AbortController().signal), pattern)
  }
  const controller = new AbortController()
  controller.abort()
  const api = createApi({ host: 'localhost:3000', apiKey: '' }, async (_path, options) => {
    assert.equal(options?.signal, controller.signal)
    options.signal.throwIfAborted()
    return Response.json({})
  })
  await assert.rejects(api.report('run', controller.signal), { name: 'AbortError' })
})

test('validates host settings', () => {
  assert.equal(normalizeHost('localhost:4000'), 'http://localhost:4000')
  assert.equal(normalizeHost('https://example.com/'), 'https://example.com')
  for (const host of ['ftp://localhost', 'http://user:pass@localhost', 'http://localhost/path', 'http://localhost?token=secret']) {
    assert.throws(() => normalizeHost(host))
  }
})

test('restarts the selected failed attempt through normal retry, preserving worker placement', async () => {
  const api = createApi({ host: 'localhost:3000', apiKey: 'key' }, async (path, options) => {
    assert.equal(String(path), '/api/workflows/failed%2Frun/tasks/task%5B3%5D/retry')
    assert.equal(options?.method, 'POST')
    assert.equal(new Headers(options.headers).get('Authorization'), 'Bearer key')
    assert.deepEqual(JSON.parse(String(options.body)), {})
    return Response.json({ status: 'queued', task_attempt_id: 'task[3]', local_context_may_be_lost: false })
  })
  assert.deepEqual(await api.retryTask('failed/run', 'task[3]'), { status: 'queued', task_attempt_id: 'task[3]', local_context_may_be_lost: false })
})

test('reports retry conflicts when the attempt is no longer failed', async () => {
  const api = createApi({ host: 'localhost:3000', apiKey: '' }, async () => new Response('', { status: 409 }))
  await assert.rejects(api.retryTask('run', 'task[1]'), /workflow state changed/)
})

test('reads workflow configuration as YAML using an encoded definition ID', async () => {
  const controller = new AbortController()
  const yaml = 'id: example\ntasks: []\ndata_bindings: []\n'
  const api = createApi({ host: 'localhost:3000', apiKey: 'key' }, async (path, options) => {
    assert.equal(String(path), '/api/workflow-def/example%2Fworkflow?format=yaml')
    assert.equal(options?.method, 'GET')
    assert.equal(options.signal, controller.signal)
    assert.equal(new Headers(options.headers).get('Authorization'), 'Bearer key')
    return new Response(yaml, { headers: { 'Content-Type': 'application/yaml' } })
  })
  assert.equal(await api.definition('example/workflow', controller.signal), yaml)
})

test('starts a workflow with raw JSON trigger input or null when no input is provided', async () => {
  const bodies: unknown[] = []
  const api = createApi({ host: 'localhost:3000', apiKey: 'key' }, async (path, options) => {
    assert.equal(String(path), '/api/workflow-def/example%2Fworkflow')
    assert.equal(options?.method, 'POST')
    assert.equal(new Headers(options.headers).get('Content-Type'), 'application/json')
    assert.equal(new Headers(options.headers).get('Authorization'), 'Bearer key')
    bodies.push(JSON.parse(String(options.body)))
    return Response.json({ status: 'queued', id: 'new-instance', pinned_host_id: 'worker' })
  })
  assert.equal((await api.startWorkflow('example/workflow')).id, 'new-instance')
  await api.startWorkflow('example/workflow', { name: 'Ada' })
  assert.deepEqual(bodies, [null, { name: 'Ada' }])
})

test('explains missing workers when starting a workflow fails', async () => {
  const api = createApi({ host: 'localhost:3000', apiKey: '' }, async () => new Response('', { status: 503 }))
  await assert.rejects(api.startWorkflow('example'), /No eligible worker is available/)
})
