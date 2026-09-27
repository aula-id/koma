import { bindDebugEditor } from '../lib/coding-debug'
import { useEffect, useMemo, useRef, useState } from 'react'
import * as monaco from 'monaco-editor/esm/vs/editor/editor.api'
import { Download, X } from 'lucide-react'
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
  registerLspDidChangeFlusher,
  flushPendingLspDidChange,
} from '../lib/monaco-lsp'
import { codingAskInChatPayload } from '../lib/codingRef'
import { viewerKindForPath, type ViewerKind } from '../lib/viewerKind'
import { useKoma, type Tab } from '../store/koma'
import { fileKey } from '../store/coding'
import { isTabVisible, normalizeGroups } from '../store/editorGroups'
import { BrailleSpinner } from './BrailleSpinner'
import { CodingFileViewer } from './CodingFileViewer'
import { EditorChrome } from './EditorChrome'
import { configureCodingEditor } from '../lib/coding-editor-config'
import { recordCodingLocation, navigateCodingHistory } from '../lib/coding-navigation'
import { CodingOutline } from './CodingOutline'
import { showCodingRefactor } from './CodingRefactor'
import { undoWorkspaceEdit } from '../lib/coding-edits'
import { showCodingHistory } from './CodingHistory'
import { isMarkdownPath } from '../lib/markdownPreview'

type CodingTab = Extract<Tab, { kind: 'codingFile' }>

const AUTOSAVE_MS = 750
// Diagnostics / CodeLens can lag typing; completion flushes pending didChange
// immediately via registerLspDidChangeFlusher so the server stays current.
const LSP_CHANGE_MS = 120
// CodeLens ref-counts are expensive (documentSymbol + N references RPCs) and
// share the host LSP mutex with hover/completion. Only warm on open / save idle.
const CODELENS_IDLE_MS = 2500
const viewStates = new Map<string, monaco.editor.ICodeEditorViewState>()

