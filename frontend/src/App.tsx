import { useCallback, useEffect, useMemo, useState } from 'react'
import type { FormEvent } from 'react'
import { createApi, normalizeHost, statuses } from './api'
import type { Connection, Report, Status, Task } from './api'
import { diagramFromYaml } from './workflowGraph'
import type { Definition } from './workflowGraph'
import { createTriggerValidator } from './triggerInput'
import WorkflowDiagram from './WorkflowDiagram'
import './index.css'

const defaultHost = import.meta.env.VITE_DEFAULT_API_HOST || 'http://localhost:3000'
function savedHost() {
  try { return normalizeHost(localStorage.getItem('relayfold-api-host') || defaultHost) }
  catch { return defaultHost }
}
function date(value: number | null | undefined) {
  return value == null ? 'Unknown' : new Date(value).toLocaleString()
}
function label(value: string) { return value === 'InputNeeded' ? 'Input needed' : value === 'Completed' ? 'Success' : value }
function Badge({ status }: { status: string }) {
  return <span className={`badge status-${status.toLowerCase()}`}>{label(status)}</span>
}
function taskStatus(task: Task) { return typeof task.status === 'string' ? task.status : 'InputNeeded' }
function message(error: unknown) { return error instanceof Error ? error.message : String(error) }

// One request at a time; navigation and refresh cancel obsolete reads.
function usePolling<T>(load: (signal: AbortSignal) => Promise<T>) {
  const [state, setState] = useState<{ data?: T; error?: string; updated?: number }>({})
  const [revision, setRevision] = useState(0)
  useEffect(() => {
    const controller = new AbortController()
    let timer: ReturnType<typeof setTimeout>
    async function run() {
      try {
        const data = await load(controller.signal)
        if (!controller.signal.aborted) setState({ data, updated: Date.now() })
      } catch (error) {
        if (!controller.signal.aborted) setState(previous => ({ ...previous, error: message(error) }))
      } finally {
        if (!controller.signal.aborted) timer = setTimeout(run, 5000)
      }
    }
    void run()
    return () => { controller.abort(); clearTimeout(timer) }
  }, [load, revision])
  return { ...state, refresh: () => setRevision(value => value + 1) }
}
function Feedback({ error, loading, empty }: { error?: string; loading: boolean; empty?: boolean }) {
  return <>{error && <p className="notice error" role="alert">{error}</p>}
    {loading && !error && <p className="notice" role="status">Loading…</p>}
    {empty && <p className="notice">No results found.</p>}</>
}
function Refresh({ updated, refresh }: { updated?: number; refresh: () => void }) {
  return <div className="refresh"><span>Refreshes every 5 seconds{updated ? ` · Updated ${new Date(updated).toLocaleTimeString()}` : ''}</span><button className="btn btn-secondary" onClick={refresh}>Refresh</button></div>
}
type Api = ReturnType<typeof createApi>

