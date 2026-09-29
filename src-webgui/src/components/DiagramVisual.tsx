import { useId, useState } from 'react'
import { ChevronDown, Clipboard, MessageSquarePlus, Shapes } from 'lucide-react'
import {
  arrowHead,
  edgeRoute,
  edgeStyle,
  nodeCenter,
  nodeStyle,
  type DiagramDoc,
  type DiagramNode,
} from '../lib/diagram'
import { diagramViewForMermaid, mermaidTitle, nodeBounds } from '../lib/diagramMermaid'

function dashArray(dash: string, width: number): string | undefined {
  if (dash === 'dashed') return `${Math.max(4, width * 4)} ${Math.max(3, width * 2.5)}`
  if (dash === 'dotted') return `${Math.max(1, width)} ${Math.max(2, width * 1.8)}`
  return undefined
}

/** The drawing people see. The model receives Mermaid, not this picture. */
export function DiagramSketch({ doc }: { doc: DiagramDoc }) {
  const markerBase = useId().replace(/:/g, '')
  const byId = new Map(doc.nodes.map((node) => [node.id, node]))
  let minX = Infinity
  let minY = Infinity
  let maxX = -Infinity
  let maxY = -Infinity
  const grow = (x: number, y: number) => {
    minX = Math.min(minX, x)
    minY = Math.min(minY, y)
    maxX = Math.max(maxX, x)
    maxY = Math.max(maxY, y)
  }
  for (const node of doc.nodes) {
    const bounds = nodeBounds(node)
    grow(bounds.x, bounds.y)
    grow(bounds.x + bounds.w, bounds.y + bounds.h)
  }
  const routes = doc.edges.flatMap((edge) => {
    const from = byId.get(edge.from)
    const to = byId.get(edge.to)
    if (!from || !to || edge.stroke === false) return []
    const points = edgeRoute(from, to, edge)
    for (const point of points) grow(point.x, point.y)
    return [{ edge, points }]
  })
  if (!Number.isFinite(minX) || !Number.isFinite(minY)) return null
  const pad = 16
  const viewBox = `${minX - pad} ${minY - pad} ${Math.max(1, maxX - minX) + pad * 2} ${Math.max(1, maxY - minY) + pad * 2}`
  return (
    <svg viewBox={viewBox} className="h-full w-full" role="img" aria-hidden="true">
      <defs>
        {routes.map(({ edge }, index) => {
          const style = edgeStyle(edge)
          const color = style.color || 'var(--color-koma-fg)'
          const id = `${markerBase}-${index}`
          const marker = (name: 'start' | 'end', kind: 'arrow' | 'open') => {
            const head = arrowHead(name, kind === 'open')
            return (
              <marker id={`${id}-${name}`} markerWidth="8" markerHeight="8" refX={head.refX} refY={head.refY} orient="auto" overflow="visible">
                <path
                  d={head.d}
                  fill={kind === 'open' ? 'none' : color}
                  stroke={kind === 'open' ? color : undefined}
                  strokeWidth={kind === 'open' ? 1.2 : undefined}
                />
              </marker>
            )
          }
          return (
            <g key={edge.id}>
              {style.end === 'none' ? null : marker('end', style.end)}
              {style.start === 'none' ? null : marker('start', style.start)}
            </g>
          )
        })}
      </defs>
      {routes.map(({ edge, points }, index) => {
        const style = edgeStyle(edge)
        const color = style.color || 'var(--color-koma-fg)'
        const id = `${markerBase}-${index}`
        const d = points.map((point, i) => `${i ? 'L' : 'M'}${point.x} ${point.y}`).join(' ')
        return (
          <path
            key={edge.id}
            d={d}
            fill="none"
            stroke={color}
            strokeWidth={style.width}
            strokeDasharray={dashArray(style.dash, style.width)}
            markerEnd={style.end === 'none' ? undefined : `url(#${id}-end)`}
            markerStart={style.start === 'none' ? undefined : `url(#${id}-start)`}
          />
        )
      })}
      {doc.nodes.map((node) => (
        <DiagramShape key={node.id} node={node} />
      ))}
    </svg>
  )
}

