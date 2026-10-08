import { parse } from 'yaml'
import type { Task } from './api'

type DefinitionTask = {
  id: string
  kind: Record<string, unknown>
  input_schemas?: (boolean | Record<string, unknown>)[]
  control?: { verifier?: { max_iterations: number; rerun_from_task_id?: string } }
}
export type Definition = {
  id: string
  description?: string
  example_input?: unknown
  tasks: DefinitionTask[]
  data_bindings: { source_task_id: string; target_task_id: string }[]
}

// Use generated node IDs and encode label punctuation so YAML cannot inject Mermaid syntax.
function label(text: string): string {
  return Array.from(text, character => /[a-zA-Z0-9 _.-]/.test(character)
    ? character : `#${character.codePointAt(0)};`).join('')
}

export function diagramFromYaml(yaml: string, attempts?: Task[]): { definition: Definition; source: string } {
  const definition: Definition = parse(yaml)
  if (!definition || typeof definition.id !== 'string' || !Array.isArray(definition.tasks) || !Array.isArray(definition.data_bindings)) {
    throw new Error('Workflow YAML must contain an id, tasks, and data_bindings.')
  }
  const nodes = new Map<string, string>()
  const lines = ['flowchart LR']
  const latest = new Map<string, Task>()
  attempts?.forEach(attempt => {
    const previous = latest.get(attempt.task_def_id)
    if (!previous || attempt.generation_index > previous.generation_index) latest.set(attempt.task_def_id, attempt)
  })
  definition.tasks.forEach((task, index) => {
    if (!task || typeof task.id !== 'string' || !task.kind || typeof task.kind !== 'object' || Object.keys(task.kind).length !== 1) {
      throw new Error('Each task must have an id and a single task kind.')
    }
    if (nodes.has(task.id)) throw new Error(`Duplicate task ID: ${task.id}`)
    const node = `task${index}`
    nodes.set(task.id, node)
    const kind = Object.keys(task.kind)[0]
    const verifier = task.control?.verifier
    const attempt = latest.get(task.id)
    const status = attempt ? typeof attempt.status === 'string' ? attempt.status : 'InputNeeded' : 'Pending'
    const statusLabel = status === 'Completed' ? 'Success' : status === 'InputNeeded' ? 'Input needed' : status
    const text = label(`${task.id} · ${kind}${verifier ? ` · verifier (max ${verifier.max_iterations} iterations)` : ''}${attempts ? ` · ${statusLabel}${attempt ? ` · generation ${attempt.generation_index}` : ''}` : ''}`)
    lines.push(verifier ? `  ${node}{"${text}"}` : `  ${node}["${text}"]`)
    if (attempts) lines.push(`  class ${node} ${status.toLowerCase()}`)
  })
  function nodeFor(id: string): string {
    const node = nodes.get(id)
    if (!node) throw new Error(`Connection references unknown task: ${id}`)
    return node
  }
  definition.data_bindings.forEach(binding => {
    lines.push(`  ${nodeFor(binding.source_task_id)} --> ${nodeFor(binding.target_task_id)}`)
  })
  definition.tasks.forEach(task => {
    const target = task.control?.verifier?.rerun_from_task_id
    if (target) lines.push(`  ${nodeFor(task.id)} -. rerun .-> ${nodeFor(target)}`)
  })
  if (attempts) lines.push(
    '  classDef pending fill:#19253b,stroke:#9aaac4,color:#e2e8f4',
    '  classDef running fill:#163139,stroke:#83d6e5,color:#e2e8f4,stroke-width:3px',
    '  classDef completed fill:#192e52,stroke:#78a5ff,color:#e2e8f4',
    '  classDef failed fill:#3b2020,stroke:#ffabab,color:#e2e8f4',
    '  classDef inputneeded fill:#39301c,stroke:#ecc77e,color:#e2e8f4,stroke-width:3px',
  )
  return { definition, source: lines.join('\n') }
}
