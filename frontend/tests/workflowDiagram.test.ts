import test from 'node:test'
import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import { diagramFromYaml } from '../src/workflowGraph.ts'

const task = (id: string) => ({ id, kind: { function: { code: 'return {}' } } })

test('uses data bindings for branches and joins without inventing edges for unconnected tasks', () => {
  const yaml = JSON.stringify({ id: 'branches', tasks: ['a', 'b', 'c', 'd', 'independent'].map(task), data_bindings: [
    { source_task_id: 'a', target_task_id: 'b' }, { source_task_id: 'a', target_task_id: 'c' },
    { source_task_id: 'b', target_task_id: 'd' }, { source_task_id: 'c', target_task_id: 'd' },
  ] })
  const { source } = diagramFromYaml(yaml)
  assert.deepEqual(source.split('\n').filter(line => line.includes('-->')), ['  task0 --> task1', '  task0 --> task2', '  task1 --> task3', '  task2 --> task3'])
  assert.match(source, /task4\["independent/)
})

test('marks verifiers and draws their explicitly configured rerun target', () => {
  const { source } = diagramFromYaml(JSON.stringify({ id: 'verify', tasks: [task('produce'), { ...task('check'), control: { verifier: { max_iterations: 3, rerun_from_task_id: 'produce' } } }], data_bindings: [{ source_task_id: 'produce', target_task_id: 'check' }] }))
  assert.match(source, /task1\{"check/)
  assert.match(source, /max 3 iterations/)
  assert.match(source, /task1 -\. rerun \.-> task0/)
})

test('keeps task labels from injecting Mermaid directives or markup', () => {
  const id = 'end"\nclick task0 "https://example.com"<script>&#34;'
  const { source } = diagramFromYaml(JSON.stringify({ id: 'escape', tasks: [task(id)], data_bindings: [] }))
  assert.equal(source.split('\n').length, 2)
  assert.ok(!source.includes('<script>'))
  assert.ok(!source.includes('"https://'))
  assert.match(source, /#34;/)
})

test('handles empty workflows and rejects invalid or ambiguous task connections', () => {
  assert.equal(diagramFromYaml('id: empty\ntasks: []\ndata_bindings: []').source, 'flowchart LR')
  assert.throws(() => diagramFromYaml('tasks: ['))
  assert.throws(() => diagramFromYaml('id: bad'), /must contain/)
  assert.throws(() => diagramFromYaml(JSON.stringify({ id: 'bad', tasks: [task('a'), task('a')], data_bindings: [] })), /Duplicate/)
  assert.throws(() => diagramFromYaml(JSON.stringify({ id: 'bad', tasks: [task('a')], data_bindings: [{ source_task_id: 'a', target_task_id: 'missing' }] })), /unknown task/)
})

test('builds diagrams from repository YAML examples', async () => {
  for (const name of ['example_workflow.yaml', 'example_human_input_workflow.yaml', 'example_github_issue_pr_workflow.yaml']) {
    const yaml = await readFile(new URL(`../../examples/${name}`, import.meta.url), 'utf8')
    const { definition, source } = diagramFromYaml(yaml)
    assert.ok(definition.tasks.length > 0)
    assert.equal(source.split('\n').filter(line => /^ {2}task\d+[[{]/.test(line)).length, definition.tasks.length)
    assert.equal(source.split('\n').filter(line => line.includes('-->')).length, definition.data_bindings.length)
  }
})
