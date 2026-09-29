import { useEffect, useRef } from 'react'
import * as monaco from 'monaco-editor/esm/vs/editor/editor.api'
import { useKoma } from '../store/koma'
import {
  initMonaco,
  applyKomaTheme,
  readMonoFont,
  langFromPath,
} from '../lib/monaco-setup'
import { isTabVisible, normalizeGroups } from '../store/editorGroups'

export type CodePaneScroll = {
  setScrollTop: (next: number) => void
}

// The same Monaco setup, font and palette as the existing coding/diff tabs.
// lineHeight / folding / stickyScroll / wordWrap stay unset unless a caller
// pins them (the blame gutter needs a uniform line box).
export function CodePane({
  value,
  path = '',
  readOnly = true,
  onChange,
  revealLine,
  revealTick,
  tabId,
  lineHeight,
  folding,
  stickyScroll,
  wordWrap,
  onScroll,
  onLineHeight,
  scrollRef,
  highlight,
}: {
  value: string
  path?: string
  readOnly?: boolean
  onChange?: (value: string) => void
  revealLine?: number
  revealTick?: number
  tabId: string
  lineHeight?: number
  folding?: boolean
  stickyScroll?: boolean
  wordWrap?: 'off' | 'on' | 'wordWrapColumn' | 'bounded'
  onScroll?: (scrollTop: number) => void
  onLineHeight?: (lineHeight: number) => void
  scrollRef?: { current: CodePaneScroll | null }
  highlight?: { start: number; end: number } | null
}) {
  const host = useRef<HTMLDivElement>(null)
  const editor = useRef<monaco.editor.IStandaloneCodeEditor | null>(null)
  const callback = useRef(onChange)
  callback.current = onChange
  const onScrollRef = useRef(onScroll)
  onScrollRef.current = onScroll
  const onLineHeightRef = useRef(onLineHeight)
  onLineHeightRef.current = onLineHeight
  const changing = useRef(false)
  const visible = useKoma((s) => isTabVisible(normalizeGroups(s.ui), tabId))
  const highlightStart = highlight?.start ?? 0
  const highlightEnd = highlight?.end ?? 0
  useEffect(() => {
    if (!host.current) return
    initMonaco()
    const model = monaco.editor.createModel('', langFromPath(path))
    const options: monaco.editor.IStandaloneEditorConstructionOptions = {
      model,
      readOnly,
      automaticLayout: true,
      minimap: { enabled: false },
      scrollBeyondLastLine: false,
      fontFamily: readMonoFont(),
      fontSize: 12,
      theme: applyKomaTheme('koma-workbench'),
      lineNumbersMinChars: 3,
    }
    if (lineHeight != null) {
      options.lineHeight = lineHeight
      options.padding = { top: 0, bottom: 0 }
    }
    if (folding != null) options.folding = folding
    if (stickyScroll != null) options.stickyScroll = { enabled: stickyScroll }
    if (wordWrap != null) options.wordWrap = wordWrap
    const instance = monaco.editor.create(host.current, options)
    editor.current = instance
    const subscription = instance.onDidChangeModelContent(() => {
      if (!changing.current) callback.current?.(instance.getValue())
    })
    const scrollSub = instance.onDidScrollChange((event) => {
      if (event.scrollTopChanged) onScrollRef.current?.(event.scrollTop)
    })
    if (scrollRef) {
      scrollRef.current = {
        setScrollTop: (next) => instance.setScrollTop(next),
      }
    }
    onLineHeightRef.current?.(
      instance.getOption(monaco.editor.EditorOption.lineHeight),
    )
    onScrollRef.current?.(instance.getScrollTop())
    return () => {
      subscription.dispose()
      scrollSub.dispose()
      if (scrollRef) scrollRef.current = null
      instance.dispose()
      model.dispose()
      editor.current = null
    }
  }, [path, lineHeight, folding, stickyScroll, wordWrap, scrollRef])
  useEffect(() => {
    editor.current?.updateOptions({ readOnly })
  }, [readOnly])
  useEffect(() => {
    const instance = editor.current
    if (instance && instance.getValue() !== value) {
      changing.current = true
      instance.setValue(value)
      changing.current = false
    }
  }, [value, path])
  useEffect(() => {
    if (revealLine) editor.current?.revealLineInCenter(revealLine)
  }, [revealLine, revealTick])
  useEffect(() => {
    if (visible) {
      const frame = requestAnimationFrame(() => editor.current?.layout())
      return () => cancelAnimationFrame(frame)
    }
  }, [visible])
  useEffect(() => {
    const instance = editor.current
    if (!instance || highlightStart <= 0 || highlightEnd < highlightStart) return
    const marks = instance.createDecorationsCollection([
      {
        range: new monaco.Range(highlightStart, 1, highlightEnd, 1),
        options: { isWholeLine: true, className: 'koma-blame-line' },
      },
    ])
    return () => marks.clear()
  }, [highlightStart, highlightEnd, path, value])
  return <div ref={host} className="h-full min-h-0 w-full" />
}
