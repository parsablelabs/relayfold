import test from 'node:test'
import assert from 'node:assert/strict'
import { eventSummary } from '../src/eventSummary.ts'

test('shows task names without generation suffixes and preserves task punctuation', () => {
  assert.deepEqual(eventSummary({ type: 'task_status_changed', task_attempt_id: 'apply_labels[3]', status: 'Completed' }), {
    taskName: 'apply_labels', label: 'task status changed · Completed', tone: 'success',
  })
  assert.equal(eventSummary({ type: 'task_output_recorded', task_attempt_id: 'agent[part]-name[2]' }).taskName, 'agent[part]-name')
  assert.equal(eventSummary({ type: 'verifier_feedback_recorded', verifier_task_id: 'review' }).taskName, 'review')
})

test('colors failed and successful status changes including verifier outcomes', () => {
  for (const type of ['workflow_status_changed', 'task_status_changed']) {
    assert.equal(eventSummary({ type, status: 'Failed' }).tone, 'failure')
    assert.equal(eventSummary({ type, status: 'Completed' }).tone, 'success')
    for (const status of ['Running', 'Pending', 'InputNeeded', 'Paused', 'Skipped']) {
      assert.equal(eventSummary({ type, status }).tone, undefined)
    }
  }
  for (const status of ['failed', 'exhausted_failed']) {
    assert.equal(eventSummary({ type: 'verifier_state_status_changed', status }).tone, 'failure')
  }
  for (const status of ['accepted', 'exhausted_accepted']) {
    assert.equal(eventSummary({ type: 'verifier_state_status_changed', status }).tone, 'success')
  }
})

test('does not attribute global events to a task or classify arbitrary output as status', () => {
  assert.deepEqual(eventSummary({ type: 'workflow_status_changed', status: 'Failed' }), {
    taskName: undefined, label: 'workflow status changed · Failed', tone: 'failure',
  })
  assert.equal(eventSummary({ type: 'task_output_recorded', status: 'Failed', output_data: { status: 'Failed' } }).tone, undefined)
})
