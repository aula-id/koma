import { useEffect } from 'react'
import { pagePoint } from '../../lib/uiScale'
import { DIAGRAM_PALETTE, ShapePreview, type DiagramPaletteEntry } from './DiagramShapePreview'

/**
 * draw.io's quick shape picker: opened from a hover arrow, a connection dropped on empty
 * canvas, or a double-click on empty canvas. Coordinates are client pixels.
 */
export function DiagramShapePicker({
  x,
  y,
  faint,
  onPick,
  onClose,
}: {
  x: number
  y: number
  /** Opened by hovering an arrow: stays see-through until the pointer reaches it. */
  faint?: boolean
  onPick: (entry: DiagramPaletteEntry) => void
  onClose: () => void
}) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== 'Escape') return
      e.stopPropagation()
      onClose()
    }
    const onDown = (e: PointerEvent) => {
      if (!(e.target as HTMLElement | null)?.closest('[data-diagram-picker]')) onClose()
    }
    window.addEventListener('keydown', onKey, true)
    window.addEventListener('pointerdown', onDown, true)
    return () => {
      window.removeEventListener('keydown', onKey, true)
      window.removeEventListener('pointerdown', onDown, true)
    }
  }, [onClose])
  return (
    <div
      data-diagram-picker=""
      data-diagram-ui=""
      className={`group fixed z-[80] grid grid-cols-5 gap-0.5 rounded border border-koma-border bg-koma-panel p-1 shadow-lg ${
        faint ? 'opacity-40 transition-opacity hover:opacity-100' : ''
      }`}
      style={{ left: pagePoint(x), top: pagePoint(y) }}
      onPointerLeave={faint ? onClose : undefined}
      onContextMenu={(e) => e.preventDefault()}
    >
      {DIAGRAM_PALETTE.map((entry) => (
        <button
          key={entry.id}
          type="button"
          title={entry.label}
          aria-label={entry.label}
          className="flex h-9 w-14 items-center justify-center rounded p-0.5 text-koma-dim hover:bg-koma-hover hover:text-koma-fg"
          onClick={() => onPick(entry)}
        >
          <ShapePreview entry={entry} />
        </button>
      ))}
    </div>
  )
}
