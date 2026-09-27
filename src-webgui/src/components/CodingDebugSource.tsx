import { useEffect, useRef, useState } from 'react'
import { X } from 'lucide-react'
export function CodingDebugSource({ source, close }: { source: { content: string; name: string; line: number }; close: () => void }) {
  const container = useRef<HTMLDivElement>(null)
  const [error, setError] = useState('')
  useEffect(() => {
    if (!container.current) return
    setError('')
    const element = container.current; let canceled = false, dispose: (() => void) | undefined
    void Promise.all([import('monaco-editor/esm/vs/editor/editor.api'), import('../lib/monaco-setup')]).then(([monaco, setup]) => {
      if (canceled) return
      setup.initMonaco(); monaco.editor.setTheme(setup.applyKomaTheme())
      const model = monaco.editor.createModel(source.content, setup.langFromPath(source.name))
      const editor = monaco.editor.create(element, { model, readOnly: true, automaticLayout: true, minimap: { enabled: false }, fontFamily: setup.readMonoFont(), fontSize: 12 })
      editor.revealLineInCenter(Math.max(1, source.line)); editor.setPosition({ lineNumber: Math.max(1, source.line), column: 1 })
      dispose = () => { editor.dispose(); model.dispose() }
    }).catch(e => { if (!canceled) setError(String(e)) })
    return () => { canceled = true; dispose?.() }
  }, [source])
  return <div role="dialog" aria-label="Debugger source" className="absolute inset-0 z-10 flex flex-col bg-koma-panel" onKeyDown={e => { if (e.key === 'Escape') { e.stopPropagation(); close() } }}><div className="flex items-center gap-2 border-b border-koma-border px-2 py-1"><span className="min-w-0 flex-1 truncate">{source.name} · read only</span><button autoFocus aria-label="Close source" onClick={close} className="rounded p-1 text-koma-dim hover:bg-koma-hover"><X size={12} /></button></div>{error && <p role="alert" className="p-2 text-koma-error">{error}</p>}<div ref={container} className="min-h-0 flex-1" /></div>
}
