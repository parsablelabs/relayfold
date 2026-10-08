import test from 'node:test'
import assert from 'node:assert/strict'
import { pendingHumanInputs, shouldPollInstance } from '../src/instanceState.ts'
import { statuses } from '../src/api.ts'
import type { Task } from '../src/api.ts'

function attempt(task: string, generation: number, status: Task['status']): Task {
  return { task_attempt_id: `${task}:${generation}`, task_def_id: task, generation_index: generation, satisfaction: 'Pending', status }
}

test('shows only the latest input request per task regardless of report order', () => {
  const first = attempt('agent', 0, { InputNeeded: { input_request: 'First question' } })
  const second = attempt('agent', 1, { InputNeeded: { input_request: 'Second question' } })
  const other = attempt('other', 0, { InputNeeded: { input_request: 'Independent question' } })
  for (const tasks of [[first, second, other], [second, first, other]]) {
    assert.deepEqual(pendingHumanInputs({ status: 'InputNeeded', tasks }), [second, other])
  }
})

test('hides historical requests when a newer attempt no longer needs input', () => {
  const first = attempt('agent', 0, { InputNeeded: { input_request: 'Answered question' } })
  for (const status of ['Pending', 'Running', 'Completed', 'Failed'] as const) {
    const latest = attempt('agent', 1, status)
    assert.deepEqual(pendingHumanInputs({ status: 'InputNeeded', tasks: [latest, first] }), [])
  }
})

test('shows input requests only while the workflow waits for input', () => {
  const tasks = [attempt('agent', 0, { InputNeeded: { input_request: 'Question' } })]
  for (const status of statuses) {
    assert.deepEqual(pendingHumanInputs({ status, tasks }), status === 'InputNeeded' ? tasks : [])
  }
})

test('polls queued and running instances, stops for all other reported states', () => {
  for (const status of statuses) {
    assert.equal(shouldPollInstance({ status }), ['Pending', 'Running'].includes(status), status)
  }
})

test('retries initial connection errors and allows polling to resume after an action', () => {
  assert.equal(shouldPollInstance(undefined), true)
  for (const status of ['Pending', 'Running', 'InputNeeded', 'Pending', 'Running', 'Completed'] as const) {
    assert.equal(shouldPollInstance({ status }), status === 'Pending' || status === 'Running')
  }
})
