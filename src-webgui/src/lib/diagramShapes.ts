// Shape geometry for diagram kinds. One segment list per kind feeds both the drawn
// SVG path and the sampled outline that ports and hit testing use, so what you see is
// what you can click and connect to. Curves follow draw.io's shapes (mxCloud, mxCylinder,
// DocumentShape) at their default sizes.

export type ShapeKind =
  | 'rect'
  | 'rounded'
  | 'text'
  | 'ellipse'
  | 'diamond'
  | 'triangle'
  | 'stadium'
  | 'process'
  | 'parallelogram'
  | 'trapezoid'
  | 'hexagon'
  | 'cylinder'
  | 'document'
  | 'cloud'

export const SHAPE_KINDS: readonly ShapeKind[] = [
  'rect',
  'rounded',
  'text',
  'ellipse',
  'diamond',
  'triangle',
  'stadium',
  'process',
  'parallelogram',
  'trapezoid',
  'hexagon',
  'cylinder',
  'document',
  'cloud',
]

type Point = { x: number; y: number }
type Segment =
  | { t: 'M'; x: number; y: number }
  | { t: 'L'; x: number; y: number }
  | { t: 'Q'; x1: number; y1: number; x: number; y: number }
  | { t: 'C'; x1: number; y1: number; x2: number; y2: number; x: number; y: number }
  | { t: 'Z' }

/** Cubic handle length for a quarter ellipse. */
const K = 0.5523

function polygon(points: Point[]): Segment[] {
  return [...points.map((point, i) => ({ t: i ? ('L' as const) : ('M' as const), ...point })), { t: 'Z' }]
}

/** Quarter arcs around (cx, cy), clockwise from the left point, as cubics. */
function ellipseArcs(cx: number, cy: number, rx: number, ry: number): Segment[] {
  return [
    { t: 'M', x: cx - rx, y: cy },
    { t: 'C', x1: cx - rx, y1: cy - ry * K, x2: cx - rx * K, y2: cy - ry, x: cx, y: cy - ry },
    { t: 'C', x1: cx + rx * K, y1: cy - ry, x2: cx + rx, y2: cy - ry * K, x: cx + rx, y: cy },
    { t: 'C', x1: cx + rx, y1: cy + ry * K, x2: cx + rx * K, y2: cy + ry, x: cx, y: cy + ry },
    { t: 'C', x1: cx - rx * K, y1: cy + ry, x2: cx - rx, y2: cy + ry * K, x: cx - rx, y: cy },
    { t: 'Z' },
  ]
}

/** Slant of parallelograms, trapezoids and hexagons, capped so tall thin shapes stay sane. */
function slant(w: number, h: number): number {
  return Math.min(w * 0.2, h * 0.6)
}

export function cylinderCap(h: number): number {
  return Math.min(15, h * 0.25)
}

