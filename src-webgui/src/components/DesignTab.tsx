import { useEffect, useRef, useState } from 'react'
import { Frame } from 'lucide-react'
import { parseDesign, type DesignDoc } from '../lib/design'
import { fileKey } from '../store/coding'
import { isTabVisible } from '../store/editorGroups'
import { useKoma } from '../store/koma'
import type { Tab } from '../store/types/tabs'
import { showCodingHistory } from './CodingHistory'
import { EditorChrome } from './EditorChrome'

export function DesignTab({ tab }: { tab: Extract<Tab, { kind: 'design' }> }) {
  const key = fileKey(tab.root, tab.path)
  const file = useKoma((s) => s.design.docs[key])
  const updateDesign = useKoma((s) => s.updateDesign)
  const saveDesign = useKoma((s) => s.saveDesign)
  const active = useKoma((s) => s.ui.activeTabId === tab.id && isTabVisible(s.ui, tab.id))
  const pastRef = useRef<DesignDoc[]>([])
  const futureRef = useRef<DesignDoc[]>([])
  const [rev, setRev] = useState(0)

  useEffect(() => {
    pastRef.current = []
    futureRef.current = []
    setRev((value) => value + 1)
  }, [key])

  const undo = () => {
    const prev = pastRef.current.pop()
    if (!prev) return
    const current = useKoma.getState().design.docs[key]?.doc
    if (current) futureRef.current.push(current)
    setRev((value) => value + 1)
    updateDesign(tab.root, tab.path, prev)
  }
  const redo = () => {
    const next = futureRef.current.pop()
    if (!next) return
    const current = useKoma.getState().design.docs[key]?.doc
    if (current) pastRef.current.push(current)
    setRev((value) => value + 1)
    updateDesign(tab.root, tab.path, next)
  }

  const revert = () => {
    const saved = useKoma.getState().design.docs[key]?.savedText
    if (!saved) return
    const parsed = parseDesign(saved)
    if (parsed.error) return
    pastRef.current = []
    futureRef.current = []
    setRev((value) => value + 1)
    updateDesign(tab.root, tab.path, parsed.doc)
  }

  useEffect(() => {
    const onRestore = (event: Event) => {
      const detail = (event as CustomEvent<{ root: string; path: string }>).detail
      if (!detail || detail.root !== tab.root || detail.path !== tab.path) return
      pastRef.current = []
      futureRef.current = []
      setRev((value) => value + 1)
    }
    window.addEventListener('koma-design-restore', onRestore)
    return () => window.removeEventListener('koma-design-restore', onRestore)
  }, [tab.path, tab.root])

  useEffect(() => {
    if (!active) return
    const onKey = (event: KeyboardEvent) => {
      if (!(event.metaKey || event.ctrlKey) || event.shiftKey || event.altKey || event.key.toLowerCase() !== 's') return
      event.preventDefault()
      saveDesign(tab.root, tab.path)
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [active, saveDesign, tab.path, tab.root])

  if (!file) {
    return (
      <div className="flex h-full items-center justify-center text-[12px] text-koma-dim">
        Loading…
      </div>
    )
  }

  const status = file.saving
    ? 'Saving…'
    : file.loading && file.savedText == null
      ? 'Loading…'
      : file.error && !file.dirty
        ? file.error
        : file.dirty
          ? 'Modified'
          : 'Saved'
  const canSave = file.dirty && !file.saving && !file.loading
  const canRevert = file.dirty && !!file.savedText && !file.saving && !file.loading
  void rev

  return (
    <div className="flex h-full min-h-0 min-w-0 flex-col bg-koma-bg text-koma-fg">
      <EditorChrome
        path={tab.path}
        icon={<Frame size={13} className="flex-none text-koma-dim" />}
        status={status}
        canSave={canSave}
        canRevert={canRevert}
        saving={file.saving}
        canUndo={pastRef.current.length > 0}
        canRedo={futureRef.current.length > 0}
        onHistory={() => showCodingHistory(tab.root, tab.path)}
        onUndo={undo}
        onRedo={redo}
        onSave={() => saveDesign(tab.root, tab.path)}
        onRevert={revert}
      />
      {file.error ? <div className="flex-none border-b border-koma-border px-3 py-1 text-[12px] text-koma-error">{file.error}</div> : null}
      <div className="flex min-h-0 flex-1 items-center justify-center text-[12px] text-koma-fg opacity-35">
        {file.loading ? 'Loading…' : 'This design has no frames yet'}
      </div>
    </div>
  )
}
