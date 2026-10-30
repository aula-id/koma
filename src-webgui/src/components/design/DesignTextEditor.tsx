import { useEffect, useLayoutEffect, useRef, type CSSProperties } from 'react'
import { retargetTextRuns, splitTextByRuns, type DesignDoc, type DesignTextRun } from '../../lib/design'
import { paintCss, weightCss } from './tabShared'

/**
 * In-place text editing that keeps styled runs visible. The browser owns the DOM while the user types,
 * so React never re-renders the content. It is rebuilt only when text or runs change from outside,
 * like a bold toggle in the inspector, and never during IME composition.
 */
export function DesignTextEditor({
  doc,
  text,
  runs,
  style,
  wrap = true,
  onText,
  onTextRange,
  onDone,
}: {
  doc: DesignDoc
  text: string
  runs: DesignTextRun[] | undefined
  style: CSSProperties
  /** Auto-width text grows on one line instead of wrapping inside its current box. */
  wrap?: boolean
  onText: (text: string) => void
  onTextRange?: (start: number, end: number) => void
  onDone: () => void
}) {
  const ref = useRef<HTMLDivElement>(null)
  const composing = useRef(false)
  const shown = useRef<{ text: string; runs: string } | null>(null)
  const onRangeRef = useRef(onTextRange)
  onRangeRef.current = onTextRange

  useLayoutEffect(() => {
    const el = ref.current
    if (!el || composing.current) return
    const key = { text, runs: JSON.stringify(runs ?? []) }
    if (shown.current && shown.current.text === key.text && shown.current.runs === key.runs) return
    const first = shown.current == null
    const caret = first ? null : readSelection(el)
    paintRuns(el, doc, text, runs)
    shown.current = key
    if (first) {
      el.focus()
      selectRange(el, 0, text.length)
    } else if (caret) selectRange(el, caret.start, caret.end)
  }, [doc, runs, text])

  useEffect(() => {
    const onSelection = () => {
      const el = ref.current
      const range = el && document.activeElement === el ? readSelection(el) : null
      if (range) onRangeRef.current?.(range.start, range.end)
    }
    document.addEventListener('selectionchange', onSelection)
    return () => document.removeEventListener('selectionchange', onSelection)
  }, [])

  const push = () => {
    const el = ref.current
    if (!el || composing.current) return
    const next = el.textContent ?? ''
    // The DOM already shows this text, and the store shifts runs the same way, so its echo must not rebuild under the caret.
    shown.current = { text: next, runs: JSON.stringify(retargetTextRuns(runs, text, next)) }
    onText(next)
  }

  return (
    <div
      ref={ref}
      contentEditable="plaintext-only"
      suppressContentEditableWarning
      spellCheck={false}
      className={`z-10 min-w-[1px] outline-none ${wrap ? 'whitespace-pre-wrap' : 'whitespace-pre'}`}
      style={style}
      onInput={push}
      onCompositionStart={() => {
        composing.current = true
      }}
      onCompositionEnd={() => {
        composing.current = false
        push()
      }}
      onBlur={onDone}
      onPointerDown={(event) => event.stopPropagation()}
      onKeyDown={(event) => {
        if (event.key === 'Escape') {
          event.preventDefault()
          ref.current?.blur()
        }
      }}
    />
  )
}

function paintRuns(el: HTMLElement, doc: DesignDoc, text: string, runs: DesignTextRun[] | undefined) {
  el.replaceChildren(
    ...splitTextByRuns(text, runs).map((part) => {
      if (!part.run) return document.createTextNode(part.text)
      const span = document.createElement('span')
      span.textContent = part.text
      const run = part.run
      if (run.fontSize) span.style.fontSize = `${run.fontSize}px`
      if (run.weight) span.style.fontWeight = String(weightCss(run.weight))
      if (run.italic) span.style.fontStyle = 'italic'
      const decoration = [run.underline ? 'underline' : '', run.strike ? 'line-through' : ''].filter(Boolean).join(' ')
      if (decoration) span.style.textDecoration = decoration
      if (run.fontFamily) span.style.fontFamily = run.fontFamily
      if (run.color) span.style.color = paintCss(doc, run.color, 'inherit')
      return span
    }),
  )
}

/** Character offsets of the selection inside the editor, or null when it is elsewhere. */
function readSelection(el: HTMLElement): { start: number; end: number } | null {
  const selection = window.getSelection()
  if (!selection?.rangeCount) return null
  const range = selection.getRangeAt(0)
  if (!el.contains(range.startContainer) || !el.contains(range.endContainer)) return null
  const offset = (node: Node, at: number) => {
    const before = document.createRange()
    before.selectNodeContents(el)
    before.setEnd(node, at)
    return before.toString().length
  }
  return { start: offset(range.startContainer, range.startOffset), end: offset(range.endContainer, range.endOffset) }
}

function selectRange(el: HTMLElement, start: number, end: number) {
  const selection = window.getSelection()
  if (!selection) return
  const range = document.createRange()
  const place = (target: number, edge: 'start' | 'end') => {
    const walker = document.createTreeWalker(el, NodeFilter.SHOW_TEXT)
    let left = target
    for (let node = walker.nextNode(); node; node = walker.nextNode()) {
      const length = node.textContent?.length ?? 0
      if (left <= length) {
        if (edge === 'start') range.setStart(node, left)
        else range.setEnd(node, left)
        return
      }
      left -= length
    }
    if (edge === 'start') range.setStart(el, el.childNodes.length)
    else range.setEnd(el, el.childNodes.length)
  }
  place(start, 'start')
  place(end, 'end')
  selection.removeAllRanges()
  selection.addRange(range)
}