export default function CodeEditorTab({ tab }: { tab: CodingTab }) {
  const host = useKoma(s => s.remoteState.hostId ?? 'local')
  return <WorkspaceCodeEditor key={JSON.stringify([host, tab.id])} tab={tab} />
}
function WorkspaceCodeEditor({ tab }: { tab: CodingTab }) {
  const hostId = useKoma(s => s.remoteState.hostId ?? 'local')
  const viewKey = JSON.stringify([hostId, tab.root, tab.path])
  const [outlineOpen, setOutlineOpen] = useState(false)
  const containerRef = useRef<HTMLDivElement | null>(null)
  const editorRef = useRef<monaco.editor.IStandaloneCodeEditor | null>(null)
  const modelRef = useRef<monaco.editor.ITextModel | null>(null)
  // Generation counter: each store→model apply bumps this; didChange is ignored
  // while applyGenRef !== 0 (sync) AND until the matching clear runs after Monaco
  // flushes. A plain boolean + rAF was still losing races on split/layout.
  const applyGenRef = useRef(0)
  const autosaveTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const lspChangeTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const lspOpenedRef = useRef(false)

  const fileState = useKoma((s) => s.coding.files[fileKey(tab.root, tab.path)] ?? null)
  const codingAutosave = useKoma((s) => !!s.settingsValues?.codingAutosave)
  const saveCodingFile = useKoma((s) => s.saveCodingFile)
  const revertCodingFile = useKoma((s) => s.revertCodingFile)
  const lspServers = useKoma((s) => s.lspServers)
  const lspProgress = useKoma((s) => s.lspProgress)
  // Markers are applied in the store push handler — do NOT subscribe to the
  // whole lspDiagnostics map here (every URI publish would re-render every
  // mounted coding tab).
  const refreshLsp = useKoma((s) => s.refreshLsp)
  const lspInstall = useKoma((s) => s.lspInstall)
  const req = useKoma((s) => s.req)
  const [bannerDismissed, setBannerDismissed] = useState<Record<string, boolean>>({})
  // Boolean only — avoid re-render on every ui.groupSizes / toast tick.
  const isActive = useKoma((s) => isTabVisible(normalizeGroups(s.ui), tab.id))

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
    !fileState.loading
  )
  const openPreview = isMarkdownPath(tab.path)
    ? () => useKoma.getState().openCodingFile(tab.root, tab.path, { preview: true })
    : undefined
  const canSave = !!(canEdit && fileState?.dirty && !fileState.saving && !fileState.conflict)
  const canRevert = !!(fileState && (fileState.dirty || fileState.conflict) && !fileState.saving)

  const status = useMemo(() => {
    if (!fileState) return 'Loading…'
    if (fileState.loading && fileState.content == null) return 'Loading…'
    if (fileState.saving) return 'Saving…'
    if (fileState.conflict) return 'Conflict — reload required'
    if (fileState.error) return fileState.error
    if (fileState.binary) return 'Binary'
    if (fileState.tooLarge) return 'Too large'
    if (fileState.dirty) return fileState.manualSaveRequired ? 'Modified · save to apply' : codingAutosave ? 'Modified · autosave on' : 'Modified'
    return 'Saved'
  }, [fileState, codingAutosave])

  useEffect(() => {
    if (autosaveTimerRef.current) {
      clearTimeout(autosaveTimerRef.current)
      autosaveTimerRef.current = null
    }
    if (!codingAutosave || fileState?.manualSaveRequired) return
    if (!fileState?.dirty) return
    if (fileState.content == null) return
    if (fileState.saving || fileState.conflict || fileState.binary || fileState.tooLarge || fileState.error) {
      return
    }
    if (fileState.content === (fileState.savedContent ?? '')) return

    autosaveTimerRef.current = setTimeout(() => {
      autosaveTimerRef.current = null
      const cur = useKoma.getState().coding.files[fileKey(tab.root, tab.path)]
      if (!cur?.dirty || cur.content == null || cur.saving || cur.conflict || cur.manualSaveRequired) return
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
    fileState?.manualSaveRequired,
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
      inlayHints: { enabled: 'on' },
      // Dim CodeLens to match VS Code secondary chrome.
      // (color comes from editorCodeLens.foreground theme token)
    })
    monaco.editor.setTheme(theme)
    editorRef.current = editor
    const stopDebug = bindDebugEditor(monaco, editor, { hostId, root: tab.root }, tab.path)
    const rememberLocation = () => {
      const position = editor.getPosition()
      if (position && editor.hasTextFocus()) recordCodingLocation(hostId, { root: tab.root, path: tab.path, line: position.lineNumber, column: position.column })
    }
    const cursor = editor.onDidChangeCursorPosition(rememberLocation)
    const focus = editor.onDidFocusEditorText(rememberLocation)
    editor.addAction({ id: 'koma.outline', label: 'Toggle Document Outline', run: () => setOutlineOpen(value => !value) })
    editor.addAction({ id: 'koma.navigateBack', label: 'Go Back', keybindings: [monaco.KeyMod.Alt | monaco.KeyCode.LeftArrow], run: () => navigateCodingHistory(-1) })
    editor.addAction({ id: 'koma.navigateForward', label: 'Go Forward', keybindings: [monaco.KeyMod.Alt | monaco.KeyCode.RightArrow], run: () => navigateCodingHistory(1) })
    const stopConfiguration = configureCodingEditor(editor, { hostId: useKoma.getState().remoteState.hostId ?? 'local', root: tab.root }, tab.path)

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

    editor.addAction({ id: 'koma.rename', label: 'Rename Symbol…', keybindings: [monaco.KeyCode.F2], contextMenuGroupId: 'navigation', contextMenuOrder: 1.5, run: async () => {
      const model = editor.getModel(), position = editor.getPosition()
      if (!model || !position) return
      const version = model.getVersionId()
      await flushPendingLspDidChange(tab.root, tab.path)
      if (model.isDisposed() || model.getVersionId() !== version) return
      const state = useKoma.getState()
      showCodingRefactor({ workspace: { hostId: state.remoteState.hostId ?? 'local', root: tab.root }, path: tab.path,
        position: { line: position.lineNumber - 1, character: position.column - 1 }, word: model.getWordAtPosition(position)?.word,
        mode: 'rename', snapshot: state.coding.files, generation: state.coding._sessionGen })
    } })
    editor.addAction({ id: 'koma.undoWorkspaceEdit', label: 'Undo Workspace Edit', run: async () => {
      try { await undoWorkspaceEdit() } catch (error) { const message = error instanceof Error ? error.message : String(error); useKoma.setState(s => { const id = s.ui.toastSeq + 1; return { ui: { ...s.ui, toastSeq: id, toast: { id, text: message, kind: 'error' } } } }) }
    } })
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

    let codeLensIdleTimer: ReturnType<typeof setTimeout> | null = null
    const sub = editor.onDidChangeModelContent(() => {
      // Ignore while a store→model apply is in flight (incl. post-flush window).
      if (applyGenRef.current !== 0) return
      const model = editor.getModel()
      if (!model) return
      // Always LF so store ↔ model never thrash on CRLF (React #185).
      const text = model.getValue(monaco.editor.EndOfLinePreference.LF)
      const key = fileKey(tab.root, tab.path)
      const prev = useKoma.getState().coding.files[key]
      if (prev?.content === text) {
        // External setValue / setEOL can still emit didChange; don't write back.
        return
      }
      useKoma.getState().updateCodingContent(tab.root, tab.path, text)
      if (lspChangeTimerRef.current) clearTimeout(lspChangeTimerRef.current)
      lspChangeTimerRef.current = setTimeout(() => {
        lspChangeTimerRef.current = null
        if (!lspOpenedRef.current) return
        // Fresh text at fire time — the closed-over `text` may be stale if the
        // user kept typing through the debounce window.
        const latest = editor
          .getModel()
          ?.getValue(monaco.editor.EndOfLinePreference.LF)
        if (latest == null) return
        useKoma.getState().req({
          r: 'LspDidChange',
          root: tab.root,
          path: tab.path,
          text: latest,
        })
      }, LSP_CHANGE_MS)
      // Defer CodeLens warm until typing goes idle — never on every didChange.
      if (codeLensIdleTimer) clearTimeout(codeLensIdleTimer)
      codeLensIdleTimer = setTimeout(() => {
        codeLensIdleTimer = null
        if (!lspOpenedRef.current) return
        const m = editor.getModel()
        if (!m) return
        warmCodeLensCache(
          (body) => useKoma.getState().req(body as never),
          tab.root,
          tab.path,
          m.getVersionId(),
        )
      }, CODELENS_IDLE_MS)
    })

    // Completion/resolve call this to push didChange before the RPC so the
    // server buffer matches Monaco (debounced path alone is too late).
    const flushDidChange = () => {
      if (lspChangeTimerRef.current) {
        clearTimeout(lspChangeTimerRef.current)
        lspChangeTimerRef.current = null
      }
      if (!lspOpenedRef.current) return
      const latest = editor
        .getModel()
        ?.getValue(monaco.editor.EndOfLinePreference.LF)
      if (latest == null) return
      useKoma.getState().req({
        r: 'LspDidChange',
        root: tab.root,
        path: tab.path,
        text: latest,
      })
    }
    registerLspDidChangeFlusher(tab.root, tab.path, flushDidChange)

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

    const onCodingCommand = (event: Event) => {
      const command = (event as CustomEvent<{ id: string; tabId: string }>).detail
      if (command?.tabId !== tab.id) return
      editor.focus()
      if (command.id === 'save') useKoma.getState().saveCodingFile(tab.root, tab.path)
      else void editor.getAction(command.id)?.run()
    }
    window.addEventListener('koma-coding-command', onCodingCommand)

    return () => {
      window.removeEventListener('koma-reveal-line', onReveal)
      window.removeEventListener('koma-coding-command', onCodingCommand)
      stopConfiguration()
      stopDebug()
      cursor.dispose(); focus.dispose()
      const view = editor.saveViewState()
      if (view) viewStates.set(viewKey, view)
      if (viewStates.size > 100) viewStates.delete(viewStates.keys().next().value!)
      sub.dispose()
      registerLspDidChangeFlusher(tab.root, tab.path, null)
      if (lspChangeTimerRef.current) clearTimeout(lspChangeTimerRef.current)
      if (codeLensIdleTimer) clearTimeout(codeLensIdleTimer)
      // Detach only — keep the file:// model alive for peek widgets / reopen.
      editor.setModel(null)
      editor.dispose()
      editorRef.current = null
      modelRef.current = null
      lspOpenedRef.current = false
    }
  }, [tab.root, tab.path, hostId])

  // Parent uses display:none for inactive panes; force layout on reveal so the
  // editor isn't stuck at 0×0 after WebKit skips ResizeObserver.
  useEffect(() => {
    if (!isActive) return
    const raf = requestAnimationFrame(() => editorRef.current?.layout())
    return () => cancelAnimationFrame(raf)
  }, [isActive])

  useEffect(() => {
    if (!fileState || !editorRef.current) return
    const editor = editorRef.current

    if (fileState.content === null) {
      editor.updateOptions({ readOnly: true })
      return
    }

    const lang = langFromPath(tab.path)
    // Host/store always uses \n; pin Monaco the same way so getValue() never
    // disagrees with store and retriggers this effect (React #185).
    const next = fileState.content.replace(/\r\n/g, '\n').replace(/\r/g, '\n')
    const gen = applyGenRef.current + 1
    applyGenRef.current = gen
    try {
      const uri = monacoUriFromPath(tab.root, tab.path)
      let model = monaco.editor.getModel(uri)
      const hostId = useKoma.getState().remoteState.hostId ?? 'local'
      if (model && (model as unknown as { __komaHost?: string }).__komaHost !== hostId) {
        model.dispose()
        model = null
      }
      if (!model) {
        model = monaco.editor.createModel(next, lang, uri)
        model.setEOL(monaco.editor.EndOfLineSequence.LF)
      } else {
        const cur = model.getValue(monaco.editor.EndOfLinePreference.LF)
        // Only touch the model when text actually differs — bare setEOL on every
        // save/fingerprint tick was still emitting didChange on some WebKit builds.
        if (cur !== next) {
          const pos = editor.getPosition()
          model.pushStackElement()
          model.pushEditOperations([], [{ range: model.getFullModelRange(), text: next }], () => null)
          model.pushStackElement()
          if (lspOpenedRef.current) useKoma.getState().req({ r: 'LspDidChange', root: tab.root, path: tab.path, text: next })
          model.setEOL(monaco.editor.EndOfLineSequence.LF)
          if (pos) editor.setPosition(pos)
        } else if (model.getEndOfLineSequence() !== monaco.editor.EndOfLineSequence.LF) {
          model.setEOL(monaco.editor.EndOfLineSequence.LF)
        }
      }
      stampModelPath(model, tab.root, tab.path)
      if (editor.getModel() !== model) {
        editor.setModel(model)
        const view = viewStates.get(viewKey)
        if (view) editor.restoreViewState(view)
      }
      modelRef.current = model
    } finally {
      // Clear only this generation after Monaco has flushed model events.
      queueMicrotask(() => {
        requestAnimationFrame(() => {
          if (applyGenRef.current === gen) applyGenRef.current = 0
        })
      })
    }

    editor.updateOptions({
      readOnly: fileState.binary || fileState.tooLarge || fileState.loading,
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
    // Fingerprint changes do not require touching the model. Loading only
    // updates readOnly; unchanged content never triggers setValue/setEOL.
  }, [
    fileState?.content,
    fileState?.binary,
    fileState?.tooLarge,
    fileState?.loading,
    fileState?.conflict,
    tab.path,
    tab.root,
    hostId,
  ])

  // Attach LSP when content is ready AND the matching server is installed.
  useEffect(() => {
    const restart = (event: Event) => {
      const workspace = (event as CustomEvent<{ hostId: string; root: string }>).detail
      const store = useKoma.getState()
      if (workspace.hostId !== (store.remoteState.hostId ?? 'local') || workspace.root !== tab.root) return
      const model = modelRef.current
      if (!model || model.isDisposed()) return
      lspOpenedRef.current = true
      store.req({ r: 'LspDidOpen', root: tab.root, path: tab.path, languageId: languageIdForPath(tab.path), text: model.getValue() })
    }
    window.addEventListener('koma-lsp-restart', restart)
    return () => window.removeEventListener('koma-lsp-restart', restart)
  }, [tab.root, tab.path])

  // Do not mark opened while source === 'missing' — install updates lspServers
  // without remounting the tab, so this effect must re-run and send LspDidOpen.
  useEffect(() => {
    if (lspOpenedRef.current) return
    if (
      !fileState ||
      fileState.content == null ||
      fileState.binary ||
      fileState.tooLarge ||
      fileState.error
    ) {
      return
    }
    const file = tab.path.split('/').pop() ?? tab.path
    const dot = file.lastIndexOf('.')
    const ext = dot > 0 ? file.slice(dot + 1).toLowerCase() : ''
    if (!ext) return
    // Wait for catalogue so we don't burn the one-shot open while status is
    // unknown. If refreshLsp never lands (empty forever), fail open after a
    // short grace so LSP still attaches for managed/path servers.
    if (lspServers.length === 0) {
      // Effect re-runs when lspServers updates; only schedule a late retry once.
      const t = window.setTimeout(() => {
        if (lspOpenedRef.current) return
        if (useKoma.getState().lspServers.length > 0) return
        // Catalogue still empty — try didOpen anyway (host no-ops missing servers).
        lspOpenedRef.current = true
        useKoma.getState().req({
          r: 'LspDidOpen',
          root: tab.root,
          path: tab.path,
          languageId: languageIdForPath(tab.path),
          text: fileState.content!,
        })
        const uri = pathToUri(tab.root, tab.path)
        const diags = useKoma.getState().lspDiagnostics[uri]
        if (diags) applyDiagnosticsToMonaco(uri, diags)
        {
          const m =
            monaco.editor.getModel(monacoUriFromPath(tab.root, tab.path)) ??
            modelRef.current
          warmCodeLensCache(
            (body) => useKoma.getState().req(body as never),
            tab.root,
            tab.path,
            m?.getVersionId() ?? 1,
          )
        }
      }, 2500)
      return () => window.clearTimeout(t)
    }
    const matches = lspServers.filter((s) => s.extensions.includes(ext))
    if (!matches.some(s => s.source !== 'missing')) return

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
    {
      const m =
        monaco.editor.getModel(monacoUriFromPath(tab.root, tab.path)) ?? modelRef.current
      warmCodeLensCache(
        (body) => useKoma.getState().req(body as never),
        tab.root,
        tab.path,
        m?.getVersionId() ?? 1,
      )
    }
  }, [
    fileState?.content,
    fileState?.binary,
    fileState?.tooLarge,
    fileState?.error,
    tab.path,
    tab.root,
    lspServers,
    hostId,
    req,
  ])

  // Known media / office types always use the binary viewer (even if FileRead
  // returned text, e.g. SVG without NULs). Don't wait for FileRead — the viewer
  // fetches bytes itself via FileDownloadBytes.
  const viewKind = viewerKindForPath(tab.path)
  if (viewKind !== 'text' && (!fileState?.error || fileState.binary) && !fileState?.tooLarge) {
    return (
      <div className="flex h-full w-full flex-col">
        <EditorChrome
          path={tab.path}
          onTogglePreview={openPreview}
          onHistory={() => showCodingHistory(tab.root, tab.path)}
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
        <EditorChrome path={tab.path} onTogglePreview={openPreview}
          onHistory={() => showCodingHistory(tab.root, tab.path)} status={status} canSave={false} canRevert={false} saving={false} onSave={() => {}} onRevert={() => {}} />
        <div className="flex min-h-0 flex-1 flex-col items-center justify-center gap-2 px-6 text-center text-[12px] text-koma-dim">
          <div>{fileState.error ?? 'Binary file — no preview'}</div>
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
        <EditorChrome path={tab.path} onTogglePreview={openPreview}
          onHistory={() => showCodingHistory(tab.root, tab.path)} status={status} canSave={false} canRevert={false} saving={false} onSave={() => {}} onRevert={() => {}} />
        <div className="flex min-h-0 flex-1 items-center justify-center px-6 text-center text-[12px] text-koma-dim">
          File too large to edit
        </div>
      </div>
    )
  }
  if (fileState?.error && fileState.content === null) {
    return (
      <div className="flex h-full w-full flex-col">
        <EditorChrome path={tab.path} onTogglePreview={openPreview}
          onHistory={() => showCodingHistory(tab.root, tab.path)} status={status} canSave={false} canRevert={false} saving={false} onSave={() => {}} onRevert={() => {}} />
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
        onTogglePreview={openPreview}
          onHistory={() => showCodingHistory(tab.root, tab.path)}
        status={status}
        canSave={canSave}
        canRevert={canRevert}
        saving={!!fileState?.saving}
        onSave={() => saveCodingFile(tab.root, tab.path)}
        onRevert={() => revertCodingFile(tab.root, tab.path)}
      />
      {fileState?.conflict && <div className="flex flex-none items-center gap-2 border-b border-koma-border bg-koma-accent/10 px-3 py-1.5 text-[12px] text-koma-fg">
        <span className="min-w-0 flex-1">File changed on disk. Your edits are kept.</span>
        <button onClick={() => showCodingHistory(tab.root, tab.path, true)} className="flex-none rounded border border-koma-border px-2 py-0.5 text-[11px] hover:bg-koma-hover">Compare and resolve</button>
      </div>}
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
      <div className="relative flex min-h-0 flex-1">
        <div className="relative min-w-0 flex-1"><div ref={containerRef} className="absolute inset-0" /></div>
        {outlineOpen && <CodingOutline root={tab.root} path={tab.path} onClose={() => setOutlineOpen(false)} />}
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