function segments(kind: ShapeKind, w: number, h: number): Segment[] {
  if (kind === 'ellipse') return ellipseArcs(w / 2, h / 2, w / 2, h / 2)
  if (kind === 'diamond') return polygon([{ x: w / 2, y: 0 }, { x: w, y: h / 2 }, { x: w / 2, y: h }, { x: 0, y: h / 2 }])
  if (kind === 'triangle') return polygon([{ x: w / 2, y: 0 }, { x: w, y: h }, { x: 0, y: h }])
  if (kind === 'parallelogram') {
    const s = slant(w, h)
    return polygon([{ x: s, y: 0 }, { x: w, y: 0 }, { x: w - s, y: h }, { x: 0, y: h }])
  }
  if (kind === 'trapezoid') {
    const s = slant(w, h)
    return polygon([{ x: s, y: 0 }, { x: w - s, y: 0 }, { x: w, y: h }, { x: 0, y: h }])
  }
  if (kind === 'hexagon') {
    const s = Math.min(w * 0.25, h * 0.5)
    return polygon([{ x: s, y: 0 }, { x: w - s, y: 0 }, { x: w, y: h / 2 }, { x: w - s, y: h }, { x: s, y: h }, { x: 0, y: h / 2 }])
  }
  if (kind === 'stadium') {
    const r = Math.min(h / 2, w / 2)
    return [
      { t: 'M', x: r, y: 0 },
      { t: 'L', x: w - r, y: 0 },
      { t: 'C', x1: w - r + r * K, y1: 0, x2: w, y2: r - r * K, x: w, y: r },
      { t: 'L', x: w, y: h - r },
      { t: 'C', x1: w, y1: h - r + r * K, x2: w - r + r * K, y2: h, x: w - r, y: h },
      { t: 'L', x: r, y: h },
      { t: 'C', x1: r - r * K, y1: h, x2: 0, y2: h - r + r * K, x: 0, y: h - r },
      { t: 'L', x: 0, y: r },
      { t: 'C', x1: 0, y1: r - r * K, x2: r - r * K, y2: 0, x: r, y: 0 },
      { t: 'Z' },
    ]
  }
  if (kind === 'cylinder') {
    const dy = cylinderCap(h)
    return [
      { t: 'M', x: 0, y: dy },
      { t: 'C', x1: 0, y1: -dy / 3, x2: w, y2: -dy / 3, x: w, y: dy },
      { t: 'L', x: w, y: h - dy },
      { t: 'C', x1: w, y1: h + dy / 3, x2: 0, y2: h + dy / 3, x: 0, y: h - dy },
      { t: 'Z' },
    ]
  }
  if (kind === 'document') {
    const dy = h * 0.3
    return [
      { t: 'M', x: 0, y: 0 },
      { t: 'L', x: w, y: 0 },
      { t: 'L', x: w, y: h - dy / 2 },
      { t: 'Q', x1: (w * 3) / 4, y1: h - dy * 1.4, x: w / 2, y: h - dy / 2 },
      { t: 'Q', x1: w / 4, y1: h + dy * 0.4, x: 0, y: h - dy / 2 },
      { t: 'Z' },
    ]
  }
  if (kind === 'cloud') {
    return [
      { t: 'M', x: 0.25 * w, y: 0.25 * h },
      { t: 'C', x1: 0.05 * w, y1: 0.25 * h, x2: 0, y2: 0.5 * h, x: 0.16 * w, y: 0.55 * h },
      { t: 'C', x1: 0, y1: 0.66 * h, x2: 0.18 * w, y2: 0.9 * h, x: 0.31 * w, y: 0.8 * h },
      { t: 'C', x1: 0.4 * w, y1: h, x2: 0.7 * w, y2: h, x: 0.8 * w, y: 0.8 * h },
      { t: 'C', x1: w, y1: 0.8 * h, x2: w, y2: 0.6 * h, x: 0.875 * w, y: 0.5 * h },
      { t: 'C', x1: w, y1: 0.3 * h, x2: 0.8 * w, y2: 0.1 * h, x: 0.625 * w, y: 0.2 * h },
      { t: 'C', x1: 0.5 * w, y1: 0.05 * h, x2: 0.3 * w, y2: 0.05 * h, x: 0.25 * w, y: 0.25 * h },
      { t: 'Z' },
    ]
  }
  return polygon([{ x: 0, y: 0 }, { x: w, y: 0 }, { x: w, y: h }, { x: 0, y: h }])
}

function round(n: number): number {
  return Math.round(n * 100) / 100
}

/** Corner radius of a rounded rectangle (draw.io arcSize 10%-ish, capped). */
export function roundedRadius(w: number, h: number): number {
  return Math.min(10, w / 4, h / 4)
}

/** Fill-and-stroke outline in a w×h box. */
export function shapePath(kind: ShapeKind, w: number, h: number): string {
  if (kind === 'rounded') {
    const r = roundedRadius(w, h)
    return `M${r} 0 H${round(w - r)} Q${w} 0 ${w} ${r} V${round(h - r)} Q${w} ${h} ${round(w - r)} ${h} H${r} Q0 ${h} 0 ${round(h - r)} V${r} Q0 0 ${r} 0 Z`
  }
  return segments(kind, w, h)
    .map((seg) => {
      if (seg.t === 'Z') return 'Z'
      if (seg.t === 'Q') return `Q${round(seg.x1)} ${round(seg.y1)} ${round(seg.x)} ${round(seg.y)}`
      if (seg.t === 'C') return `C${round(seg.x1)} ${round(seg.y1)} ${round(seg.x2)} ${round(seg.y2)} ${round(seg.x)} ${round(seg.y)}`
      return `${seg.t}${round(seg.x)} ${round(seg.y)}`
    })
    .join(' ')
}

/** Inner strokes that are not part of the outline: the cylinder's front rim, the process bars. */
export function shapeDecor(kind: ShapeKind, w: number, h: number): string | null {
  if (kind === 'cylinder') {
    const dy = cylinderCap(h)
    return `M0 ${round(dy)} C0 ${round(2 * dy)} ${w} ${round(2 * dy)} ${w} ${round(dy)}`
  }
  if (kind === 'process') {
    const inset = Math.min(12, w * 0.1)
    return `M${round(inset)} 0 V${h} M${round(w - inset)} 0 V${h}`
  }
  return null
}

const outlines = new Map<string, Point[]>()

