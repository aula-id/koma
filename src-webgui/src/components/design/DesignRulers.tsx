import type { ReactNode } from 'react'
import { locateDesign, designLayerName, stackDesign, type DesignDoc } from '../../lib/design'
import type { DesignMenuItem } from '../DesignMenu'
import { SELECTION } from './tabShared'

type View = { panX: number; panY: number; zoom: number }

export function canvasBackdrop(snap: boolean, grid: number, view: View): { backgroundImage?: string; backgroundSize?: string; backgroundPosition?: string } {
  if (view.zoom >= 8) {
    const cell = view.zoom
    return {
      backgroundImage: 'linear-gradient(to right, var(--color-koma-border) 1px, transparent 1px), linear-gradient(to bottom, var(--color-koma-border) 1px, transparent 1px)',
      backgroundSize: `${cell}px ${cell}px`,
      backgroundPosition: `${view.panX}px ${view.panY}px`,
    }
  }
  if (!snap) return {}
  return {
    backgroundImage: 'radial-gradient(circle, var(--color-koma-border) 1px, transparent 1px)',
    backgroundSize: `${grid * view.zoom}px ${grid * view.zoom}px`,
    backgroundPosition: `${view.panX}px ${view.panY}px`,
  }
}

const RULER_SIZE = 16
const RULER_BADGE_EXCLUSION = 28

export function DesignRulers({ panX, panY, zoom, bounds }: { panX: number; panY: number; zoom: number; bounds: { x: number; y: number; w: number; h: number } | null }) {
  const step = zoom >= 32 ? 10 : zoom >= 8 ? 50 : zoom >= 2 ? 100 : 200
  const ticks = (span: number, origin: number, edge?: number, edge2?: number) => {
    const start = Math.floor(-origin / zoom / step) * step - step
    const items: { at: number; label: number }[] = []
    for (let value = start; items.length < 80; value += step) {
      items.push({ at: origin + value * zoom, label: value })
    }
    return items.filter((item) => {
      if (item.at <= -40 || item.at >= span + 40) return false
      if (edge != null && (Math.abs(item.at - edge) < RULER_BADGE_EXCLUSION || Math.abs(item.at - (edge2 ?? edge)) < RULER_BADGE_EXCLUSION)) return false
      return true
    })
  }
  const span = 4096
  const screen = bounds
    ? {
        sx1: bounds.x * zoom + panX,
        sx2: (bounds.x + bounds.w) * zoom + panX,
        sy1: bounds.y * zoom + panY,
        sy2: (bounds.y + bounds.h) * zoom + panY,
        wx1: Math.round(bounds.x),
        wx2: Math.round(bounds.x + bounds.w),
        wy1: Math.round(bounds.y),
        wy2: Math.round(bounds.y + bounds.h),
      }
    : null
  return (
    <>
      <div className="pointer-events-none absolute inset-x-0 top-0 z-20 h-4 overflow-hidden border-b border-koma-border bg-koma-panel/90 text-[9px] text-koma-dim">
        {screen ? (
          <div
            className="absolute top-0 h-4 bg-[color-mix(in_srgb,var(--koma-accent)_30%,transparent)]"
            style={{ left: Math.max(RULER_SIZE, screen.sx1), width: Math.max(0, screen.sx2 - Math.max(RULER_SIZE, screen.sx1)) }}
          />
        ) : null}
        {ticks(span, panX, screen?.sx1, screen?.sx2).map((tick) => (
          <span key={`x${tick.label}`} className="absolute top-0 h-4 overflow-hidden pl-0.5 leading-4" style={{ left: tick.at }}>{tick.label}</span>
        ))}
        {screen ? (
          <>
            <RulerBadge axis="horizontal" label={String(screen.wx1)} at={Math.max(RULER_SIZE, screen.sx1)} />
            <RulerBadge axis="horizontal" label={String(screen.wx2)} at={screen.sx2} />
          </>
        ) : null}
      </div>
      <div className="pointer-events-none absolute inset-y-0 left-0 z-20 w-4 overflow-hidden border-r border-koma-border bg-koma-panel/90 text-[9px] text-koma-dim">
        {screen ? (
          <div
            className="absolute left-0 w-4 bg-[color-mix(in_srgb,var(--koma-accent)_30%,transparent)]"
            style={{ top: Math.max(RULER_SIZE, screen.sy1), height: Math.max(0, screen.sy2 - Math.max(RULER_SIZE, screen.sy1)) }}
          />
        ) : null}
        {ticks(span, panY, screen?.sy1, screen?.sy2).map((tick) => (
          <span key={`y${tick.label}`} className="absolute left-0 w-4 overflow-hidden pl-px leading-3" style={{ top: tick.at }}>{tick.label}</span>
        ))}
        {screen ? (
          <>
            <RulerBadge axis="vertical" label={String(screen.wy1)} at={Math.max(RULER_SIZE, screen.sy1)} />
            <RulerBadge axis="vertical" label={String(screen.wy2)} at={screen.sy2} />
          </>
        ) : null}
      </div>
      <div className="pointer-events-none absolute left-0 top-0 z-30 h-4 w-4 border-b border-r border-koma-border bg-koma-panel" />
    </>
  )
}

