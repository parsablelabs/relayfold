import test from 'node:test'
import assert from 'node:assert/strict'
import { pageUrl, parseRoute } from '../src/routes.ts'

test('opens each list and detail view from its URL', () => {
  for (const page of ['functions', 'workflows', 'instances'] as const) {
    assert.deepEqual(parseRoute(`/${page}`), { page, id: undefined, definitionTab: 'browse', instanceTab: 'status' })
    assert.equal(parseRoute(`/${page}/123`)?.id, '123')
    assert.equal(parseRoute(`/${page}/123`)?.page, page)
  }
  assert.equal(parseRoute('/settings')?.page, 'settings')
  assert.equal(parseRoute('/')?.page, 'instances')
})

test('round trips IDs containing spaces, slashes, percent signs and URL delimiters', () => {
  for (const id of ['format.hello', '123', 'folder/function', 'what? #100%', '任务']) {
    const url = pageUrl('functions', id)
    assert.equal(parseRoute(url)?.id, id)
    assert.equal(parseRoute(url)?.definitionTab, 'browse')
  }
})

test('restores registration and events tabs from URLs without treating IDs as tab names', () => {
  assert.equal(parseRoute('/functions?tab=register')?.definitionTab, 'register')
  assert.equal(parseRoute('/workflows?tab=register')?.definitionTab, 'register')
  assert.equal(parseRoute('/instances/run?tab=events')?.instanceTab, 'events')
  assert.equal(parseRoute('/functions/register')?.id, 'register')
  assert.equal(parseRoute('/functions/123?tab=register')?.definitionTab, 'browse')
  assert.equal(parseRoute('/instances/run?tab=unknown')?.instanceTab, 'status')
})

test('accepts trailing slashes and rejects unknown or malformed paths', () => {
  assert.equal(parseRoute('/workflows/')?.page, 'workflows')
  assert.equal(parseRoute('/instances/run/')?.id, 'run')
  for (const path of ['/unknown', '/settings/123', '/functions/123/extra', '/functions/%ZZ', '/functions//']) {
    assert.equal(parseRoute(path), undefined, path)
  }
})
