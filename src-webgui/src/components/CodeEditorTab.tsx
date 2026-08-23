import { useEffect, useMemo, useRef, useState } from 'react'
import * as monaco from 'monaco-editor/esm/vs/editor/editor.api'
import { Code2, Download, RotateCcw, Save, X } from 'lucide-react'
import { initMonaco, applyKomaTheme, readMonoFont, langFromPath } from '../lib/monaco-setup'
import {
  ensureLspProviders,
  languageIdForPath,
  stampModelPath,
  applyDiagnosticsToMonaco,
  pathToUri,
  consumeReveal,
  monacoUriFromPath,
  setGoToDefinitionHandler,
  warmCodeLensCache,
} from '../lib/monaco-lsp'
import { codingAskInChatPayload } from '../lib/codingRef'
import { viewerKindForPath, type ViewerKind } from '../lib/viewerKind'
import { useKoma, type Tab } from '../store/koma'
import { fileKey } from '../store/coding'
import { BrailleSpinner } from './BrailleSpinner'
import { CodingFileViewer } from './CodingFileViewer'

type CodingTab = Extract<Tab, { kind: 'codingFile' }>

const AUTOSAVE_MS = 750
const LSP_CHANGE_MS = 300

export default function CodeEditorTab({ tab }: { tab: CodingTab }) {
  const containerRef = useRef<HTMLDivElement | null>(null)
  const editorRef = useRef<monaco.editor.IStandaloneCodeEditor | null>(null)
  const modelRef = useRef<monaco.editor.ITextModel | null>(null)
  const applyingRef = useRef(false)
  const autosaveTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const lspChangeTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const lspOpenedRef = useRef(false)

  const fileState = useKoma((s) => s.coding.files[fileKey(tab.root, tab.path)] ?? null)
  const codingAutosave = useKoma((s) => !!s.settingsValues?.codingAutosave)
  const saveCodingFile = useKoma((s) => s.saveCodingFile)
  const revertCodingFile = useKoma((s) => s.revertCodingFile)
  const lspServers = useKoma((s) => s.lspServers)
  const lspProgress = useKoma((s) => s.lspProgress)
  const lspDiagnostics = useKoma((s) => s.lspDiagnostics)
  const refreshLsp = useKoma((s) => s.refreshLsp)
  const lspInstall = useKoma((s) => s.lspInstall)
  const req = useKoma((s) => s.req)
  const [bannerDismissed, setBannerDismissed] = useState<Record<string, boolean>>({})

  useEffect(() => {
    if (lspServers.length === 0) refreshLsp()
  }, [lspServers.length, refreshLsp])

  useEffect(() => {
    setGoToDefinitionHandler((uri, line, character) => {
      useKoma.getState().openDiagnostic(uri, line, character)
    })
    ensureLspProviders(
      (body) => useKoma.getState().req(body as never),
      () => (useKoma.getState().settingsValues?.workdir ?? []).filter(Boolean),
      (uri, line, character) => {
        useKoma.getState().openDiagnostic(uri, line, character)
      },
    )
  }, [])

  const missingServer = useMemo(() => {
    const file = tab.path.split('/').pop() ?? tab.path
    const dot = file.lastIndexOf('.')
    const ext = dot > 0 ? file.slice(dot + 1).toLowerCase() : ''
    if (!ext) return null
    const match = lspServers.find((s) => s.extensions.includes(ext))
    if (!match || match.source !== 'missing') return null
    if (bannerDismissed[match.id]) return null
    return match
  }, [tab.path, lspServers, bannerDismissed])

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
      quickSuggestions: true,
      suggestOnTriggerCharacters: true,
      parameterHints: { enabled: true },
      hover: { enabled: true, delay: 300 },
      links: true,
      folding: true,
      // Alt = multi-cursor so Ctrl/Cmd+click is free for Go to Definition (VS Code).
      multiCursorModifier: 'alt',
      definitionLinkOpensInPeek: false,
      codeLens: true,
      gotoLocation: {
        multipleDefinitions: 'goto',
        // Always prefer peek for multi-ref; CodeLens forces peek even for 1 hit.
        multipleReferences: 'peek',
        multipleDeclarations: 'goto',
        multipleImplementations: 'peek',
        multipleTypeDefinitions: 'goto',
      },
      // Peek chrome tracks koma theme (title actions, tree focus).
      peekWidgetDefaultFocus: 'tree',
      renderValidationDecorations: 'on',
      matchBrackets: 'always',
      bracketPairColorization: { enabled: true },
      guides: { indentation: true, bracketPairs: false },
      stickyScroll: { enabled: true },
      inlayHints: { enabled: 'off' },
      // Dim CodeLens to match VS Code secondary chrome.
      // (color comes from editorCodeLens.foreground theme token)
    })
    monaco.editor.setTheme(theme)
    editorRef.current = editor

    editor.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyCode.KeyS, () => {
      useKoma.getState().saveCodingFile(tab.root, tab.path)
    })
    editor.addCommand(monaco.KeyCode.F12, () => {
      void editor.getAction('editor.action.revealDefinition')?.run()
    })
    editor.addCommand(monaco.KeyMod.Alt | monaco.KeyCode.F12, () => {
      void editor.getAction('editor.action.peekDefinition')?.run()
    })
    editor.addCommand(monaco.KeyMod.Shift | monaco.KeyCode.F12, () => {
      void editor.getAction('editor.action.referenceSearch.trigger')?.run()
    })

    // Selection → composer: `@path:start-end` + fenced buffer text, then focus chat.
    editor.addAction({
      id: 'koma.askInChat',
      label: 'Ask in chat',
      contextMenuGroupId: 'navigation',
      contextMenuOrder: 0.5,
      precondition: 'editorHasSelection',
      run: (ed) => {
        const model = ed.getModel()
        const sel = ed.getSelection()
        if (!model || !sel || sel.isEmpty()) return
        let startLine = Math.min(sel.startLineNumber, sel.endLineNumber)
        let endLine = Math.max(sel.startLineNumber, sel.endLineNumber)
        // Line-wise selections often end at column 1 of the next line.
        if (
          endLine > startLine &&
          sel.endLineNumber > sel.startLineNumber &&
          sel.endColumn === 1
        ) {
          endLine = endLine - 1
        }
        const selectedText = model.getValueInRange(sel)
        const workdirs = (useKoma.getState().settingsValues?.workdir ?? []).filter(Boolean)
        const payload = codingAskInChatPayload(
          tab.root,
          tab.path,
          workdirs,
          startLine,
          endLine,
          selectedText,
        )
        useKoma.getState().askCodingSelectionInChat(payload)
      },
    })

    const sub = editor.onDidChangeModelContent(() => {
      if (applyingRef.current) return
      const model = editor.getModel()
      if (!model) return
      const text = model.getValue()
      useKoma.getState().updateCodingContent(tab.root, tab.path, text)
      if (lspChangeTimerRef.current) clearTimeout(lspChangeTimerRef.current)
      lspChangeTimerRef.current = setTimeout(() => {
        lspChangeTimerRef.current = null
        if (!lspOpenedRef.current) return
        useKoma.getState().req({
          r: 'LspDidChange',
          root: tab.root,
          path: tab.path,
          text,
        })
        // Re-warm CodeLens counts for the new content (cache keyed by hash).
        warmCodeLensCache(
          (body) => useKoma.getState().req(body as never),
          tab.root,
          tab.path,
          text,
        )
      }, LSP_CHANGE_MS)
    })

    const onReveal = (ev: Event) => {
      const detail = (ev as CustomEvent).detail as
        | { root: string; path: string; line: number; column: number }
        | undefined
      if (!detail) return
      if (detail.root !== tab.root || detail.path !== tab.path) return
      // Drop any queued reveal so a later content paint does not re-jump.
      consumeReveal(tab.root, tab.path)
      const ed = editorRef.current
      if (!ed) return
      ed.setPosition({ lineNumber: detail.line, column: detail.column })
      ed.revealLineInCenter(detail.line)
      ed.focus()
    }
    window.addEventListener('koma-reveal-line', onReveal)

    return () => {
      window.removeEventListener('koma-reveal-line', onReveal)
      sub.dispose()
      if (lspChangeTimerRef.current) clearTimeout(lspChangeTimerRef.current)
      // Detach only — keep the file:// model alive for peek widgets / reopen.
      editor.setModel(null)
      editor.dispose()
      editorRef.current = null
      modelRef.current = null
      lspOpenedRef.current = false
    }
  }, [tab.root, tab.path])

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
      const uri = monacoUriFromPath(tab.root, tab.path)
      let model = monaco.editor.getModel(uri)
      if (!model) {
        model = monaco.editor.createModel(next, lang, uri)
      } else if (model.getValue() !== next) {
        const pos = editor.getPosition()
        model.setValue(next)
        if (pos) editor.setPosition(pos)
      }
      stampModelPath(model, tab.root, tab.path)
      if (editor.getModel() !== model) editor.setModel(model)
      modelRef.current = model
    } finally {
      queueMicrotask(() => {
        applyingRef.current = false
      })
    }

    editor.updateOptions({
      readOnly: fileState.binary || fileState.tooLarge || !!fileState.error || fileState.conflict,
    })

    // Go-to-def / Problems may open this tab before content is ready — apply
    // the queued reveal once the model has text.
    const reveal = consumeReveal(tab.root, tab.path)
    if (reveal) {
      const apply = () => {
        const ed = editorRef.current
        if (!ed) return
        ed.setPosition({ lineNumber: reveal.line, column: reveal.column })
        ed.revealLineInCenter(reveal.line)
        ed.focus()
      }
      queueMicrotask(apply)
      // Second pass after layout / late model attach.
      setTimeout(apply, 50)
    }

    if (
      !lspOpenedRef.current &&
      !fileState.binary &&
      !fileState.tooLarge &&
      !fileState.error &&
      fileState.content != null
    ) {
      lspOpenedRef.current = true
      req({
        r: 'LspDidOpen',
        root: tab.root,
        path: tab.path,
        languageId: languageIdForPath(tab.path),
        text: fileState.content,
      })
      const uri = pathToUri(tab.root, tab.path)
      const diags = useKoma.getState().lspDiagnostics[uri]
      if (diags) applyDiagnosticsToMonaco(uri, diags)
      // Eager CodeLens counts so reopen / first paint hits cache.
      warmCodeLensCache(
        (body) => useKoma.getState().req(body as never),
        tab.root,
        tab.path,
        fileState.content,
      )
    }
  }, [
    fileState?.content,
    fileState?.loading,
    fileState?.binary,
    fileState?.tooLarge,
    fileState?.error,
    fileState?.conflict,
    fileState?.fingerprint,
    tab.path,
    tab.root,
    req,
  ])

  useEffect(() => {
    const uri = pathToUri(tab.root, tab.path)
    const diags = lspDiagnostics[uri]
    if (diags) applyDiagnosticsToMonaco(uri, diags)
  }, [lspDiagnostics, tab.root, tab.path])

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
  // Known media / office types always use the binary viewer (even if FileRead
  // returned text, e.g. SVG without NULs). Don't wait for FileRead — the viewer
  // fetches bytes itself via FileDownloadBytes.
  const viewKind = viewerKindForPath(tab.path)
  if (viewKind !== 'text' && !fileState?.error && !fileState?.tooLarge) {
    return (
      <div className="flex h-full w-full flex-col">
        <EditorChrome
          path={tab.path}
          status={fileState?.binary ? status : kindStatus(viewKind)}
          canSave={false}
          canRevert={false}
          saving={false}
          onSave={() => {}}
          onRevert={() => {}}
        />
        <CodingFileViewer
          root={tab.root}
          path={tab.path}
          onDownload={() => useKoma.getState().downloadCodingFile(tab.root, tab.path)}
        />
      </div>
    )
  }

  if (fileState?.binary) {
    return (
      <div className="flex h-full w-full flex-col">
        <EditorChrome path={tab.path} status={status} canSave={false} canRevert={false} saving={false} onSave={() => {}} onRevert={() => {}} />
        <div className="flex min-h-0 flex-1 flex-col items-center justify-center gap-2 px-6 text-center text-[12px] text-koma-dim">
          <div>Binary file — no preview</div>
          <button
            type="button"
            onClick={() => useKoma.getState().downloadCodingFile(tab.root, tab.path)}
            className="flex items-center gap-1 rounded border border-koma-border px-2 py-1 text-[11.5px] text-koma-fg hover:bg-koma-hover"
          >
            <Download size={12} />
            Download
          </button>
        </div>
      </div>
    )
  }
  if (fileState?.tooLarge) {
    return (
      <div className="flex h-full w-full flex-col">
        <EditorChrome path={tab.path} status={status} canSave={false} canRevert={false} saving={false} onSave={() => {}} onRevert={() => {}} />
        <div className="flex min-h-0 flex-1 items-center justify-center px-6 text-center text-[12px] text-koma-dim">
          File too large to edit
        </div>
      </div>
    )
  }
  if (fileState?.error && fileState.content === null) {
    return (
      <div className="flex h-full w-full flex-col">
        <EditorChrome path={tab.path} status={status} canSave={false} canRevert={false} saving={false} onSave={() => {}} onRevert={() => {}} />
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
      {missingServer && (
        <div className="flex flex-none items-center gap-2 border-b border-koma-border bg-koma-accent/10 px-3 py-1.5 text-[12px] text-koma-fg">
          <span className="min-w-0 flex-1 truncate opacity-85">
            Install <strong className="font-semibold">{missingServer.name}</strong> for
            language features on this filetype.
          </span>
          <button
            type="button"
            onClick={() => lspInstall(missingServer.id, false, false)}
            disabled={!!lspProgress[missingServer.id] && lspProgress[missingServer.id].pct < 100}
            className="flex flex-none items-center gap-1 rounded border border-koma-accent/40 bg-koma-accent/15 px-2 py-0.5 text-[11.5px] font-medium text-koma-accent hover:bg-koma-accent/25 disabled:opacity-50"
          >
            {lspProgress[missingServer.id] && lspProgress[missingServer.id].pct < 100 ? (
              <BrailleSpinner size={12} />
            ) : (
              <Download size={12} />
            )}
            Install
          </button>
          <button
            type="button"
            onClick={() => setBannerDismissed((m) => ({ ...m, [missingServer.id]: true }))}
            aria-label="Dismiss"
            className="flex h-6 w-6 flex-none items-center justify-center rounded text-koma-fg opacity-50 hover:bg-koma-hover hover:opacity-100"
          >
            <X size={13} />
          </button>
        </div>
      )}
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

function kindStatus(kind: ViewerKind): string {
  switch (kind) {
    case 'image':
      return 'Image'
    case 'pdf':
      return 'PDF'
    case 'video':
      return 'Video'
    case 'sqlite':
      return 'SQLite'
    case 'docx':
      return 'Word'
    case 'excel':
      return 'Excel'
    default:
      return 'Preview'
  }
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
  return (
    <div className="flex h-8 flex-none items-center gap-2 border-b border-koma-border bg-koma-panel px-3 text-[12px]">
      <Code2 size={13} className="flex-none text-koma-dim" />
      <span className="min-w-0 flex-1 truncate font-mono text-koma-fg" title={path}>
        {path}
      </span>
      <span className="flex-none text-[11px] text-koma-dim">{status}</span>
      <button
        type="button"
        onClick={onRevert}
        disabled={!canRevert || saving}
        title="Revert"
        className="flex h-6 w-6 flex-none items-center justify-center rounded text-koma-dim hover:bg-koma-hover hover:text-koma-fg disabled:opacity-30"
      >
        <RotateCcw size={13} />
      </button>
      <button
        type="button"
        onClick={onSave}
        disabled={!canSave}
        title="Save"
        className="flex h-6 w-6 flex-none items-center justify-center rounded text-koma-dim hover:bg-koma-hover hover:text-koma-fg disabled:opacity-30"
      >
        <Save size={13} />
      </button>
    </div>
  )
}
