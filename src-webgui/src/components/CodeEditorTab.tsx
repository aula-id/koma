import { useEffect, useMemo, useRef } from 'react'
import * as monaco from 'monaco-editor/esm/vs/editor/editor.api'
import { Code2, RotateCcw, Save } from 'lucide-react'
import { initMonaco, applyKomaTheme, readMonoFont, langFromPath } from '../lib/monaco-setup'
import { useKoma, type Tab } from '../store/koma'
import { fileKey } from '../store/coding'
import { BrailleSpinner } from './BrailleSpinner'

type CodingTab = Extract<Tab, { kind: 'codingFile' }>

const AUTOSAVE_MS = 750

export default function CodeEditorTab({ tab }: { tab: CodingTab }) {
  const containerRef = useRef<HTMLDivElement | null>(null)
  const editorRef = useRef<monaco.editor.IStandaloneCodeEditor | null>(null)
  const modelRef = useRef<monaco.editor.ITextModel | null>(null)
  // Suppress markDirty while applying host content programmatically.
  const applyingRef = useRef(false)
  const autosaveTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)

  const fileState = useKoma((s) => s.coding.files[fileKey(tab.root, tab.path)] ?? null)
  const codingAutosave = useKoma((s) => !!s.settingsValues?.codingAutosave)
  const saveCodingFile = useKoma((s) => s.saveCodingFile)
  const revertCodingFile = useKoma((s) => s.revertCodingFile)

  const canEdit = !!(
    fileState &&
    fileState.content != null &&
    !fileState.binary &&
    !fileState.tooLarge &&
    !fileState.error &&
    !fileState.conflict
  )
  const canSave = !!(canEdit && fileState?.dirty && !fileState.saving)
  const canRevert = !!(fileState && (fileState.dirty || fileState.conflict) && !fileState.saving)

  const status = useMemo(() => {
    if (!fileState) return 'Loading…'
    if (fileState.loading && fileState.content == null) return 'Loading…'
    if (fileState.saving) return 'Saving…'
    if (fileState.conflict) return 'Conflict — reload required'
    if (fileState.error) return fileState.error
    if (fileState.binary) return 'Binary'
    if (fileState.tooLarge) return 'Too large'
    if (fileState.dirty) return codingAutosave ? 'Modified · autosave on' : 'Modified'
    return 'Saved'
  }, [fileState, codingAutosave])

  // Safe debounced autosave: only when enabled, dirty, editable, not already
  // saving/conflicting, and content differs from last save.
  useEffect(() => {
    if (autosaveTimerRef.current) {
      clearTimeout(autosaveTimerRef.current)
      autosaveTimerRef.current = null
    }
    if (!codingAutosave) return
    if (!fileState?.dirty) return
    if (fileState.content == null) return
    if (fileState.saving || fileState.conflict || fileState.binary || fileState.tooLarge || fileState.error) {
      return
    }
    if (fileState.content === (fileState.savedContent ?? '')) return

    autosaveTimerRef.current = setTimeout(() => {
      autosaveTimerRef.current = null
      const cur = useKoma.getState().coding.files[fileKey(tab.root, tab.path)]
      if (!cur?.dirty || cur.content == null || cur.saving || cur.conflict) return
      if (cur.binary || cur.tooLarge || cur.error) return
      if (cur.content === (cur.savedContent ?? '')) return
      if (!useKoma.getState().settingsValues?.codingAutosave) return
      useKoma.getState().saveCodingFile(tab.root, tab.path)
    }, AUTOSAVE_MS)

    return () => {
      if (autosaveTimerRef.current) {
        clearTimeout(autosaveTimerRef.current)
        autosaveTimerRef.current = null
      }
    }
  }, [
    codingAutosave,
    fileState?.dirty,
    fileState?.content,
    fileState?.savedContent,
    fileState?.saving,
    fileState?.conflict,
    fileState?.binary,
    fileState?.tooLarge,
    fileState?.error,
    tab.root,
    tab.path,
  ])

  // Init Monaco + editor once per tab identity.
  useEffect(() => {
    const host = containerRef.current
    if (!host) return
    initMonaco()
    const theme = applyKomaTheme()
    const editor = monaco.editor.create(host, {
      value: '',
      language: langFromPath(tab.path),
      readOnly: true,
      automaticLayout: true,
      minimap: { enabled: false },
      scrollBeyondLastLine: false,
      fontFamily: readMonoFont(),
      fontSize: 12,
      lineNumbersMinChars: 3,
      theme,
      wordWrap: 'on',
    })
    monaco.editor.setTheme(theme)
    editorRef.current = editor

    // Ctrl/Cmd+S → save
    editor.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyCode.KeyS, () => {
      useKoma.getState().saveCodingFile(tab.root, tab.path)
    })

    const sub = editor.onDidChangeModelContent(() => {
      if (applyingRef.current) return
      const model = editor.getModel()
      if (!model) return
      useKoma.getState().updateCodingContent(tab.root, tab.path, model.getValue())
    })

    return () => {
      sub.dispose()
      const model = modelRef.current
      editor.dispose()
      if (model) model.dispose()
      editorRef.current = null
      modelRef.current = null
    }
  }, [tab.root, tab.path])

  // Sync content when file state changes from the host.
  useEffect(() => {
    if (!fileState || !editorRef.current) return
    const editor = editorRef.current

    if (fileState.content === null) {
      editor.updateOptions({ readOnly: true })
      return
    }

    const lang = langFromPath(tab.path)
    const next = fileState.content
    applyingRef.current = true
    try {
      if (modelRef.current) {
        if (modelRef.current.getValue() !== next) {
          const pos = editor.getPosition()
          modelRef.current.setValue(next)
          if (pos) editor.setPosition(pos)
        }
      } else {
        const model = monaco.editor.createModel(next, lang)
        editor.setModel(model)
        modelRef.current = model
      }
    } finally {
      // Defer so Monaco's own content-change event from setValue is ignored.
      queueMicrotask(() => {
        applyingRef.current = false
      })
    }

    editor.updateOptions({
      readOnly: fileState.binary || fileState.tooLarge || !!fileState.error || fileState.conflict,
    })
  }, [
    fileState?.content,
    fileState?.loading,
    fileState?.binary,
    fileState?.tooLarge,
    fileState?.error,
    fileState?.conflict,
    fileState?.fingerprint,
    tab.path,
  ])

  if (fileState?.conflict) {
    return (
      <div className="flex h-full w-full flex-col">
        <EditorChrome
          path={tab.path}
          status={status}
          canSave={false}
          canRevert={canRevert}
          saving={!!fileState.saving}
          onSave={() => saveCodingFile(tab.root, tab.path)}
          onRevert={() => revertCodingFile(tab.root, tab.path)}
        />
        <div className="flex min-h-0 flex-1 flex-col items-center justify-center gap-2 px-6 text-center text-[12px] text-koma-dim">
          <div>File changed on disk — save was rejected.</div>
          <button
            type="button"
            onClick={() => revertCodingFile(tab.root, tab.path)}
            className="text-koma-fg underline hover:opacity-80"
          >
            Reload from disk
          </button>
        </div>
      </div>
    )
  }
  if (fileState?.binary) {
    return (
      <div className="flex h-full w-full flex-col">
        <EditorChrome
          path={tab.path}
          status={status}
          canSave={false}
          canRevert={false}
          saving={false}
          onSave={() => {}}
          onRevert={() => {}}
        />
        <div className="flex min-h-0 flex-1 items-center justify-center px-6 text-center text-[12px] text-koma-dim">
          Binary file — no preview
        </div>
      </div>
    )
  }
  if (fileState?.tooLarge) {
    return (
      <div className="flex h-full w-full flex-col">
        <EditorChrome
          path={tab.path}
          status={status}
          canSave={false}
          canRevert={false}
          saving={false}
          onSave={() => {}}
          onRevert={() => {}}
        />
        <div className="flex min-h-0 flex-1 items-center justify-center px-6 text-center text-[12px] text-koma-dim">
          File too large to edit
        </div>
      </div>
    )
  }
  if (fileState?.error && fileState.content === null) {
    return (
      <div className="flex h-full w-full flex-col">
        <EditorChrome
          path={tab.path}
          status={status}
          canSave={false}
          canRevert={false}
          saving={false}
          onSave={() => {}}
          onRevert={() => {}}
        />
        <div className="flex min-h-0 flex-1 items-center justify-center px-6 text-center text-[12px] text-koma-dim">
          {fileState.error}
        </div>
      </div>
    )
  }

  return (
    <div className="flex h-full w-full flex-col">
      <EditorChrome
        path={tab.path}
        status={status}
        canSave={canSave}
        canRevert={canRevert}
        saving={!!fileState?.saving}
        onSave={() => saveCodingFile(tab.root, tab.path)}
        onRevert={() => revertCodingFile(tab.root, tab.path)}
      />
      <div className="relative min-h-0 flex-1">
        <div ref={containerRef} className="absolute inset-0" />
        {fileState?.loading && (
          <div className="pointer-events-none absolute right-2 top-2 text-koma-dim">
            <BrailleSpinner size={14} className="opacity-70" />
          </div>
        )}
      </div>
    </div>
  )
}

