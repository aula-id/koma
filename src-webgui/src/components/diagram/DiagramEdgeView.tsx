import { memo } from 'react'
import { edgeStyle, routePath, type DiagramDash, type DiagramEdge, type DiagramPoint } from '../../lib/diagram'
import { DiagramMarker } from '../DiagramVisual'

/** draw.io's default jump size. */
const JUMP_SIZE = 6

export function dashArray(dash: DiagramDash, width: number): string | undefined {
  if (dash === 'dashed') return `${Math.max(4, width * 4)} ${Math.max(3, width * 2.5)}`
  if (dash === 'dotted') return `${Math.max(1, width)} ${Math.max(2, width * 1.8)}`
  return undefined
}

export function themeStroke(selected: boolean): string {
  return selected
    ? 'var(--color-koma-accent)'
    : 'color-mix(in srgb, var(--koma-fg, #c8d3f5) 45%, transparent)'
}

/** One line in doc coordinates, with its own markers so a restyle repaints only this line. */
export const DiagramEdgeView = memo(function DiagramEdgeView({
  edge,
  points,
  selected,
  markerBase,
  crossings,
}: {
  edge: DiagramEdge
  points: DiagramPoint[]
  selected: boolean
  markerBase: string
  /** Per segment, where this line crosses lines drawn before it. Only set when it jumps. */
  crossings?: DiagramPoint[][]
}) {
  const style = edgeStyle(edge)
  if (!style.stroke) return null
  const color = style.color || themeStroke(selected)
  const id = `${markerBase}-${edge.id.replace(/[^a-zA-Z0-9_-]/g, '')}`
  return (
    <g>
      <defs>
        {style.end === 'none' ? null : <DiagramMarker id={`${id}-end`} at="end" kind={style.end} color={color} />}
        {style.start === 'none' ? null : <DiagramMarker id={`${id}-start`} at="start" kind={style.start} color={color} />}
      </defs>
      <path
        d={routePath(points, style.corner === 'rounded', {
          curved: style.route === 'curved',
          jumps: crossings && style.jump !== 'none' ? { style: style.jump, size: JUMP_SIZE, crossings } : undefined,
        })}
        stroke={color}
        strokeWidth={style.width}
        strokeDasharray={dashArray(style.dash, style.width)}
        strokeLinejoin={style.corner === 'rounded' ? 'round' : 'miter'}
        fill="none"
        markerEnd={style.end === 'none' ? undefined : `url(#${id}-end)`}
        markerStart={style.start === 'none' ? undefined : `url(#${id}-start)`}
      />
    </g>
  )
})
