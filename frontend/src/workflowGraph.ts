import { parse } from 'yaml'

type DefinitionTask = {
  id: string
  kind: Record<string, unknown>
  control?: { verifier?: { max_iterations: number; rerun_from_task_id?: string } }
}
type Definition = {
  id: string
  description?: string
  tasks: DefinitionTask[]
  data_bindings: { source_task_id: string; target_task_id: string }[]
}

// Use generated node IDs and encode label punctuation so YAML cannot inject Mermaid syntax.
function label(text: string): string {
  return Array.from(text, character => /[a-zA-Z0-9 _.-]/.test(character)
    ? character : `#${character.codePointAt(0)};`).join('')
}

export function diagramFromYaml(yaml: string): { definition: Definition; source: string } {
  const definition: Definition = parse(yaml)
  if (!definition || typeof definition.id !== 'string' || !Array.isArray(definition.tasks) || !Array.isArray(definition.data_bindings)) {
    throw new Error('Workflow YAML must contain an id, tasks, and data_bindings.')
  }
  const nodes = new Map<string, string>()
  const lines = ['flowchart LR']
  definition.tasks.forEach((task, index) => {
    if (!task || typeof task.id !== 'string' || !task.kind || typeof task.kind !== 'object' || Object.keys(task.kind).length !== 1) {
      throw new Error('Each task must have an id and a single task kind.')
    }
    if (nodes.has(task.id)) throw new Error(`Duplicate task ID: ${task.id}`)
    const node = `task${index}`
    nodes.set(task.id, node)
    const kind = Object.keys(task.kind)[0]
    const verifier = task.control?.verifier
    const text = label(`${task.id} · ${kind}${verifier ? ` · verifier (max ${verifier.max_iterations} iterations)` : ''}`)
    lines.push(verifier ? `  ${node}{"${text}"}` : `  ${node}["${text}"]`)
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
  return { definition, source: lines.join('\n') }
}