function Workflows({ api, open }: { api: Api; open: (id: string) => void }) {
  const load = useCallback((signal: AbortSignal) => api.workflows(signal), [api])
  const state = usePolling(load)
  return <>
    <Refresh {...state} />
    <Feedback error={state.error} loading={!state.data} empty={state.data?.workflow_defs.length === 0} />
    <div className="workflow-grid">{state.data?.workflow_defs.map(workflow => <article className="glass-panel workflow-card" key={workflow.id}>
      <div className="eyebrow">Registered workflow</div><h2><button className="text-button" onClick={() => open(workflow.id)}>{workflow.id}</button></h2><p>{workflow.description || 'No description provided.'}</p>
      <dl><dt>Registered</dt><dd>{date(workflow.created_at_epoch_ms)}</dd><dt>Last invoked</dt><dd>{workflow.last_invoked_at_epoch_ms == null ? 'Never' : date(workflow.last_invoked_at_epoch_ms)}</dd></dl>
    </article>)}</div>
  </>
}
function StartWorkflow({ api, id, definition, openInstance }: { api: Api; id: string; definition: Definition; openInstance: (id: string) => void }) {
  const [input, setInput] = useState('')
  const [starting, setStarting] = useState(false)
  const [error, setError] = useState('')
  const validator = useMemo(() => createTriggerValidator(definition), [definition])
  const validation = useMemo(() => validator.validate(input), [validator, input])
  async function submit(event: FormEvent) {
    event.preventDefault()
    if (starting) return
    setError('')
    if (validation.errors.length) return
    setStarting(true)
    try {
      const instance = await api.startWorkflow(id, validation.payload)
      openInstance(instance.id)
    } catch (error) { setError(message(error)) }
    finally { setStarting(false) }
  }
  return <form className="glass-panel panel input-form" onSubmit={submit}>
    <h3>Start workflow</h3>
    <div className="trigger-input-grid">
      <div><label>JSON trigger input<textarea rows={12} value={input} disabled={starting} aria-invalid={validation.errors.length > 0} aria-describedby="trigger-validation" onChange={event => { setInput(event.target.value); setError('') }} placeholder={'{"name": "Ada"}'} /></label><p className="muted">{validator.schemas.length ? 'Input must satisfy every entry task schema shown.' : 'No entry task declares an input schema. Input is optional.'}</p></div>
      <div className="trigger-schemas"><h4>Input schema</h4>{validator.schemas.length ? validator.schemas.map(({ taskId, index, schema }) => <label key={`${taskId}:${index}`}>{taskId} · input slot {index + 1}<textarea rows={12} readOnly value={JSON.stringify(schema, null, 2)} /></label>) : <p className="notice">No input schema defined.</p>}</div>
    </div>
    <div id="trigger-validation" aria-live="polite">{validation.errors.length > 0 && <ul className="notice error">{validation.errors.map((error, index) => <li key={index}>{error}</li>)}</ul>}</div>
    <button className="btn btn-primary" disabled={starting || validation.errors.length > 0}>{starting ? 'Starting…' : 'Start workflow'}</button>
    {error && <p className="notice error" role="alert">{error}</p>}
  </form>
}
function WorkflowDetails({ api, id, back, openInstance }: { api: Api; id: string; back: () => void; openInstance: (id: string) => void }) {
  const load = useCallback(async (signal: AbortSignal) => {
    const yaml = await api.definition(id, signal)
    try { return { yaml, diagram: diagramFromYaml(yaml), error: undefined } }
    catch (error) { return { yaml, diagram: undefined, error: message(error) } }
  }, [api, id])
  const state = usePolling(load)
  const diagram = state.data?.diagram
  return <>
    <button className="text-button back" onClick={back}>← Back to workflows</button>
    <Refresh {...state} /><Feedback error={state.error} loading={!state.data} />
    {state.data && <>
      <section className="glass-panel panel"><h2>{diagram?.definition.id ?? id}</h2><p className="muted">{diagram?.definition.description}</p></section>
      {diagram && <StartWorkflow api={api} id={id} definition={diagram.definition} openInstance={openInstance} />}
      <h2 className="section-title">Workflow diagram</h2>
      <p className="muted diagram-legend">Arrows show data bindings. Diamonds mark verifiers; dashed arrows show explicitly configured rerun targets. Tasks without bindings are independent.</p>
      <section className="glass-panel panel">
        {state.data.error && <p className="notice error" role="alert">Unable to build workflow diagram: {state.data.error}</p>}
        {diagram && (diagram.definition.tasks.length ? <WorkflowDiagram key={diagram.source} source={diagram.source} /> : <p className="notice">This workflow has no tasks.</p>)}
      </section>
      <details className="glass-panel panel"><summary>Registered YAML configuration</summary><pre>{state.data.yaml}</pre></details>
    </>}
  </>
}
function Instances({ api, open }: { api: Api; open: (id: string) => void }) {
  const [status, setStatus] = useState('')
  return <><div className="filters"><label htmlFor="status-filter">Status</label><select id="status-filter" value={status} onChange={event => setStatus(event.target.value)}>
    <option value="">All statuses</option>{statuses.map(value => <option key={value} value={value}>{label(value)}</option>)}
  </select><span>Newest first by creation time</span></div><InstanceList key={status} api={api} status={status} open={open} /></>
}
function InstanceList({ api, status, open }: { api: Api; status: string; open: (id: string) => void }) {
  const load = useCallback((signal: AbortSignal) => api.instances(status, signal), [api, status])
  const state = usePolling(load)
  return <><Refresh {...state} /><Feedback error={state.error} loading={!state.data} empty={state.data?.length === 0} />
    {state.data && state.data.length > 0 && <div className="glass-panel table-scroll"><table><thead><tr><th>Instance / Workflow</th><th>Status</th><th>Created</th><th>Tasks completed</th><th>Last modified</th></tr></thead>
      <tbody>{state.data.map(instance => <tr key={instance.id}><td><button className="text-button" onClick={() => open(instance.id)}>{instance.id}</button><div className="muted">{instance.workflow_def_id}</div></td><td><Badge status={instance.status} /></td><td>{date(instance.created_at_epoch_ms)}</td><td>{instance.completed_task_count} / {instance.total_task_count}</td><td>{date(instance.modified_at_epoch_ms)}</td></tr>)}</tbody></table></div>}
  </>
}
function HumanInput({ api, report, task, complete, busy }: { api: Api; report: Report; task: Task; complete: () => void; busy: boolean }) {
  const [input, setInput] = useState('')
  const [format, setFormat] = useState('text')
  const [sending, setSending] = useState(false)
  const [error, setError] = useState('')
  const [sent, setSent] = useState(false)
  async function submit(event: FormEvent) {
    event.preventDefault()
    setError('')
    try {
      const value: unknown = format === 'json' ? JSON.parse(input) : input
      setSending(true)
      await api.humanInput(report.instance_id, task.task_attempt_id, value)
      setSent(true)
      complete()
    } catch (error) { setError(message(error)) }
    finally { setSending(false) }
  }
  return <form className="input-form" onSubmit={submit}>
    <h3>Human input · {task.task_attempt_id}</h3>
    <p>{typeof task.status === 'object' ? task.status.InputNeeded.input_request : ''}</p>
    {sent ? <p className="notice" role="status">Input submitted. Waiting for updated workflow state…</p> : <>
      <label>Input format<select value={format} onChange={event => setFormat(event.target.value)}><option value="text">Plain text</option><option value="json">JSON</option></select></label>
      <label>Response<textarea required rows={5} value={input} onChange={event => setInput(event.target.value)} placeholder={format === 'json' ? '{"approved": true}' : 'Enter your response'} /></label>
      <button className="btn btn-primary" disabled={sending || busy || !input.trim() || report.status !== 'InputNeeded'}>{sending ? 'Submitting…' : 'Submit input'}</button>
    </>}{error && <p className="notice error" role="alert">{error}</p>}
  </form>
}
function InstanceDetails({ api, id, back }: { api: Api; id: string; back: () => void }) {
  const load = useCallback(async (signal: AbortSignal) => {
    const [report, events] = await Promise.all([api.report(id, signal), api.events(id, signal)])
    return { report, events }
  }, [api, id])
  const state = usePolling(load)
  const [busy, setBusy] = useState(false)
  const [actionError, setActionError] = useState('')
  const [actionMessage, setActionMessage] = useState('')
  const report = state.data?.report
  const events = state.data?.events ?? []
  async function performAction(operation: () => Promise<unknown>, success: string) {
    setBusy(true); setActionError(''); setActionMessage('')
    try { await operation(); setActionMessage(success); state.refresh() }
    catch (error) { setActionError(message(error)); state.refresh() }
    finally { setBusy(false) }
  }
  function action(kind: 'pause' | 'resume') {
    return performAction(() => api.action(id, kind), kind === 'pause' ? 'Workflow paused.' : 'Workflow queued to resume.')
  }
  return <><button className="text-button back" onClick={back}>← Back to instances</button><Refresh {...state} />
    <Feedback error={state.error} loading={!state.data} />
    {report && <>
      <section className="glass-panel detail-summary"><div><div className="eyebrow">{report.workflow_def_id}</div><h2>{report.instance_id}</h2><Badge status={report.status} /></div>
        <div className="actions">{(['Pending', 'Running'] as Status[]).includes(report.status) && <button className="btn btn-secondary" disabled={busy} onClick={() => void action('pause')}>Pause workflow</button>}
          {report.status === 'Paused' && <button className="btn btn-primary" disabled={busy} onClick={() => void action('resume')}>Resume workflow</button>}</div>
      </section>
      {actionError && <p className="notice error" role="alert">{actionError}</p>}{actionMessage && <p className="notice" role="status">{actionMessage}</p>}
      {report.status === 'Paused' && <p className="notice">Running tasks may finish; further task execution waits until you resume.</p>}
      {report.tasks.filter(task => typeof task.status === 'object').map(task => <section className="glass-panel panel" key={task.task_attempt_id}><HumanInput api={api} report={report} task={task} busy={busy} complete={state.refresh} /></section>)}
      <h2 className="section-title">Task attempts</h2><div className="glass-panel table-scroll"><table><thead><tr><th>Attempt</th><th>Status</th><th>Generation</th><th>Satisfaction</th><th>Actions</th></tr></thead><tbody>{report.tasks.map(task => <tr key={task.task_attempt_id}><td>{task.task_attempt_id}</td><td><Badge status={taskStatus(task)} /></td><td>{task.generation_index}</td><td>{task.satisfaction}</td><td>{report.status === 'Failed' && task.status === 'Failed' && <button className="btn btn-secondary" disabled={busy} aria-label={`Restart failed task ${task.task_attempt_id}`} onClick={() => void performAction(() => api.retryTask(id, task.task_attempt_id), `Task ${task.task_attempt_id} queued to restart.`)}>Restart task</button>}</td></tr>)}</tbody></table>{report.tasks.length === 0 && <p className="notice">No task attempts yet.</p>}</div>
      {report.verifier_states && report.verifier_states.length > 0 && <details className="glass-panel panel"><summary>Verifier states</summary><pre>{JSON.stringify(report.verifier_states, null, 2)}</pre></details>}
      <h2 className="section-title">Event log <span className="muted">Newest first · {events.length} events</span></h2>
      <div className="glass-panel event-log">{events.length === 0 && <p className="notice">No events recorded yet.</p>}{events.toReversed().map((record, index) => <details key={events.length - index} className="event"><summary><span className="muted">#{events.length - index} · {date(record.created_time)}</span><span>{record.event.type.replaceAll('_', ' ')}</span></summary><pre>{JSON.stringify(record.event, null, 2)}</pre></details>)}</div>
    </>}
  </>
}
function Settings({ connection, save }: { connection: Connection; save: (connection: Connection) => void }) {
  const [host, setHost] = useState(connection.host)
  const [apiKey, setApiKey] = useState(connection.apiKey)
  const [error, setError] = useState('')
  const [success, setSuccess] = useState('')
  function submit(event: FormEvent) {
    event.preventDefault(); setError(''); setSuccess('')
    try {
      const normalized = normalizeHost(host.trim())
      localStorage.setItem('relayfold-api-host', normalized)
      save({ host: normalized, apiKey: apiKey.trim() })
      setHost(normalized); setSuccess('Connection settings saved. Open Workflows or Instances to connect.')
    } catch (error) { setError(message(error)) }
  }
  return <form className="glass-panel panel settings" onSubmit={submit}><h2>Orchestrator connection</h2>
    <p className="muted">Set the public API address as seen from the frontend server.</p>
    <label>API host<input required value={host} onChange={event => setHost(event.target.value)} placeholder={defaultHost} /></label>
    <p className="muted">Default: {defaultHost}. The host is saved in this browser.</p>
    <label>API key (optional)<input type="password" autoComplete="off" value={apiKey} onChange={event => setApiKey(event.target.value)} /></label>
    <p className="muted">Use a bearer API key when namespace authentication is enabled. The key is kept in memory and cleared on page reload.</p>
    <button className="btn btn-primary">Save settings</button>{error && <p className="notice error" role="alert">{error}</p>}{success && <p className="notice" role="status">{success}</p>}
  </form>
}
function App() {
  const [page, setPage] = useState<'workflows' | 'instances' | 'settings'>('instances')
  const [selectedWorkflow, setSelectedWorkflow] = useState<string | null>(null)
  const [selected, setSelected] = useState<string | null>(null)
  const [connection, setConnection] = useState<Connection>(() => ({ host: savedHost(), apiKey: '' }))
  const api = useMemo(() => createApi(connection), [connection])
  const title = page === 'workflows' ? selectedWorkflow ? 'Workflow definition' : 'Workflows' : page === 'settings' ? 'Settings' : selected ? 'Instance details' : 'Instances'
  return <div className="app-container"><aside className="sidebar"><div className="sidebar-logo">◇ RelayFold</div><div className="nav-section-title">Orchestrator</div><nav className="nav-menu" aria-label="Main navigation">
    {(['workflows', 'instances', 'settings'] as const).map(value => <button key={value} className={`nav-item ${page === value ? 'active' : ''}`} aria-current={page === value ? 'page' : undefined} onClick={() => { setPage(value); setSelectedWorkflow(null); if (value !== 'instances') setSelected(null) }}>{value === 'workflows' ? '▤' : value === 'instances' ? '▷' : '⚙'} {value[0].toUpperCase() + value.slice(1)}</button>)}
    </nav><div className="sidebar-host">Public API<br /><span>{connection.host}</span></div></aside>
    <main className="main-content"><header className="page-header"><div><h1 className="page-title">{title}</h1><p className="page-subtitle">{page === 'workflows' ? 'Discover registered workflow definitions.' : page === 'settings' ? 'Configure your orchestrator connection.' : 'Monitor workflow execution and respond to input requests.'}</p></div></header>
      {page === 'settings' ? <Settings connection={connection} save={setConnection} /> : <div key={connection.host + connection.apiKey}>{page === 'workflows' ? selectedWorkflow ? <WorkflowDetails key={selectedWorkflow} api={api} id={selectedWorkflow} back={() => setSelectedWorkflow(null)} openInstance={id => { setSelected(id); setPage('instances') }} /> : <Workflows api={api} open={setSelectedWorkflow} /> : selected ? <InstanceDetails key={selected} api={api} id={selected} back={() => setSelected(null)} /> : <Instances api={api} open={setSelected} />}</div>}
    </main></div>
}
export default App
