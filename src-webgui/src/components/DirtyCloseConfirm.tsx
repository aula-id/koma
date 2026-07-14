import { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import { FileWarning } from 'lucide-react'

type Props = {
  anchor: HTMLElement | null
  title: string
  onConfirm: () => void
  onCancel: () => void
}

const WIDTH = 230

// Floating dirty-close confirm for Coding file tabs — mirrors RebaseDropConfirm:
// viewport-clamped portal, dismissed on outside-click / Esc, never window.confirm.
export function DirtyCloseConfirm({ anchor, title, onConfirm, onCancel }: Props) {
  const ref = useRef<HTMLDivElement>(null)
  const [pos, setPos] = useState({ left: 4, top: 4 })

  const recalculate = useCallback(() => {
    if (!anchor || !anchor.isConnected) {
      onCancel()
      return
    }
    const rect = anchor.getBoundingClientRect()
    const w = ref.current?.offsetWidth ?? WIDTH
    const h = ref.current?.offsetHeight ?? 90
    setPos({
      left: Math.max(4, Math.min(rect.left, window.innerWidth - w - 4)),
      top: Math.max(4, Math.min(rect.bottom + 4, window.innerHeight - h - 4)),
    })
  }, [anchor, onCancel])

  useLayoutEffect(() => {
    recalculate()
  }, [recalculate])

  useEffect(() => {
    window.addEventListener('resize', recalculate)
    // Scroll events do not bubble, so capture them to track scrolling of the
    // tab strip as well as the window/document.
    window.addEventListener('scroll', recalculate, true)
    return () => {
      window.removeEventListener('resize', recalculate)
      window.removeEventListener('scroll', recalculate, true)
    }
  }, [recalculate])

  useEffect(() => {
    const onDoc = (e: MouseEvent) => {
      if (ref.current?.contains(e.target as Node)) return
      onCancel()
    }
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onCancel()
    }
    window.addEventListener('mousedown', onDoc, true)
    window.addEventListener('keydown', onKey)
    return () => {
      window.removeEventListener('mousedown', onDoc, true)
      window.removeEventListener('keydown', onKey)
    }
  }, [onCancel])

  return createPortal(
    <div
      ref={ref}
      style={{ position: 'fixed', left: pos.left, top: pos.top, width: WIDTH, zIndex: 90 }}
      className="flex flex-col gap-1.5 rounded-md border border-koma-border bg-koma-panel px-2.5 py-2 shadow-sm"
    >
      <div className="flex items-start gap-1.5 text-koma-fg opacity-90">
        <FileWarning size={13} className="mt-0.5 flex-none" />
        <span className="min-w-0 text-[11px] leading-snug">Close without saving?</span>
      </div>
      <div className="flex items-center gap-1">
        <button
          type="button"
          autoFocus
          onClick={onConfirm}
          className="flex flex-1 items-center justify-center gap-1 rounded bg-koma-error/15 px-2 py-1 text-[11px] font-semibold text-koma-error hover:bg-koma-error/25"
        >
          yes
        </button>
        <button
          type="button"
          onClick={onCancel}
          className="flex-1 rounded px-2 py-1 text-[11px] text-koma-fg opacity-70 hover:bg-koma-hover hover:opacity-100"
        >
          no
        </button>
      </div>
    </div>,
    document.body,
  )
}
