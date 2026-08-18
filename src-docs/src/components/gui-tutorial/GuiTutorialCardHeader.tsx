import { Minus, Plus, type LucideIcon } from 'lucide-react'
import type { ReactNode } from 'react'

type Props = {
  eyebrow: string
  title: ReactNode
  zoom: number
  zoomIndex: number
  zoomLevels: number[]
  onZoomIndex: (index: number) => void
  action?: { label: string; onClick: () => void; icon?: LucideIcon }
  children?: ReactNode
}

/** Shared chrome for every interactive desktop GUI tutorial card. */
export function GuiTutorialCardHeader({
  eyebrow,
  title,
  zoom,
  zoomIndex,
  zoomLevels,
  onZoomIndex,
  action,
  children,
}: Props) {
  return <div className="flex flex-wrap items-center gap-3 border-b border-koma-border bg-koma-panel px-4 py-2.5">
    <span className="text-xs text-koma-dim">{eyebrow}</span>
    <span className="text-xs text-koma-fg">{title}</span>
    {children}
    <div className="ml-auto flex items-center gap-1.5">
      <div className="flex items-center rounded border border-koma-border bg-koma-panel2" aria-label="Stage zoom">
        <button type="button" onClick={() => onZoomIndex(Math.max(0, zoomIndex - 1))} disabled={zoomIndex === 0} aria-label="Zoom out" className="gui-tutorial-control"><Minus size={13}/></button>
        <button type="button" onClick={() => onZoomIndex(1)} aria-label="Reset zoom" className="min-w-11 px-1 text-[11px] text-koma-dim hover:text-koma-fg">{Math.round(zoom * 100)}%</button>
        <button type="button" onClick={() => onZoomIndex(Math.min(zoomLevels.length - 1, zoomIndex + 1))} disabled={zoomIndex === zoomLevels.length - 1} aria-label="Zoom in" className="gui-tutorial-control"><Plus size={13}/></button>
      </div>
      {action && <button type="button" onClick={action.onClick} className="rounded px-2 py-1 text-[11px] font-semibold text-koma-accent hover:bg-koma-hover">{action.label}</button>}
    </div>
  </div>
}
