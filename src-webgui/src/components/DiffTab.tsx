import { useEffect, useRef, type ReactNode } from 'react'
import * as monaco from 'monaco-editor/esm/vs/editor/editor.api'
import { useKoma, type Tab } from '../store/koma'
import { isTabVisible, normalizeGroups } from '../store/editorGroups'
import { BrailleSpinner } from './BrailleSpinner'
import { initMonaco, applyKomaTheme, readMonoFont, langFromPath } from '../lib/monaco-setup'

type DiffTabModel = Extract<Tab, { kind: 'diff' }>

function Centered({ children }: { children: ReactNode }) {
  return (
    <div className="flex h-full w-full items-center justify-center px-6 text-center text-[12px] text-koma-dim">
      {children}
    </div>
  )
}

// Side-by-side Monaco DiffEditor for one File-changed path. Lazy-loaded (this
// whole module + monaco lands in its own async chunk), so nothing here touches
// the main bundle until the first diff tab is opened.
export default function DiffTab({ tab }: { tab: DiffTabModel }) {
  const containerRef = useRef<HTMLDivElement | null>(null)
  const editorRef = useRef<monaco.editor.IStandaloneDiffEditor | null>(null)
  const modelsRef = useRef<{
    original: monaco.editor.ITextModel
    modified: monaco.editor.ITextModel
  } | null>(null)

  const diff = tab.diff
  const showEditor = diff != null && (diff.error == null || diff.error === '') && !diff.binary
  const isActive = useKoma((s) => isTabVisible(normalizeGroups(s.ui), tab.id))

  // Create the diff editor once, when it first becomes showable; dispose on
  // unmount (or when it stops being showable, e.g. a re-request returns an
  // error/binary). NOT re-created on diff-content changes, so re-focus of an
  // already-open tab doesn't flash.
  useEffect(() => {
    const host = containerRef.current
    if (!showEditor || !host) return
    initMonaco()
    const theme = applyKomaTheme('koma-diff')
    const editor = monaco.editor.createDiffEditor(host, {
      renderSideBySide: true,
      readOnly: true,
      originalEditable: false,
      automaticLayout: true,
      minimap: { enabled: false },
      folding: false,
      scrollBeyondLastLine: false,
      fontFamily: readMonoFont(),
      fontSize: 12,
      lineNumbersMinChars: 3,
      theme,
    })
    monaco.editor.setTheme(theme)
    editorRef.current = editor
    return () => {
      editor.dispose()
      editorRef.current = null
    }
  }, [showEditor])

  // display:none inactive panes zero the host; WebKit often skips ResizeObserver
  // on the reveal, so force a layout when this tab becomes the visible one.
  useEffect(() => {
    if (!isActive || !showEditor) return
    const raf = requestAnimationFrame(() => editorRef.current?.layout())
    return () => cancelAnimationFrame(raf)
  }, [isActive, showEditor])

  // Sync the two models to the current diff payload — initial open AND every
  // re-request on re-activate. The editor persists across these; only the models
  // are swapped, so a re-focus updates content without a rebuild/flash. Monaco's
  // createDiffEditor.dispose() does NOT dispose caller-owned models, so we own
  // their lifecycle explicitly (dispose the prior pair after setting the new).
  useEffect(() => {
    if (!showEditor || !diff || !editorRef.current) return
    const lang = langFromPath(tab.path)
    const original = monaco.editor.createModel(diff.original, lang)
    const modified = monaco.editor.createModel(diff.modified, lang)
    editorRef.current.setModel({ original, modified })
    const prev = modelsRef.current
    modelsRef.current = { original, modified }
    if (prev) {
      prev.original.dispose()
      prev.modified.dispose()
    }
  }, [diff, showEditor, tab.path])

  // Drop whatever models remain on final unmount (tab closed / session switch).
  useEffect(
    () => () => {
      const m = modelsRef.current
      if (m) {
        m.original.dispose()
        m.modified.dispose()
        modelsRef.current = null
      }
    },
    [],
  )

  // ---- Body states ----------------------------------------------------------
  if (diff && diff.error != null && diff.error !== '')
    return (
      <div className="relative h-full w-full">
        <Centered>{diff.error}</Centered>
        {tab.loading && (
          <div className="pointer-events-none absolute right-2 top-2 text-koma-dim">
            <BrailleSpinner size={14} className="opacity-70" />
          </div>
        )}
      </div>
    )
  if (diff && diff.binary)
    return (
      <div className="relative h-full w-full">
        <Centered>binary file — no preview</Centered>
        {tab.loading && (
          <div className="pointer-events-none absolute right-2 top-2 text-koma-dim">
            <BrailleSpinner size={14} className="opacity-70" />
          </div>
        )}
      </div>
    )
  if (!diff) {
    return (
      <div className="flex h-full w-full items-center justify-center text-koma-dim">
        <BrailleSpinner size={18} className="opacity-70" />
      </div>
    )
  }
  return (
    <div className="relative h-full w-full">
      <div ref={containerRef} className="absolute inset-0" />
      {/* Non-git dirs diff against the session's first-touch pre-image ("virtual
          git") — badge the origin so nobody mistakes it for a git diff. */}
      {diff.origin === 'baseline' && (
        <div className="pointer-events-none absolute bottom-2 right-4 rounded border border-koma-border bg-koma-panel/90 px-1.5 py-0.5 font-mono text-[10px] text-koma-dim">
          session baseline
        </div>
      )}
      {tab.loading && (
        <div className="pointer-events-none absolute right-2 top-2 text-koma-dim">
          <BrailleSpinner size={14} className="opacity-70" />
        </div>
      )}
    </div>
  )
}
