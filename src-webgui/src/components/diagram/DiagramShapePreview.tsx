import { SHAPE_SIZES } from '../../lib/diagramShapes'
import type { DiagramKind } from '../../lib/diagram'
import { DiagramShapeSvg } from './DiagramShapeSvg'

export type DiagramPaletteEntry = {
  /** Stable id carried by drag-and-drop. */
  id: string
  kind: DiagramKind
  label: string
  group: 'Basic' | 'Flowchart' | 'Containers' | 'Database'
  /** Size override for presets such as the circle. */
  size?: { w: number; h: number }
  /** Tables only: a document collection instead of a SQL table. */
  dialect?: 'document'
}

/** Shapes offered by the sidebar palette and by the canvas shape picker, in order. */
export const DIAGRAM_PALETTE: DiagramPaletteEntry[] = [
  { id: 'rect', kind: 'rect', label: 'Rectangle', group: 'Basic' },
  { id: 'rounded', kind: 'rounded', label: 'Rounded rectangle', group: 'Basic' },
  { id: 'text', kind: 'text', label: 'Text', group: 'Basic' },
  { id: 'ellipse', kind: 'ellipse', label: 'Ellipse', group: 'Basic' },
  { id: 'circle', kind: 'ellipse', label: 'Circle', group: 'Basic', size: { w: 80, h: 80 } },
  { id: 'diamond', kind: 'diamond', label: 'Diamond', group: 'Basic' },
  { id: 'triangle', kind: 'triangle', label: 'Triangle', group: 'Basic' },
  { id: 'stadium', kind: 'stadium', label: 'Terminator', group: 'Flowchart' },
  { id: 'process', kind: 'process', label: 'Predefined process', group: 'Flowchart' },
  { id: 'parallelogram', kind: 'parallelogram', label: 'Data (input/output)', group: 'Flowchart' },
  { id: 'trapezoid', kind: 'trapezoid', label: 'Manual operation', group: 'Flowchart' },
  { id: 'hexagon', kind: 'hexagon', label: 'Preparation', group: 'Flowchart' },
  { id: 'cylinder', kind: 'cylinder', label: 'Database', group: 'Flowchart' },
  { id: 'document', kind: 'document', label: 'Document', group: 'Flowchart' },
  { id: 'cloud', kind: 'cloud', label: 'Cloud', group: 'Flowchart' },
  { id: 'group', kind: 'group', label: 'Group', group: 'Containers' },
  { id: 'table', kind: 'table', label: 'Table', group: 'Database' },
  { id: 'collection', kind: 'table', label: 'Collection (document)', group: 'Database', dialect: 'document' },
]

export function paletteEntry(id: string): DiagramPaletteEntry | undefined {
  return DIAGRAM_PALETTE.find((entry) => entry.id === id)
}

/** Tile drawing for one palette entry, fitted into a 48×30 box with the shape's own aspect. */
export function ShapePreview({ entry }: { entry: DiagramPaletteEntry }) {
  if (entry.kind === 'text') {
    return <span className="flex h-8 w-12 items-center justify-center text-[13px] leading-none text-koma-fg/80">Text</span>
  }
  if (entry.kind === 'group') {
    return (
      <svg viewBox="0 0 40 30" width={40} height={30} className="m-auto block" aria-hidden="true">
        <rect x="0.75" y="0.75" width="38.5" height="28.5" fill="var(--color-koma-bg)" stroke="currentColor" strokeWidth="1.25" />
        <path d="M0.75 8 H39.25" fill="none" stroke="currentColor" strokeWidth="1" />
        <rect x="8" y="13" width="16" height="10" fill="none" stroke="currentColor" strokeWidth="1" />
      </svg>
    )
  }
  if (entry.kind === 'table' && entry.dialect === 'document') {
    return (
      <svg viewBox="0 0 40 30" width={40} height={30} className="m-auto block" aria-hidden="true">
        <rect x="0.75" y="0.75" width="38.5" height="28.5" rx="2" fill="var(--color-koma-bg)" stroke="currentColor" strokeWidth="1.25" />
        <path d="M0.75 8.5 H39.25" fill="none" stroke="currentColor" strokeWidth="1" />
        <text x="33" y="6.6" fontSize="5" textAnchor="middle" fill="currentColor">{'{}'}</text>
        <path d="M5 13.5 H24 M5 18.5 H14 M9 23 H28" fill="none" stroke="currentColor" strokeWidth="1" opacity="0.6" />
        <path d="M7 19.5 V25" fill="none" stroke="currentColor" strokeWidth="0.75" opacity="0.6" />
      </svg>
    )
  }
  if (entry.kind === 'table') {
    return (
      <svg viewBox="0 0 40 30" width={40} height={30} className="m-auto block" aria-hidden="true">
        <rect x="0.75" y="0.75" width="38.5" height="28.5" rx="2" fill="var(--color-koma-bg)" stroke="currentColor" strokeWidth="1.25" />
        <path d="M0.75 8.5 H39.25 M9 8.5 V29.25" fill="none" stroke="currentColor" strokeWidth="1" />
        <path d="M12 14.5 H32 M12 20 H28 M12 25.5 H30" fill="none" stroke="currentColor" strokeWidth="1" opacity="0.6" />
      </svg>
    )
  }
  const size = entry.size ?? SHAPE_SIZES[entry.kind]
  const scale = Math.min(46 / size.w, 28 / size.h)
  const w = Math.round(size.w * scale)
  const h = Math.round(size.h * scale)
  return (
    <svg viewBox={`-1 -1 ${w + 2} ${h + 2}`} width={w + 2} height={h + 2} className="m-auto block" aria-hidden="true">
      <DiagramShapeSvg kind={entry.kind} w={w} h={h} fill="var(--color-koma-bg)" stroke="currentColor" strokeWidth={1.25} />
    </svg>
  )
}
