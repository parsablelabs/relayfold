import { useEffect, useState } from 'react'

let diagramId = 0

export default function WorkflowDiagram({ source }: { source: string }) {
  const [svg, setSvg] = useState('')
  const [error, setError] = useState('')
  useEffect(() => {
    let cancelled = false
    const id = `workflow-diagram-${++diagramId}`
    async function render() {
      try {
        const { default: mermaid } = await import('mermaid')
        if (cancelled) return
        mermaid.initialize({ startOnLoad: false, securityLevel: 'strict', theme: 'base', fontFamily: 'SFMono-Regular, Consolas, Liberation Mono, monospace', themeVariables: { fontSize: '12px', darkMode: true, background: '#111a2b', primaryColor: '#19253b', primaryTextColor: '#e2e8f4', primaryBorderColor: '#527ccb', lineColor: '#9aaac4', secondaryColor: '#19253b', tertiaryColor: '#111a2b' }, flowchart: { htmlLabels: false, useMaxWidth: false, nodeSpacing: 30, rankSpacing: 35, padding: 10 } })
        const result = await mermaid.render(id, source)
        if (!cancelled) { setSvg(result.svg); setError('') }
      } catch (error) {
        if (!cancelled) setError(error instanceof Error ? error.message : String(error))
      }
    }
    void render()
    return () => { cancelled = true }
  }, [source])
  if (error) return <p className="notice error" role="alert">Unable to render workflow diagram: {error}</p>
  if (!svg) return <p className="notice" role="status">Rendering diagram…</p>
  return <div className="workflow-diagram" role="img" aria-label="Workflow task dependency diagram" dangerouslySetInnerHTML={{ __html: svg }} />
}