function EditorChrome({
  path,
  status,
  canSave,
  canRevert,
  saving,
  onSave,
  onRevert,
}: {
  path: string
  status: string
  canSave: boolean
  canRevert: boolean
  saving: boolean
  onSave: () => void
  onRevert: () => void
}) {
  const segments = path.split('/').filter(Boolean)

  return (
    <div className="flex flex-none items-center gap-2 border-b border-koma-border bg-koma-panel2 px-3 py-1.5">
      <Code2 size={14} className="flex-none text-koma-fg opacity-70" />
      <nav aria-label="File path" className="flex min-w-0 flex-1 items-center truncate font-mono text-[12px] text-koma-fg" title={path}>
        {segments.map((segment, index) => (
          <span key={`${segment}-${index}`} className="flex min-w-0 items-center">
            {index > 0 && <span className="mx-1 flex-none text-koma-dim">/</span>}
            <span className="truncate">{segment}</span>
          </span>
        ))}
      </nav>
      <span className="flex-none text-[11px] text-koma-dim">{status}</span>
      <button
        type="button"
        onClick={onRevert}
        disabled={!canRevert}
        title="Revert"
        aria-label="Revert"
        className="flex h-6 items-center gap-1 rounded px-1.5 text-[11px] text-koma-fg opacity-70 hover:bg-koma-hover hover:opacity-100 disabled:cursor-default disabled:opacity-30"
      >
        <RotateCcw size={12} />
        Revert
      </button>
      <button
        type="button"
        onClick={onSave}
        disabled={!canSave}
        title="Save"
        aria-label="Save"
        className="flex h-6 items-center gap-1 rounded bg-koma-accent/15 px-1.5 text-[11px] font-semibold text-koma-accent hover:bg-koma-accent/25 disabled:cursor-default disabled:bg-transparent disabled:opacity-30"
      >
        {saving ? <BrailleSpinner size={12} /> : <Save size={12} />}
        Save
      </button>
    </div>
  )
}
