import Ajv from 'ajv'
import Ajv2019 from 'ajv/dist/2019.js'
import Ajv2020 from 'ajv/dist/2020.js'
import addFormats from 'ajv-formats'
import type { ValidateFunction } from 'ajv'
import type { Definition } from './workflowGraph.ts'

export function createTriggerValidator(definition: Definition) {
  const inbound = new Set(definition.data_bindings.map(binding => binding.target_task_id))
  const schemas = definition.tasks.filter(task => !inbound.has(task.id)).flatMap(task =>
    (task.input_schemas ?? []).map((schema, index) => ({ taskId: task.id, index, schema })))
  const schemaErrors: string[] = []
  const validators: { taskId: string; validate: ValidateFunction }[] = []
  for (const { taskId, index, schema } of schemas) {
    if (index > 0) {
      schemaErrors.push(`${taskId}: requires input slot ${index + 1}, but workflow trigger input supplies only one slot.`)
      continue
    }
    try {
      const dialect = typeof schema === 'object' ? schema.$schema : undefined
      const options = { allErrors: true, strict: false }
      const ajv = typeof dialect === 'string' && dialect.includes('draft-07') ? new Ajv(options)
        : typeof dialect === 'string' && dialect.includes('2019-09') ? new Ajv2019(options) : new Ajv2020(options)
      addFormats(ajv)
      if (typeof schema === 'object' && schema.$async) throw new Error('Asynchronous schemas cannot be validated in this form.')
      const validate = ajv.compile(schema)
      validators.push({ taskId, validate })
    } catch (error) {
      schemaErrors.push(`${taskId}: cannot validate input schema: ${error instanceof Error ? error.message : String(error)}`)
    }
  }
  return {
    schemas,
    validate(text: string): { payload: unknown; errors: string[] } {
      let payload: unknown = null
      const errors = [...schemaErrors]
      if (text.trim()) {
        try { payload = JSON.parse(text) }
        catch { return { payload: null, errors: [...errors, 'Trigger input must be valid JSON.'] } }
      }
      // The trigger API treats JSON null as absent input, even for nullable schemas.
      if (payload === null && schemas.length) {
        return { payload, errors: [...errors, 'Provide JSON input for the entry tasks. Empty input and null are treated as no input.'] }
      }
      for (const { taskId, validate } of validators) {
        if (validate(payload)) continue
        for (const error of validate.errors ?? []) {
          const field = error.params.missingProperty ?? error.params.additionalProperty
          const path = `${error.instancePath || '/'}${field ? ` (${field})` : ''}`
          errors.push(`${taskId}: ${path} ${error.message}`)
        }
      }
      return { payload, errors }
    },
  }
}
