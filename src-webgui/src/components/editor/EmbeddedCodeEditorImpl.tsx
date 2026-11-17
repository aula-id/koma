import { useEffect, useId, useRef, useState } from 'react'
import * as monaco from 'monaco-editor/esm/vs/editor/editor.api'
import { applyKomaTheme, attachJsonSchema, initMonaco, readMonoFont } from '../../lib/monaco-setup'
import { EMBEDDED_BOX, embeddedHeightStyle, type EmbeddedCodeEditorProps } from './EmbeddedCodeEditor'

const LINE_HEIGHT = 19
const PAD = 6

/** The Monaco half of EmbeddedCodeEditor. Import it only through the lazy wrapper. */
export default function EmbeddedCodeEditorImpl(props: EmbeddedCodeEditorProps) {
  const {
    value, language = 'plaintext', readOnly = false, height, placeholder, ariaLabel,
    jsonSchema, lineNumbers = false, wordWrap = true, autoFocus = false, className = '', testId,
  } = props
  const uid = useId().replace(/[^a-zA-Z0-9]/g, '')
  const hostRef = useRef<HTMLDivElement | null>(null)
  const editorRef = useRef<monaco.editor.IStandaloneCodeEditor | null>(null)
  const modelRef = useRef<monaco.editor.ITextModel | null>(null)
  // Latest callbacks and props, read by listeners created once.
  const live = useRef(props)
  live.current = props
  const [grown, setGrown] = useState<number | null>(null)
  const [ready, setReady] = useState(false)
  const auto = typeof height === 'object' && height !== null
  const minLines = auto ? height.minLines : 0
  const maxLines = auto ? height.maxLines : 0

  useEffect(() => {
    const host = hostRef.current
    if (!host) return
    initMonaco()
    const theme = applyKomaTheme()
    const uri = monaco.Uri.parse(`koma-embedded://${uid}/model`)
    const model = monaco.editor.createModel(live.current.value, live.current.language ?? 'plaintext', uri)
    model.setEOL(monaco.editor.EndOfLineSequence.LF)
    const editor = monaco.editor.create(host, {
      model,
      theme,
      automaticLayout: true,
      minimap: { enabled: false },
      scrollBeyondLastLine: false,
      fontFamily: readMonoFont(),
      fontSize: 12,
      lineHeight: LINE_HEIGHT,
      padding: { top: PAD, bottom: PAD },
      folding: false,
      glyphMargin: false,
      lineDecorationsWidth: 6,
      lineNumbersMinChars: 3,
      renderLineHighlight: 'none',
      overviewRulerLanes: 0,
      hideCursorInOverviewRuler: true,
      overviewRulerBorder: false,
      stickyScroll: { enabled: false },
      scrollbar: { alwaysConsumeMouseWheel: false, useShadows: false },
      fixedOverflowWidgets: true,
      tabSize: 2,
      insertSpaces: true,
      detectIndentation: false,
      renderValidationDecorations: 'on',
      quickSuggestions: { other: true, comments: false, strings: true },
      contextmenu: true,
    })
    editorRef.current = editor
    modelRef.current = model
    const detachSchema = live.current.jsonSchema && (live.current.language ?? '') === 'json'
      ? attachJsonSchema(uri.toString(), live.current.jsonSchema)
      : null

    const sub = model.onDidChangeContent(() => live.current.onChange(model.getValue(monaco.editor.EndOfLinePreference.LF)))
    // Scoped to this editor instance (never a global keybinding).
    editor.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyCode.Enter, () => live.current.onSubmit?.())

    editor.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyCode.KeyS, () => live.current.onSave?.())

    // Escape then Tab leaves the editor. Capturing on this host keeps it from reaching
    // Monaco and from affecting any other editor.
    let escaped = false
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        const open = host.querySelector('.suggest-widget.visible, .find-widget.visible, .parameter-hints-widget.visible')
        escaped = !open
        return
      }
      if (e.key === 'Tab' && escaped && !e.ctrlKey && !e.metaKey && !e.altKey) {
        escaped = false
        e.stopPropagation()
        return
      }
      escaped = false
    }
    host.addEventListener('keydown', onKey, true)

    const size = editor.onDidContentSizeChange((e) => {
      if (!live.current.height || typeof live.current.height !== 'object') return
      const { minLines: lo, maxLines: hi } = live.current.height
      setGrown(Math.min(Math.max(e.contentHeight, lo * LINE_HEIGHT + 2 * PAD), hi * LINE_HEIGHT + 2 * PAD))
    })
    if (live.current.autoFocus) editor.focus()
    setReady(true)
    return () => {
      host.removeEventListener('keydown', onKey, true)
      size.dispose()
      sub.dispose()
      detachSchema?.()
      editor.dispose()
      model.dispose()
      editorRef.current = null
      modelRef.current = null
    }
  }, [uid])

  // External value changes (a reset, a switch from the form) replace the text as one undoable edit.
  useEffect(() => {
    const model = modelRef.current
    const editor = editorRef.current
    if (!model || !editor || model.getValue(monaco.editor.EndOfLinePreference.LF) === value) return
    // Model edits, not editor.executeEdits: that silently no-ops while the editor is read-only.
    editor.pushUndoStop()
    model.pushEditOperations(null, [{ range: model.getFullModelRange(), text: value }], () => null)
    editor.pushUndoStop()
  }, [value])

  useEffect(() => {
    const editor = editorRef.current
    const model = modelRef.current
    if (!editor || !model) return
    if (model.getLanguageId() !== language) monaco.editor.setModelLanguage(model, language)
    editor.updateOptions({
      readOnly,
      ariaLabel,
      placeholder: placeholder ?? '',
      lineNumbers: lineNumbers ? 'on' : 'off',
      wordWrap: wordWrap ? 'on' : 'off',
    })
  }, [language, readOnly, ariaLabel, placeholder, lineNumbers, wordWrap])

  useEffect(() => {
    if (autoFocus) editorRef.current?.focus()
  }, [autoFocus])

  const style = auto
    ? { height: grown ?? minLines * LINE_HEIGHT + 2 * PAD, maxHeight: maxLines * LINE_HEIGHT + 2 * PAD }
    : embeddedHeightStyle(height)
  return (
    <div className={`${EMBEDDED_BOX} ${className}`} style={style} data-testid={testId} data-editor-state={ready ? 'ready' : 'loading'}>
      <div ref={hostRef} className="absolute inset-0" data-embedded-monaco />
    </div>
  )
}