function shapePaint(node: DiagramNode): { fill: string; stroke: string; strokeWidth: number } {
  const style = nodeStyle(node)
  return {
    fill: style.fill ? style.fillColor || 'var(--color-koma-panel)' : 'none',
    stroke: style.stroke ? style.strokeColor || 'var(--color-koma-border)' : 'none',
    strokeWidth: style.stroke ? 1 : 0,
  }
}

function DiagramShape({ node }: { node: DiagramNode }) {
  const center = nodeCenter(node)
  const transform = node.rotation ? `rotate(${node.rotation} ${center.x} ${center.y})` : undefined
  const paint = shapePaint(node)
  const label = (
    <text
      x={center.x}
      y={center.y}
      textAnchor="middle"
      dominantBaseline="middle"
      fontSize="12"
      fill="var(--color-koma-fg)"
    >
      {node.text}
    </text>
  )
  if (node.kind === 'text') {
    const style = nodeStyle(node)
    return (
      <g transform={transform}>
        {style.fill || style.stroke ? (
          <rect x={node.x} y={node.y} width={node.w} height={node.h} fill={paint.fill} stroke={paint.stroke} strokeWidth={paint.strokeWidth} />
        ) : null}
        {label}
      </g>
    )
  }
  if (node.kind === 'ellipse') {
    return (
      <g transform={transform}>
        <ellipse cx={center.x} cy={center.y} rx={node.w / 2} ry={node.h / 2} fill={paint.fill} stroke={paint.stroke} strokeWidth={paint.strokeWidth} />
        {label}
      </g>
    )
  }
  if (node.kind === 'diamond') {
    const points = `${center.x},${node.y} ${node.x + node.w},${center.y} ${center.x},${node.y + node.h} ${node.x},${center.y}`
    return (
      <g transform={transform}>
        <polygon points={points} fill={paint.fill} stroke={paint.stroke} strokeWidth={paint.strokeWidth} />
        {label}
      </g>
    )
  }
  return (
    <g transform={transform}>
      <rect x={node.x} y={node.y} width={node.w} height={node.h} rx="4" fill={paint.fill} stroke={paint.stroke} strokeWidth={paint.strokeWidth} />
      {label}
    </g>
  )
}

export function DiagramObservationCard({ mermaid }: { mermaid: string }) {
  const [expanded, setExpanded] = useState(false)
  const view = diagramViewForMermaid(mermaid)
  const title = mermaidTitle(mermaid)
  return (
    <article className="overflow-hidden rounded-lg border border-koma-border bg-koma-panel/40 text-koma-fg">
      <button type="button" aria-expanded={expanded} onClick={() => setExpanded((open) => !open)} className="flex w-full items-center gap-3 p-3 text-left hover:bg-koma-hover">
        <div className="flex h-12 w-20 shrink-0 items-center justify-center overflow-hidden rounded border border-koma-border bg-koma-bg">
          {view.doc.nodes.length ? <DiagramSketch doc={view.doc} /> : <Shapes size={20} className="text-koma-dim" />}
        </div>
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-1.5 text-[10px] text-koma-dim">
            <Shapes size={12} />
            <span>Diagram</span>
          </div>
          <p className="mt-1 truncate text-[12.5px]">{title}</p>
        </div>
        <ChevronDown size={14} className={`shrink-0 text-koma-dim transition-transform ${expanded ? 'rotate-180' : ''}`} />
      </button>
      {expanded && view.doc.nodes.length ? (
        <div className="h-52 border-t border-koma-border bg-koma-bg">
          <DiagramSketch doc={view.doc} />
        </div>
      ) : null}
    </article>
  )
}

const menuItem = 'flex w-full items-center gap-2 px-2.5 py-1.5 text-left text-[12px] text-koma-fg opacity-90 hover:bg-koma-hover'

export function DiagramRefMenuItems({ onAdd, onCopy }: { onAdd: () => void; onCopy: () => void }) {
  return (
    <>
      <button type="button" className={menuItem} onClick={onAdd}>
        <MessageSquarePlus size={12} className="opacity-70" />
        Add to chat
      </button>
      <button type="button" className={menuItem} onClick={onCopy}>
        <Clipboard size={12} className="opacity-70" />
        Copy mermaid
      </button>
    </>
  )
}
