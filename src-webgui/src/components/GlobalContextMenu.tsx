import { useCallback, useEffect, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import { Bug, Clipboard, ClipboardPaste, RotateCcw } from 'lucide-react'

type Props = {
  onResume: () => void
  /** When true the global menu does not mount at all (onboarding). */
  hidden?: boolean
}

type Pos = { left: number; top: number }
type ContextTarget = {
  element: Element
  inputSelection: { start: number; end: number } | null
  contentRange: Range | null
  copyText: string | null
}

function postWin(a: string) {
  try {
    window.ipc?.postMessage(JSON.stringify({ t: 'win', a }))
  } catch {
    /* ipc unavailable */
  }
}

/** Open the native WebView inspector (host WinCmd::OpenDevTools). */
function openDevTools() {
  postWin('devtools')
}

function isMonacoTarget(el: EventTarget | null): boolean {
  if (!(el instanceof Element)) return false
  return !!el.closest('.monaco-editor, .monaco-diff-editor')
}

// Clamp after mount so the menu never renders off-screen.
function useClampedPos(raw: Pos, ref: React.RefObject<HTMLDivElement | null>) {
  const [pos, setPos] = useState(raw)
  useEffect(() => {
    const el = ref.current
    const w = el?.offsetWidth ?? 160
    const h = el?.offsetHeight ?? 100
    setPos({
      left: Math.max(4, Math.min(raw.left, window.innerWidth - w - 4)),
      top: Math.max(4, Math.min(raw.top, window.innerHeight - h - 4)),
    })
  }, [raw])
  return pos
}

// Detect whether a target can receive text paste.
function isPasteableTarget(el: Element | null): boolean {
  if (!el) return false
  if (el instanceof HTMLTextAreaElement) return !el.disabled && !el.readOnly
  if (el instanceof HTMLInputElement) {
    const t = (el as HTMLInputElement).type ?? 'text'
    return !el.disabled && !el.readOnly && (t === 'text' || t === 'password' || t === 'search' || t === '' || t === 'url' || t === 'tel')
  }
  if (el instanceof HTMLElement && el.isContentEditable) return true
  return false
}

function SectionLabel({ children }: { children: string }) {
  return (
    <div className="px-2.5 pt-1.5 pb-0.5 text-[10px] font-semibold uppercase tracking-wider text-koma-dim">
      {children}
    </div>
  )
}

function MenuItem({
  icon,
  onClick,
  disabled,
  children,
}: {
  icon: React.ReactNode
  onClick: () => void
  disabled?: boolean
  children: React.ReactNode
}) {
  return (
    <button
      type="button"
      role="menuitem"
      onClick={onClick}
      disabled={disabled}
      className={`flex w-full items-center gap-2 px-2.5 py-1.5 text-left text-[12px] transition-colors ${
        disabled
          ? 'cursor-not-allowed text-koma-dim opacity-40'
          : 'text-koma-fg opacity-80 hover:bg-koma-hover hover:opacity-100'
      }`}
    >
      {icon}
      <span className="min-w-0 flex-1 truncate">{children}</span>
    </button>
  )
}

function Separator() {
  return <div className="my-1 border-t border-koma-border" />
}

export function GlobalContextMenu({ onResume, hidden }: Props) {
  const [open, setOpen] = useState(false)
  const [rawPos, setRawPos] = useState<Pos>({ left: 0, top: 0 })
  // The element that was right-clicked — stored so actions target it, not
  // whatever gains focus after the menu opens.
  const [target, setTarget] = useState<ContextTarget | null>(null)
  const menuRef = useRef<HTMLDivElement>(null)
  const pos = useClampedPos(rawPos, menuRef)

  // Can the current target receive a paste?
  const canPaste = isPasteableTarget(target?.element ?? null)
  // Is there a non-empty selection to copy?
  const canCopy = target?.copyText != null

  const close = useCallback(() => setOpen(false), [])

  // --- DevTools: Ctrl+Shift+I always; F12 outside Monaco (Monaco owns F12 go-to) ---
  useEffect(() => {
    if (hidden) return
    const onKey = (e: KeyboardEvent) => {
      const isInspectChord =
        (e.ctrlKey || e.metaKey) && e.shiftKey && (e.key === 'I' || e.key === 'i')
      const isF12 = e.key === 'F12' && !e.ctrlKey && !e.metaKey && !e.altKey && !e.shiftKey
      if (!isInspectChord && !isF12) return
      if (isF12 && isMonacoTarget(e.target)) return
      e.preventDefault()
      e.stopPropagation()
      openDevTools()
    }
    window.addEventListener('keydown', onKey, true)
    return () => window.removeEventListener('keydown', onKey, true)
  }, [hidden])

  // --- Open on right-click (bubbling-phase, respects defaultPrevented) ---
  useEffect(() => {
    if (hidden) return
    const onCtx = (e: MouseEvent) => {
      // If another handler already claimed the event (GraphContextMenu,
      // Monaco, extension content, etc.) respect that ownership.
      if (e.defaultPrevented) return
      const eventTarget = e.target
      let element = eventTarget instanceof Element
        ? eventTarget
        : eventTarget instanceof Node
          ? eventTarget.parentElement
          : null
      if (!element) return

      // Use the editable host rather than a clicked descendant so a saved
      // selection elsewhere in the same editor remains valid.
      if (!(element instanceof HTMLTextAreaElement) && !(element instanceof HTMLInputElement)) {
        let editable = element instanceof HTMLElement ? element : element.parentElement
        if (editable?.isContentEditable) {
          while (editable.parentElement?.isContentEditable) editable = editable.parentElement
          element = editable
        }
      }

      e.preventDefault()
      let inputSelection: ContextTarget['inputSelection'] = null
      let contentRange: Range | null = null
      let copyText: string | null = null
      if (element instanceof HTMLTextAreaElement || element instanceof HTMLInputElement) {
        inputSelection = {
          start: element.selectionStart ?? element.value.length,
          end: element.selectionEnd ?? element.value.length,
        }
        if (inputSelection.start !== inputSelection.end) {
          copyText = element.value.slice(inputSelection.start, inputSelection.end)
        }
      } else {
        const selection = window.getSelection()
        if (element instanceof HTMLElement && element.isContentEditable && selection && selection.rangeCount > 0) {
          const range = selection.getRangeAt(0)
          if (element.contains(range.startContainer)) {
            contentRange = range.cloneRange()
            copyText = contentRange.toString() || null
          }
        } else {
          copyText = selection?.toString() || null
        }
      }
      setTarget({ element, inputSelection, contentRange, copyText })
      setRawPos({ left: e.clientX, top: e.clientY })
      setOpen(true)
    }
    document.addEventListener('contextmenu', onCtx)
    return () => document.removeEventListener('contextmenu', onCtx)
  }, [hidden])

  // --- Close on outside mousedown/pointerdown (capture), Escape, window
  //     blur, resize, or scroll ---
  useEffect(() => {
    if (!open) return
    const onDown = (e: MouseEvent) => {
      if (menuRef.current?.contains(e.target as Node)) return
      close()
    }
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') close()
    }
    const onWinBlur = () => close()
    const onResize = () => close()
    const onScroll = () => close()

    // Capture-phase so any mousedown inside the menu doesn't reach here,
    // but clicks elsewhere always close first.
    window.addEventListener('mousedown', onDown, true)
    window.addEventListener('pointerdown', onDown, true)
    window.addEventListener('keydown', onKey)
    window.addEventListener('blur', onWinBlur)
    window.addEventListener('resize', onResize)
    window.addEventListener('scroll', onScroll, true)
    return () => {
      window.removeEventListener('mousedown', onDown, true)
      window.removeEventListener('pointerdown', onDown, true)
      window.removeEventListener('keydown', onKey)
      window.removeEventListener('blur', onWinBlur)
      window.removeEventListener('resize', onResize)
      window.removeEventListener('scroll', onScroll, true)
    }
  }, [open, close])

  // --- Actions ---
  const handleCopy = useCallback(() => {
    const text = target?.copyText
    if (text) {
      navigator.clipboard.writeText(text).catch(() => {
        // Swallow platform rejection silently (e.g. secure-context, permission).
      })
    }
    close()
  }, [target, close])

  const handlePaste = useCallback(async () => {
    if (!target) { close(); return }
    const { element, inputSelection, contentRange } = target
    let clipText: string | null = null
    try {
      clipText = await navigator.clipboard.readText()
    } catch {
      // Clipboard read unavailable/rejected — close without destroying content.
      close()
      return
    }
    if (clipText == null) { close(); return }

    if (element instanceof HTMLTextAreaElement || element instanceof HTMLInputElement) {
      const start = inputSelection?.start ?? element.value.length
      const end = inputSelection?.end ?? start
      element.focus()
      element.setSelectionRange(start, end)
      element.setRangeText(clipText, start, end, 'end')
      // Dispatch a bubbling input event so React-controlled fields update.
      element.dispatchEvent(new Event('input', { bubbles: true }))
    } else if (element instanceof HTMLElement && element.isContentEditable) {
      element.focus()
      const selection = window.getSelection()
      const range = contentRange && element.contains(contentRange.startContainer) && element.contains(contentRange.endContainer)
        ? contentRange.cloneRange()
        : document.createRange()
      if (!contentRange || !element.contains(contentRange.startContainer) || !element.contains(contentRange.endContainer)) {
        range.selectNodeContents(element)
        range.collapse(false)
      }
      range.deleteContents()
      const textNode = document.createTextNode(clipText)
      range.insertNode(textNode)
      range.setStartAfter(textNode)
      range.collapse(true)
      selection?.removeAllRanges()
      selection?.addRange(range)
      element.dispatchEvent(new InputEvent('input', { bubbles: true, inputType: 'insertFromPaste' }))
    }
    close()
  }, [target, close])

  const handleResume = useCallback(() => {
    close()
    onResume()
  }, [close, onResume])

  const handleInspect = useCallback(() => {
    close()
    openDevTools()
  }, [close])

  if (hidden) return null
  if (!open) return null

  return createPortal(
    <div
      ref={menuRef}
      role="menu"
      aria-label="Context menu"
      style={{ position: 'fixed', left: pos.left, top: pos.top, zIndex: 95 }}
      className="flex w-44 flex-col overflow-hidden rounded-md border border-koma-border bg-koma-panel py-1 shadow-sm"
      onContextMenu={(e) => e.preventDefault()}
    >
      <SectionLabel>Main</SectionLabel>
      <MenuItem
        icon={<Clipboard size={13} />}
        onClick={handleCopy}
        disabled={!canCopy}
      >
        Copy
      </MenuItem>
      <MenuItem
        icon={<ClipboardPaste size={13} />}
        onClick={handlePaste}
        disabled={!canPaste}
      >
        Paste
      </MenuItem>
      <MenuItem icon={<RotateCcw size={13} />} onClick={handleResume}>
        Resume
      </MenuItem>
      <Separator />
      <SectionLabel>Debug</SectionLabel>
      <MenuItem icon={<Bug size={13} />} onClick={handleInspect}>
        Inspect
      </MenuItem>
    </div>,
    document.body,
  )
}
