import { useEffect, useRef, useState, type RefObject } from 'react'
import { createPortal } from 'react-dom'
import { ChevronDown } from 'lucide-react'
import { useKoma } from '../store/koma'

// Track the trigger button's viewport rect while the menu is open, so the
// menu can render in a body portal (fixed positioning) that no `overflow`
// ancestor can clip. Mirrors panels/form.tsx's `useAnchorRect`.
function useAnchorRect<T extends HTMLElement>(open: boolean, ref: RefObject<T | null>) {
  const [rect, setRect] = useState<DOMRect | null>(null)
  useEffect(() => {
    if (!open) {
      setRect(null)
      return
    }
    const update = () => {
      if (ref.current) setRect(ref.current.getBoundingClientRect())
    }
    update()
    window.addEventListener('scroll', update, true)
    window.addEventListener('resize', update)
    return () => {
      window.removeEventListener('scroll', update, true)
      window.removeEventListener('resize', update)
    }
  }, [open, ref])
  return rect
}

type NewSessionMenuProps = {
  // Called right after a menu pick fires its req — lets the caller close its
  // own overlay (ResumePalette's onClose), mirroring the primary button.
  afterPick?: () => void
  className?: string
}

// The chevron segment of the split "+ New session" button. Opens a small
// portaled drop menu (reuses the panels/form.tsx Select/Combobox portal
// pattern) with "New session" (keep current cooking, unchanged) and — only
// when a session is currently attached — "New session + close current"
// (NewSession{kill:true}). Returns null when nothing is attached (StartScreen):
// the "close current" option doesn't apply there, so no split renders at all.
export function NewSessionMenu({ afterPick, className = '' }: NewSessionMenuProps) {
  const req = useKoma((s) => s.req)
  const attachedId = useKoma((s) => s.session.id)
  const [open, setOpen] = useState(false)
  const ref = useRef<HTMLButtonElement>(null)
  const menuRef = useRef<HTMLDivElement>(null)
  const rect = useAnchorRect(open, ref)

  useEffect(() => {
    if (!open) return
    const onDoc = (e: MouseEvent) => {
      const t = e.target as Node
      if (ref.current?.contains(t) || menuRef.current?.contains(t)) return
      setOpen(false)
    }
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setOpen(false)
    }
    window.addEventListener('mousedown', onDoc)
    window.addEventListener('keydown', onKey)
    return () => {
      window.removeEventListener('mousedown', onDoc)
      window.removeEventListener('keydown', onKey)
    }
  }, [open])

  if (!attachedId) return null

  const pick = (kill: boolean) => {
    req(kill ? { r: 'NewSession', kill: true } : { r: 'NewSession' })
    setOpen(false)
    afterPick?.()
  }

  const menuWidth = 208

  return (
    <span className={`flex flex-none items-center ${className}`}>
      <span className="mx-1.5 h-3 w-px flex-none bg-koma-border" />
      <button
        ref={ref}
        onClick={(e) => {
          e.stopPropagation()
          setOpen((o) => !o)
        }}
        aria-label="New session options"
        title="New session options"
        className="flex-none rounded p-0.5 text-koma-fg opacity-60 transition-opacity hover:opacity-100"
      >
        <ChevronDown size={12} className="flex-none" />
      </button>
      {open &&
        rect &&
        createPortal(
          <div
            ref={menuRef}
            style={{
              position: 'fixed',
              top: rect.bottom + 4,
              left: Math.max(8, rect.right - menuWidth),
              width: menuWidth,
              zIndex: 80,
            }}
            className="overflow-hidden rounded-md border border-koma-border bg-koma-panel py-1 shadow-sm"
          >
            <button
              onMouseDown={(e) => {
                e.preventDefault()
                pick(false)
              }}
              className="flex w-full items-center px-2.5 py-1.5 text-left text-[12px] text-koma-fg opacity-80 transition-colors hover:bg-koma-hover hover:opacity-100"
            >
              New session
            </button>
            <button
              onMouseDown={(e) => {
                e.preventDefault()
                pick(true)
              }}
              className="flex w-full items-center px-2.5 py-1.5 text-left text-[12px] text-koma-fg opacity-80 transition-colors hover:bg-koma-hover hover:opacity-100"
            >
              New session + close current
            </button>
          </div>,
          document.body,
        )}
    </span>
  )
}
