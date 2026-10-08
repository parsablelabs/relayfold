import test from 'node:test'
import assert from 'node:assert/strict'
import { shouldPollInstance } from '../src/instanceState.ts'
import { statuses } from '../src/api.ts'

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
