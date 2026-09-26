import { useEffect, useRef } from 'react'
import * as monaco from 'monaco-editor/esm/vs/editor/editor.api'
import { useKoma } from '../store/koma'
import { initMonaco, applyKomaTheme, readMonoFont, langFromPath } from '../lib/monaco-setup'
import { isTabVisible, normalizeGroups } from '../store/editorGroups'

// The same Monaco setup, font and palette as the existing coding/diff tabs.
export function CodePane({ value, path = '', readOnly = true, onChange, revealLine, tabId }: {
  value: string; path?: string; readOnly?: boolean; onChange?: (value: string) => void; revealLine?: number; tabId: string
}) {
  const host = useRef<HTMLDivElement>(null)
  const editor = useRef<monaco.editor.IStandaloneCodeEditor | null>(null)
  const callback = useRef(onChange); callback.current = onChange
  const changing = useRef(false)
  const visible = useKoma(s => isTabVisible(normalizeGroups(s.ui), tabId))
  useEffect(() => {
    if (!host.current) return
    initMonaco()
    const model = monaco.editor.createModel('', langFromPath(path))
    const instance = monaco.editor.create(host.current, {
      model, readOnly, automaticLayout: true, minimap: { enabled: false }, scrollBeyondLastLine: false,
      fontFamily: readMonoFont(), fontSize: 12, theme: applyKomaTheme('koma-workbench'), lineNumbersMinChars: 3,
    })
    editor.current = instance
    const subscription = instance.onDidChangeModelContent(() => { if (!changing.current) callback.current?.(instance.getValue()) })
    return () => { subscription.dispose(); instance.dispose(); model.dispose(); editor.current = null }
  }, [path])
  useEffect(() => { editor.current?.updateOptions({ readOnly }) }, [readOnly])
  useEffect(() => {
    const instance = editor.current
    if (instance && instance.getValue() !== value) { changing.current = true; instance.setValue(value); changing.current = false }
  }, [value, path])
  useEffect(() => { if (revealLine) editor.current?.revealLineInCenter(revealLine) }, [revealLine])
  useEffect(() => { if (visible) { const frame = requestAnimationFrame(() => editor.current?.layout()); return () => cancelAnimationFrame(frame) } }, [visible])
  return <div ref={host} className="h-full min-h-0 w-full" />
}