function RulerBadge({ axis, label, at }: { axis: 'horizontal' | 'vertical'; label: string; at: number }) {
  if (axis === 'horizontal') {
    return (
      <span
        className="absolute top-0.5 z-10 -translate-x-1/2 rounded px-1 py-px text-[9px] font-medium leading-none text-white"
        style={{ left: at, background: SELECTION }}
      >
        {label}
      </span>
    )
  }
  return (
    <span
      className="absolute z-10 rounded px-1 py-px text-[9px] font-medium leading-none text-white"
      style={{ left: RULER_SIZE / 2, top: at, background: SELECTION, transform: 'translate(-50%, -50%) rotate(-90deg)' }}
    >
      {label}
    </span>
  )
}

export function designMenuItems(doc: DesignDoc, selection: string[], x: number, y: number, canPaste: boolean, canPasteStyle: boolean, canComponent: boolean): DesignMenuItem[] {
  const selected = selection.length > 0
  const rows = selection.map((id) => locateDesign(doc, id))
  const sameParent = rows.length > 0 && rows.every((row) => row && row.parentId === rows[0]?.parentId)
  const canUngroup = rows.some((row) => row?.node.kind === 'group')
  const stack = stackDesign(doc, x, y).slice(0, 12)
  return [
    { id: 'copy', label: 'Copy', shortcut: '⌘C', disabled: !sameParent },
    { id: 'paste', label: 'Paste', shortcut: '⌘V', disabled: !canPaste },
    { id: 'paste-here', label: 'Paste here', disabled: !canPaste },
    { id: 'duplicate', label: 'Duplicate', shortcut: '⌘D', disabled: !selected },
    { id: 'copy-style', label: 'Copy style', shortcut: '⌥⌘C', disabled: !selected },
    { id: 'paste-style', label: 'Paste style', shortcut: '⌥⌘V', disabled: !canPasteStyle || !selected },
    { id: 'divider-1', label: '' },
    { id: 'select-layer', label: 'Select layer', disabled: stack.length === 0, children: stack.map((node) => ({ id: `select:${node.id}`, label: designLayerName(node) })) },
    { id: 'divider-2', label: '' },
    { id: 'front', label: 'Bring to front', shortcut: '⌘]', disabled: !selected },
    { id: 'forward', label: 'Bring forward', shortcut: ']', disabled: !selected },
    { id: 'backward', label: 'Send backward', shortcut: '[', disabled: !selected },
    { id: 'back', label: 'Send to back', shortcut: '⌘[', disabled: !selected },
    { id: 'divider-3', label: '' },
    { id: 'group', label: 'Group selection', shortcut: '⌘G', disabled: !sameParent },
    { id: 'ungroup', label: 'Ungroup', shortcut: '⇧⌘G', disabled: !canUngroup },
    { id: 'frame-selection', label: 'Frame selection', shortcut: '⌥⌘G', disabled: !sameParent },
    { id: 'auto', label: 'Add auto layout', disabled: !sameParent },
    { id: 'component', label: 'Create component', disabled: !canComponent },
    { id: 'divider-4', label: '' },
    { id: 'hide', label: 'Show/Hide', shortcut: '⇧⌘H', disabled: !selected },
    { id: 'lock', label: 'Lock/Unlock', disabled: !selected },
    { id: 'flip-x', label: 'Flip horizontal', shortcut: '⇧H', disabled: !selected },
    { id: 'flip-y', label: 'Flip vertical', shortcut: '⇧V', disabled: !selected },
    { id: 'divider-5', label: '' },
    { id: 'delete', label: 'Delete', shortcut: 'Del', danger: true, disabled: !selected },
  ]
}

export function ToolButton({ label, selected, onClick, children }: { label: string; selected: boolean; onClick: () => void; children: ReactNode }) {
  return (
    <button
      type="button"
      title={label}
      aria-label={label}
      aria-pressed={selected}
      onClick={onClick}
      className={`flex h-6 w-6 items-center justify-center rounded ${selected ? 'bg-koma-accent/20 text-koma-accent ring-1 ring-inset ring-koma-accent' : 'text-koma-dim hover:bg-koma-hover hover:text-koma-fg'}`}
    >
      {children}
    </button>
  )
}
