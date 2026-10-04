import test from 'node:test'
import assert from 'node:assert/strict'
import { createTriggerValidator } from '../src/triggerInput.ts'
import type { Definition } from '../src/workflowGraph.ts'

function definition(schema: boolean | Record<string, unknown>): Definition {
  return { id: 'workflow', tasks: [{ id: 'entry', kind: { function: {} }, input_schemas: [schema] }], data_bindings: [] }
}

test('validates required fields, nested types, enums and extra properties without coercing input', () => {
  const validator = createTriggerValidator(definition({ type: 'object', required: ['user'], additionalProperties: false, properties: { user: { type: 'object', required: ['age', 'role'], properties: { age: { type: 'integer', minimum: 18 }, role: { enum: ['admin', 'reader'] } } } } }))
  const input = { user: { age: 20, role: 'reader' } }
  assert.deepEqual(validator.validate(JSON.stringify(input)), { payload: input, errors: [] })
  assert.match(validator.validate('{}').errors.join('\n'), /entry:.*user.*required/)
  const errors = validator.validate('{"user":{"age":"20","role":"invalid"},"extra":true}').errors.join('\n')
  assert.match(errors, /age.*integer/)
  assert.match(errors, /role.*allowed values/)
  assert.match(errors, /extra.*additional properties/)
})

test('only entry task schemas apply, and all entry task schemas must pass', () => {
  const workflow = definition({ type: 'object', required: ['a'] })
  workflow.tasks.push({ id: 'other-entry', kind: { agent: {} }, input_schemas: [{ type: 'object', required: ['b'] }] })
  workflow.tasks.push({ id: 'downstream', kind: { function: {} }, input_schemas: [{ type: 'integer' }] })
  workflow.data_bindings.push({ source_task_id: 'entry', target_task_id: 'downstream' })
  const validator = createTriggerValidator(workflow)
  assert.deepEqual(validator.schemas.map(value => value.taskId), ['entry', 'other-entry'])
  assert.match(validator.validate('{"a":1}').errors.join('\n'), /other-entry:.*b/)
  assert.deepEqual(validator.validate('{"a":1,"b":2}').errors, [])
})

test('blocks absent trigger input and invalid JSON, while allowing no input without schemas', () => {
  const validator = createTriggerValidator(definition(true))
  assert.match(validator.validate('').errors.join('\n'), /Provide JSON input/)
  assert.match(validator.validate('null').errors.join('\n'), /no input/)
  assert.match(validator.validate('{').errors.join('\n'), /valid JSON/)
  assert.deepEqual(validator.validate('false').errors, [])
  const workflow = definition(true)
  workflow.tasks[0].input_schemas = []
  assert.deepEqual(createTriggerValidator(workflow).validate(''), { payload: null, errors: [] })
})

test('validates standard formats, local references, and explicit schema drafts', () => {
  for (const dialect of [undefined, 'http://json-schema.org/draft-07/schema#', 'https://json-schema.org/draft/2019-09/schema', 'https://json-schema.org/draft/2020-12/schema']) {
    const validator = createTriggerValidator(definition({ ...(dialect ? { $schema: dialect } : {}), type: 'object', properties: { email: { $ref: '#/definitions/email' } }, definitions: { email: { type: 'string', format: 'email' } } }))
    assert.deepEqual(validator.validate('{"email":"person@example.com"}').errors, [])
    assert.match(validator.validate('{"email":"invalid"}').errors.join('\n'), /email/)
  }
})

test('blocks invalid schemas, unresolved references and unsupported multiple entry input slots', () => {
  assert.match(createTriggerValidator(definition({ type: 'invalid-type' })).validate('{}').errors.join('\n'), /cannot validate input schema/)
  assert.match(createTriggerValidator(definition({ $ref: 'https://example.com/missing.json' })).validate('{}').errors.join('\n'), /cannot validate input schema/)
  const workflow = definition(true)
  workflow.tasks[0].input_schemas?.push(true)
  assert.match(createTriggerValidator(workflow).validate('{}').errors.join('\n'), /input slot 2/)
  assert.ok(createTriggerValidator(definition(false)).validate('{}').errors.length > 0)
})