/** The outline as a closed polygon, curves sampled. Cached per kind and size. */
export function shapeOutline(kind: ShapeKind, w: number, h: number): Point[] {
  const key = `${kind}:${w}:${h}`
  const hit = outlines.get(key)
  if (hit) return hit
  const points: Point[] = []
  let at: Point = { x: 0, y: 0 }
  const steps = 12
  for (const seg of segments(kind === 'rounded' ? 'rect' : kind, w, h)) {
    if (seg.t === 'Z') continue
    if (seg.t === 'M' || seg.t === 'L') {
      at = { x: seg.x, y: seg.y }
      points.push(at)
      continue
    }
    const from = at
    for (let i = 1; i <= steps; i++) {
      const t = i / steps
      const u = 1 - t
      if (seg.t === 'Q') {
        points.push({ x: u * u * from.x + 2 * u * t * seg.x1 + t * t * seg.x, y: u * u * from.y + 2 * u * t * seg.y1 + t * t * seg.y })
      } else {
        points.push({
          x: u * u * u * from.x + 3 * u * u * t * seg.x1 + 3 * u * t * t * seg.x2 + t * t * t * seg.x,
          y: u * u * u * from.y + 3 * u * u * t * seg.y1 + 3 * u * t * t * seg.y2 + t * t * t * seg.y,
        })
      }
    }
    at = { x: seg.x, y: seg.y }
  }
  if (outlines.size > 512) outlines.clear()
  outlines.set(key, points)
  return points
}

/** Point-in-polygon on the sampled outline, in box-local coordinates. */
export function pointInShape(kind: ShapeKind, w: number, h: number, x: number, y: number): boolean {
  if (x < 0 || y < 0 || x > w || y > h) return false
  if (kind === 'rect' || kind === 'rounded' || kind === 'text' || kind === 'process') return true
  if (kind === 'ellipse') return ((x - w / 2) / (w / 2)) ** 2 + ((y - h / 2) / (h / 2)) ** 2 <= 1
  const poly = shapeOutline(kind, w, h)
  let inside = false
  for (let i = 0, j = poly.length - 1; i < poly.length; j = i++) {
    const a = poly[i]
    const b = poly[j]
    if (a.y > y !== b.y > y && x < ((b.x - a.x) * (y - a.y)) / (b.y - a.y) + a.x) inside = !inside
  }
  return inside
}

/**
 * Where a ray from the box center toward `target` (box fractions) leaves the outline,
 * as box fractions. Rect-like kinds keep the target as is, so their ports stay on the box.
 */
export function outlineFraction(kind: ShapeKind, w: number, h: number, target: Point): Point {
  if (kind === 'rect' || kind === 'rounded' || kind === 'text' || kind === 'process' || w <= 0 || h <= 0) return target
  const cx = w / 2
  const cy = h / 2
  const dx = target.x * w - cx
  const dy = target.y * h - cy
  if (!dx && !dy) return target
  if (kind === 'ellipse') {
    // Exact, since ellipses are common and their sampled outline sits slightly inside.
    const scale = 1 / Math.sqrt((dx / cx) ** 2 + (dy / cy) ** 2)
    return { x: (cx + dx * scale) / w, y: (cy + dy * scale) / h }
  }
  const poly = shapeOutline(kind, w, h)
  let best = Infinity
  for (let i = 0; i < poly.length; i++) {
    const a = poly[i]
    const b = poly[(i + 1) % poly.length]
    // Solve center + s * (dx, dy) = a + t * (b - a) for s > 0, 0 <= t <= 1.
    const ex = b.x - a.x
    const ey = b.y - a.y
    const det = dx * -ey - dy * -ex
    if (Math.abs(det) < 1e-9) continue
    const s = ((a.x - cx) * -ey - (a.y - cy) * -ex) / det
    const t = (dx * (a.y - cy) - dy * (a.x - cx)) / det
    if (s > 0 && t >= 0 && t <= 1 && s < best) best = s
  }
  if (!Number.isFinite(best)) return target
  return { x: (cx + dx * best) / w, y: (cy + dy * best) / h }
}

export const SHAPE_SIZES: Record<ShapeKind, { w: number; h: number }> = {
  rect: { w: 160, h: 64 },
  rounded: { w: 160, h: 64 },
  text: { w: 120, h: 36 },
  ellipse: { w: 160, h: 64 },
  diamond: { w: 160, h: 64 },
  triangle: { w: 100, h: 80 },
  stadium: { w: 160, h: 56 },
  process: { w: 160, h: 64 },
  parallelogram: { w: 160, h: 64 },
  trapezoid: { w: 160, h: 64 },
  hexagon: { w: 160, h: 72 },
  cylinder: { w: 80, h: 96 },
  document: { w: 160, h: 80 },
  cloud: { w: 160, h: 100 },
}
