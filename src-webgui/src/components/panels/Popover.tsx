import { useEffect, useRef, type ReactNode, type RefObject } from 'react'
import { createPortal } from 'react-dom'
import { useAnchorRect } from './form'

/**
 * A small floating menu anchored to its trigger, rendered in a portal so no panel clips it.
 * It opens below the trigger, right-aligned to it, and flips up near the bottom of the window.
 */
export function Popover({
  anchor,
  open,
  onClose,
  width,
  children,
}: {
  anchor: RefObject<HTMLElement | null>
  open: boolean
  onClose: () => void
  width: number
  children: ReactNode
}) {
  const rect = useAnchorRect(open, anchor)
  const menuRef = useRef<HTMLDivElement>(null)
  useEffect(() => {
    if (!open) return
    const onDown = (e: MouseEvent) => {
      const target = e.target as Node
      if (anchor.current?.contains(target) || menuRef.current?.contains(target)) return
      onClose()
    }
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose()
    }
    window.addEventListener('mousedown', onDown)
    window.addEventListener('keydown', onKey)
    return () => {
      window.removeEventListener('mousedown', onDown)
      window.removeEventListener('keydown', onKey)
    }
  }, [anchor, onClose, open])
  if (!open || !rect) return null
  const below = rect.bottom + 260 < window.innerHeight
  return createPortal(
    <div
      ref={menuRef}
      style={{
        position: 'fixed',
        left: Math.max(4, rect.right - width),
        width,
        zIndex: 80,
        ...(below ? { top: rect.bottom + 4 } : { bottom: window.innerHeight - rect.top + 4 }),
      }}
      className="max-h-64 overflow-y-auto rounded-md border border-koma-border bg-koma-panel py-1 shadow-xl"
    >
      {children}
    </div>,
    document.body,
  )
}

/** One row of a Popover. It picks on mouse down so the trigger never loses focus first. */
export function MenuItem({
  children,
  onPick,
  disabled,
  danger,
  active,
  title,
}: {
  children: ReactNode
  onPick: () => void
  disabled?: boolean
  danger?: boolean
  /** The current choice, shown highlighted. */
  active?: boolean
  title?: string
}) {
  return (
    <button
      type="button"
      title={title}
      disabled={disabled}
      onMouseDown={(e) => {
        e.preventDefault()
        if (!disabled) onPick()
      }}
      className={`flex w-full items-center gap-2 px-2.5 py-1 text-left text-[12px] disabled:opacity-35 disabled:hover:bg-transparent ${
        active ? 'bg-koma-accent/20 text-koma-accent' : `hover:bg-koma-hover ${danger ? 'text-koma-error' : 'text-koma-fg'}`
      }`}
    >
      {children}
    </button>
  )
}
