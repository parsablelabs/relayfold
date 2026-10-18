import type { EventRecord } from './api.ts'

export function eventSummary(event: EventRecord['event']) {
  const taskName = typeof event.task_attempt_id === 'string'
    ? event.task_attempt_id.replace(/\[\d+\]$/, '')
    : typeof event.verifier_task_id === 'string' ? event.verifier_task_id : undefined
  const isStatusChange = ['workflow_status_changed', 'task_status_changed', 'verifier_state_status_changed'].includes(event.type)
  const status = isStatusChange && typeof event.status === 'string' ? event.status : undefined
  const tone = status === 'Failed' || status === 'failed' || status === 'exhausted_failed' ? 'failure'
    : status === 'Completed' || status === 'accepted' || status === 'exhausted_accepted' ? 'success' : undefined
  return {
    taskName,
    label: event.type.replaceAll('_', ' ') + (status ? ` · ${status.replaceAll('_', ' ')}` : ''),
    tone,
  }
}
