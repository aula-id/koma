import { useEffect, useLayoutEffect, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import { GitPullRequestArrow } from 'lucide-react'

type Props = {
  x: number
  y: number
  branch: string
  targetLabel: string
  onConfirm: () => void
  onCancel: () => void
}

const WIDTH = 230

// The GitKraken-style drag-to-rebase drop confirm (G6) — a small floating
// popover anchored at the drop point, NOT `window.confirm` (mirrors
// GraphContextMenu's own inline-confirm idiom exactly: clamped to the
// viewport, dismissed on outside-click/Esc). Rendered by GraphTab the instant
// a valid (non-no-op) drag-drop lands on a commit row or ref chip; Confirm
// fires `gitRebase(target, branch)`, Cancel/dismiss just clears the pending
// state — no request is ever sent speculatively.
export function RebaseDropConfirm({ x, y, branch, targetLabel, onConfirm, onCancel }: Props) {
  const ref = useRef<HTMLDivElement>(null)
  const [pos, setPos] = useState({ left: x, top: y })

  // Re-measure after mount (the popover's real size depends on the branch/
  // target label lengths) and clamp so it never renders off-screen. Layout
  // effect (not a plain effect) so the clamp runs BEFORE the browser paints —
  // otherwise a drop near a screen edge flashes at the raw, unclamped
  // {x, y} for one frame.
  useLayoutEffect(() => {
    const el = ref.current
    const w = el?.offsetWidth ?? WIDTH
    const h = el?.offsetHeight ?? 90
    setPos({
      left: Math.max(4, Math.min(x, window.innerWidth - w - 4)),
      top: Math.max(4, Math.min(y, window.innerHeight - h - 4)),
    })
  }, [x, y])

  useEffect(() => {
    const onDoc = (e: MouseEvent) => {
      if (ref.current?.contains(e.target as Node)) return
      onCancel()
    }
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onCancel()
    }
    // Capture-phase mousedown so a stray click elsewhere (including starting
    // another drag) always dismisses this first.
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
        <GitPullRequestArrow size={13} className="mt-0.5 flex-none" />
        <span className="min-w-0 text-[11px] leading-snug">
          Rebase <span className="font-semibold">{branch}</span> onto{' '}
          <span className="font-semibold">{targetLabel}</span>? May conflict.
        </span>
      </div>
      <div className="flex items-center gap-1">
        <button
          type="button"
          autoFocus
          onClick={onConfirm}
          className="flex flex-1 items-center justify-center gap-1 rounded bg-koma-accent/15 px-2 py-1 text-[11px] font-semibold text-koma-accent hover:bg-koma-accent/25"
        >
          Rebase
        </button>
        <button
          type="button"
          onClick={onCancel}
          className="flex-1 rounded px-2 py-1 text-[11px] text-koma-fg opacity-70 hover:bg-koma-hover hover:opacity-100"
        >
          Cancel
        </button>
      </div>
    </div>,
    document.body,
  )
}
