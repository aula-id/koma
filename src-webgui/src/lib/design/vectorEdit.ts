// Path editing on a vector network, modelled on Penpot's path editor
// (frontend/src/app/main/data/workspace/path/{tools,edition,helpers}.cljs).
// Every function is pure: it takes a vector in node-local coordinates and returns a new
// one. Callers run `fitVectorNode` afterwards to keep the node box tight.
import type { DesignVector, DesignVectorPoint, DesignVectorRegion, DesignVectorSegment } from './types'

type Point = DesignVectorPoint
export type VectorNodeType = 'corner' | 'mirror' | 'aligned' | 'independent'

const ZERO: Point = { x: 0, y: 0 }
const EPS = 1e-6

const add = (a: Point, b: Point): Point => ({ x: a.x + b.x, y: a.y + b.y })
const sub = (a: Point, b: Point): Point => ({ x: a.x - b.x, y: a.y - b.y })
const scale = (a: Point, k: number): Point => ({ x: a.x * k, y: a.y * k })
const length = (a: Point) => Math.hypot(a.x, a.y)
const isZero = (a: Point) => length(a) < EPS
const unit = (a: Point): Point => {
  const len = length(a)
  return len < EPS ? ZERO : { x: a.x / len, y: a.y / len }
}

/** Segments touching vertex `i`, with which end of each segment sits on it. */
export function segmentsAt(vector: DesignVector, i: number): { index: number; end: 'start' | 'end' }[] {
  const found: { index: number; end: 'start' | 'end' }[] = []
  vector.segments.forEach((segment, index) => {
    if (segment.start === i) found.push({ index, end: 'start' })
    if (segment.end === i) found.push({ index, end: 'end' })
  })
  return found
}

function tangentOf(segment: DesignVectorSegment, end: 'start' | 'end'): Point {
  return end === 'start' ? segment.tangentStart : segment.tangentEnd
}

function withTangent(segment: DesignVectorSegment, end: 'start' | 'end', tangent: Point): DesignVectorSegment {
  return end === 'start' ? { ...segment, tangentStart: tangent } : { ...segment, tangentEnd: tangent }
}

/** The vertex at the far end of a segment. */
function across(segment: DesignVectorSegment, end: 'start' | 'end'): number {
  return end === 'start' ? segment.end : segment.start
}

/**
 * Recompute fill regions after an edit. Loops whose segments all survived keep their
 * winding and order; any other closed chain (every vertex on it has two segments)
 * becomes a new loop. A vector that had no regions (an open stroke) stays open.
 */
function rebuildRegions(before: DesignVector, segments: DesignVectorSegment[], vertexCount: number): DesignVectorRegion[] {
  if (!before.regions.length) return []
  const winding = before.regions[0]?.winding ?? 'nonzero'
  const position = new Map(segments.map((segment, index) => [segment, index]))
  const kept: number[][] = []
  const covered = new Set<number>()
  for (const region of before.regions) {
    for (const loop of region.loops) {
      const mapped = loop.map((index) => position.get(before.segments[index] as DesignVectorSegment))
      if (mapped.length && mapped.every((index): index is number => index != null)) {
        kept.push(mapped)
        for (const index of mapped) covered.add(index)
      }
    }
  }
  const degree = new Array(vertexCount).fill(0)
  for (const segment of segments) {
    degree[segment.start] += 1
    degree[segment.end] += 1
  }
  const used = new Set<number>(covered)
  for (let first = 0; first < segments.length; first++) {
    if (used.has(first)) continue
    // Walk the chain from this segment; it is a loop if it returns to its start
    // through vertices that each have exactly two segments.
    const loop: number[] = []
    let index = first
    let at = segments[first].end
    const startVertex = segments[first].start
    let closed = false
    while (!used.has(index) && loop.length <= segments.length) {
      loop.push(index)
      used.add(index)
      if (at === startVertex) {
        closed = true
        break
      }
      if (degree[at] !== 2) break
      const next = segments.findIndex((segment, i) => !used.has(i) && (segment.start === at || segment.end === at))
      if (next < 0) break
      at = segments[next].start === at ? segments[next].end : segments[next].start
      index = next
    }
    // Every segment of a closed chain is reached from any of its segments, so an open
    // walk proves there is no simple loop here and its segments need no second look.
    if (closed && loop.length >= 2) kept.push(loop)
  }
  return kept.length ? [{ winding, loops: kept }] : []
}

/** Rebuild the vector from new vertex and segment lists, dropping unused vertices if asked. */
function rebuild(before: DesignVector, vertices: Point[], segments: DesignVectorSegment[], pruneIsolated = false): DesignVector {
  let nextVertices = vertices
  let nextSegments = segments
  if (pruneIsolated) {
    const used = new Set<number>()
    for (const segment of segments) {
      used.add(segment.start)
      used.add(segment.end)
    }
    if (used.size < vertices.length && used.size > 0) {
      const map = new Map<number, number>()
      nextVertices = []
      vertices.forEach((vertex, index) => {
        if (!used.has(index)) return
        map.set(index, nextVertices.length)
        nextVertices.push(vertex)
      })
      nextSegments = segments.map((segment) => {
        const start = map.get(segment.start) ?? 0
        const end = map.get(segment.end) ?? 0
        return start === segment.start && end === segment.end ? segment : { ...segment, start, end }
      })
      // Keep segment identity for unchanged loops by mapping old objects to new ones.
      const remap = new Map(segments.map((segment, i) => [segment, nextSegments[i] as DesignVectorSegment]))
      before = {
        ...before,
        segments: before.segments.map((segment) => remap.get(segment) ?? segment),
      }
    }
  }
  return { vertices: nextVertices, segments: nextSegments, regions: rebuildRegions(before, nextSegments, nextVertices.length) }
}

/** Corner when it has no handles; mirror or aligned when its two handles are collinear. */
export function nodeType(vector: DesignVector, i: number): VectorNodeType {
  const touching = segmentsAt(vector, i)
  const tangents = touching.map((item) => tangentOf(vector.segments[item.index] as DesignVectorSegment, item.end))
  if (tangents.every(isZero)) return 'corner'
  if (tangents.length !== 2 || tangents.some(isZero)) return 'independent'
  const [a, b] = tangents as [Point, Point]
  const cross = a.x * b.y - a.y * b.x
  const dot = a.x * b.x + a.y * b.y
  if (Math.abs(cross) > 0.01 * length(a) * length(b) || dot >= 0) return 'independent'
  return Math.abs(length(a) - length(b)) < 0.5 ? 'mirror' : 'aligned'
}

/**
 * Apply a node type to vertices. A corner loses its handles. A corner turned into a curve
 * gets a smooth pair along the line between its neighbors (Penpot make-curve), each a third
 * of the way to that neighbor. Mirror evens the pair out; aligned straightens it.
 */
export function setNodeType(vector: DesignVector, ids: readonly number[], type: VectorNodeType): DesignVector {
  let segments = vector.segments.slice()
  for (const i of ids) {
    const vertex = vector.vertices[i]
    if (!vertex) continue
    const touching = segmentsAt({ ...vector, segments }, i)
    if (type === 'corner') {
      for (const item of touching) segments[item.index] = withTangent(segments[item.index] as DesignVectorSegment, item.end, ZERO)
      continue
    }
    if (type === 'independent') continue
    const neighbours = touching.map((item) => vector.vertices[across(segments[item.index] as DesignVectorSegment, item.end)] ?? vertex)
    const current = touching.map((item) => tangentOf(segments[item.index] as DesignVectorSegment, item.end))
    if (touching.length === 1) {
      const toward = sub(neighbours[0] as Point, vertex)
      const tangent = isZero(current[0] as Point) ? scale(toward, 1 / 3) : (current[0] as Point)
      segments[touching[0].index] = withTangent(segments[touching[0].index] as DesignVectorSegment, touching[0].end, tangent)
      continue
    }
    if (touching.length !== 2) continue
    const [a, b] = neighbours as [Point, Point]
    const [ta, tb] = current as [Point, Point]
    // Direction of the smooth pair: from the first handle when there is one, else along the neighbors.
    const dir = !isZero(tb) && !isZero(ta) ? unit(sub(tb, ta)) : !isZero(tb) ? unit(tb) : !isZero(ta) ? scale(unit(ta), -1) : unit(sub(b, a))
    let lenA = isZero(ta) ? length(sub(a, vertex)) / 3 : length(ta)
    let lenB = isZero(tb) ? length(sub(b, vertex)) / 3 : length(tb)
    if (type === 'mirror') {
      const even = (lenA + lenB) / 2
      lenA = even
      lenB = even
    }
    segments[touching[0].index] = withTangent(segments[touching[0].index] as DesignVectorSegment, touching[0].end, scale(dir, -lenA))
    segments[touching[1].index] = withTangent(segments[touching[1].index] as DesignVectorSegment, touching[1].end, scale(dir, lenB))
  }
  segments = segments.map((segment, index) => (segment === vector.segments[index] ? segment : { ...segment }))
  return rebuild(vector, vector.vertices, segments)
}

/** Move vertices; their handles ride along because handles are offsets from their vertex. */
export function moveVertices(vector: DesignVector, ids: readonly number[], dx: number, dy: number): DesignVector {
  if (!ids.length || (!dx && !dy)) return vector
  const moving = new Set(ids)
  return { ...vector, vertices: vector.vertices.map((vertex, i) => (moving.has(i) ? { x: vertex.x + dx, y: vertex.y + dy } : vertex)) }
}

/**
 * Delete vertices. With `heal` (plain Delete), a vertex between exactly two segments is
 * bridged so the path stays whole, keeping the outer handles. Without it (Shift+Delete)
 * every segment touching a deleted vertex goes too.
 */
export function deleteVertices(vector: DesignVector, ids: readonly number[], heal: boolean): DesignVector {
  const removing = new Set(ids.filter((i) => i >= 0 && i < vector.vertices.length))
  if (!removing.size) return vector
  let segments = vector.segments.slice()
  if (heal) {
    for (const i of removing) {
      const touching = segmentsAt({ ...vector, segments }, i)
      if (touching.length !== 2) continue
      const [first, second] = touching as [{ index: number; end: 'start' | 'end' }, { index: number; end: 'start' | 'end' }]
      const s1 = segments[first.index] as DesignVectorSegment
      const s2 = segments[second.index] as DesignVectorSegment
      const a = across(s1, first.end)
      const b = across(s2, second.end)
      if (a === b || removing.has(a) || removing.has(b)) continue
      const bridge: DesignVectorSegment = {
        start: a,
        end: b,
        tangentStart: first.end === 'end' ? s1.tangentStart : s1.tangentEnd,
        tangentEnd: second.end === 'start' ? s2.tangentEnd : s2.tangentStart,
      }
      segments = segments.filter((_, index) => index !== first.index && index !== second.index)
      segments.push(bridge)
    }
  }
  segments = segments.filter((segment) => !removing.has(segment.start) && !removing.has(segment.end))
  const map = new Map<number, number>()
  const vertices: Point[] = []
  vector.vertices.forEach((vertex, i) => {
    if (removing.has(i)) return
    map.set(i, vertices.length)
    vertices.push(vertex)
  })
  const remapped = segments.map((segment) => {
    const start = map.get(segment.start) as number
    const end = map.get(segment.end) as number
    return start === segment.start && end === segment.end ? segment : { ...segment, start, end }
  })
  const identity = new Map(segments.map((segment, i) => [segment, remapped[i] as DesignVectorSegment]))
  const before = { ...vector, segments: vector.segments.map((segment) => identity.get(segment) ?? segment) }
  return rebuild(before, vertices, remapped)
}

/** Remove segments; vertices left with no segment go too. */
export function deleteSegments(vector: DesignVector, ids: readonly number[]): DesignVector {
  const removing = new Set(ids)
  if (!removing.size) return vector
  return rebuild(vector, vector.vertices, vector.segments.filter((_, index) => !removing.has(index)), true)
}

/** J: a straight segment between two vertices that are not already connected. */
export function joinVertices(vector: DesignVector, a: number, b: number): DesignVector {
  if (a === b || !vector.vertices[a] || !vector.vertices[b]) return vector
  if (vector.segments.some((s) => (s.start === a && s.end === b) || (s.start === b && s.end === a))) return vector
  return rebuild(vector, vector.vertices, [...vector.segments, { start: a, end: b, tangentStart: ZERO, tangentEnd: ZERO }])
}

/** Cmd+J: the vertices collapse into one at their average; their segments meet there. */
export function mergeVertices(vector: DesignVector, ids: readonly number[]): DesignVector {
  const merging = [...new Set(ids)].filter((i) => vector.vertices[i]).sort((p, q) => p - q)
  if (merging.length < 2) return vector
  const keep = merging[0] as number
  const center = scale(
    merging.reduce((sum, i) => add(sum, vector.vertices[i] as Point), ZERO),
    1 / merging.length,
  )
  const gone = new Set(merging.slice(1))
  const vertices = vector.vertices.map((vertex, i) => (i === keep ? center : vertex))
  const seen = new Set<string>()
  const segments: DesignVectorSegment[] = []
  for (const segment of vector.segments) {
    const start = gone.has(segment.start) ? keep : segment.start
    const end = gone.has(segment.end) ? keep : segment.end
    if (start === end) continue
    const pair = start < end ? `${start}:${end}` : `${end}:${start}`
    if (seen.has(pair)) continue
    seen.add(pair)
    segments.push(start === segment.start && end === segment.end ? segment : { ...segment, start, end })
  }
  return deleteVertices(rebuild(vector, vertices, segments), [...gone], false)
}

/** K: every segment beyond the first gets its own copy of the vertex, opening the path there. */
export function separateVertices(vector: DesignVector, ids: readonly number[]): DesignVector {
  const vertices = vector.vertices.slice()
  const segments = vector.segments.slice()
  for (const i of ids) {
    const touching = segmentsAt({ ...vector, segments }, i)
    for (const item of touching.slice(1)) {
      const copy = vertices.length
      vertices.push({ ...(vertices[i] as Point) })
      const segment = segments[item.index] as DesignVectorSegment
      segments[item.index] = item.end === 'start' ? { ...segment, start: copy } : { ...segment, end: copy }
    }
  }
  return rebuild(vector, vertices, segments)
}

/** Copies of the vertices and of the segments between them, for Alt-drag and Cmd+D. */
export function duplicateVertices(vector: DesignVector, ids: readonly number[], dx = 0, dy = 0): { vector: DesignVector; ids: number[] } {
  const picked = [...new Set(ids)].filter((i) => vector.vertices[i])
  const map = new Map<number, number>()
  const vertices = vector.vertices.slice()
  for (const i of picked) {
    map.set(i, vertices.length)
    const vertex = vector.vertices[i] as Point
    vertices.push({ x: vertex.x + dx, y: vertex.y + dy })
  }
  const segments = vector.segments.slice()
  for (const segment of vector.segments) {
    const start = map.get(segment.start)
    const end = map.get(segment.end)
    if (start != null && end != null) segments.push({ ...segment, start, end })
  }
  return { vector: rebuild(vector, vertices, segments), ids: picked.map((i) => map.get(i) as number) }
}

function bezier(p0: Point, c1: Point, c2: Point, p3: Point, t: number): Point {
  const u = 1 - t
  return {
    x: u * u * u * p0.x + 3 * u * u * t * c1.x + 3 * u * t * t * c2.x + t * t * t * p3.x,
    y: u * u * u * p0.y + 3 * u * u * t * c1.y + 3 * u * t * t * c2.y + t * t * t * p3.y,
  }
}

/** Absolute control points of a segment. */
export function segmentCurve(vector: DesignVector, index: number): [Point, Point, Point, Point] | null {
  const segment = vector.segments[index]
  const p0 = segment && vector.vertices[segment.start]
  const p3 = segment && vector.vertices[segment.end]
  if (!segment || !p0 || !p3) return null
  return [p0, add(p0, segment.tangentStart), add(p3, segment.tangentEnd), p3]
}

/** Closest point on one segment: its parameter, the point, and the distance. */
export function closestOnSegment(vector: DesignVector, index: number, point: Point): { t: number; point: Point; distance: number } | null {
  const curve = segmentCurve(vector, index)
  if (!curve) return null
  let best = { t: 0, point: curve[0], distance: Infinity }
  const steps = 48
  for (let i = 0; i <= steps; i++) {
    const t = i / steps
    const at = bezier(...curve, t)
    const distance = length(sub(at, point))
    if (distance < best.distance) best = { t, point: at, distance }
  }
  // One refinement pass around the coarse hit.
  for (let i = -8; i <= 8; i++) {
    const t = Math.min(1, Math.max(0, best.t + i / (steps * 8)))
    const at = bezier(...curve, t)
    const distance = length(sub(at, point))
    if (distance < best.distance) best = { t, point: at, distance }
  }
  return best
}

/** The segment nearest to a point within `tolerance`, for clicks on the path outline. */
export function closestSegment(vector: DesignVector, point: Point, tolerance: number): { index: number; t: number; point: Point } | null {
  let found: { index: number; t: number; point: Point; distance: number } | null = null
  vector.segments.forEach((_, index) => {
    const hit = closestOnSegment(vector, index, point)
    if (hit && hit.distance <= tolerance && (!found || hit.distance < found.distance)) found = { index, ...hit }
  })
  return found
}

/** Split a segment at `t` (de Casteljau), keeping its shape. Returns the new vertex index. */
export function splitSegment(vector: DesignVector, index: number, t: number): { vector: DesignVector; vertex: number } {
  const curve = segmentCurve(vector, index)
  const segment = vector.segments[index]
  if (!curve || !segment) return { vector, vertex: -1 }
  const [p0, c1, c2, p3] = curve
  const k = Math.min(1, Math.max(0, t))
  const lerp = (a: Point, b: Point) => ({ x: a.x + (b.x - a.x) * k, y: a.y + (b.y - a.y) * k })
  const p01 = lerp(p0, c1)
  const p12 = lerp(c1, c2)
  const p23 = lerp(c2, p3)
  const p012 = lerp(p01, p12)
  const p123 = lerp(p12, p23)
  const mid = lerp(p012, p123)
  const vertex = vector.vertices.length
  const first: DesignVectorSegment = { start: segment.start, end: vertex, tangentStart: sub(p01, p0), tangentEnd: sub(p012, mid) }
  const second: DesignVectorSegment = { start: vertex, end: segment.end, tangentStart: sub(p123, mid), tangentEnd: sub(p23, p3) }
  const segments = vector.segments.slice()
  segments.splice(index, 1, first)
  segments.push(second)
  // The split segment's loop gets both halves in order.
  const regions = vector.regions.map((region) => ({
    ...region,
    loops: region.loops.map((loop) => loop.flatMap((i) => (i === index ? [index, segments.length - 1] : [i]))),
  }))
  return { vector: { vertices: [...vector.vertices, mid], segments, regions }, vertex }
}

/**
 * Cmd/Ctrl-drag on a segment: bend it so the point at `t` lands on `target`, moving both
 * handles by the same amount. A straight segment bends from handles a third of the way
 * along it (Penpot bend-reference-curve). `base` is the segment as it was when the drag began.
 */
export function bendSegment(vector: DesignVector, index: number, base: DesignVectorSegment, t: number, target: Point): DesignVector {
  const p0 = vector.vertices[base.start]
  const p3 = vector.vertices[base.end]
  if (!p0 || !p3) return vector
  const chord = sub(p3, p0)
  const c1 = isZero(base.tangentStart) ? add(p0, scale(chord, 1 / 3)) : add(p0, base.tangentStart)
  const c2 = isZero(base.tangentEnd) ? add(p3, scale(chord, -1 / 3)) : add(p3, base.tangentEnd)
  const k = Math.min(0.9, Math.max(0.1, t))
  const current = bezier(p0, c1, c2, p3, k)
  const shift = scale(sub(target, current), 1 / (3 * k * (1 - k)))
  const segments = vector.segments.slice()
  segments[index] = { ...base, tangentStart: sub(add(c1, shift), p0), tangentEnd: sub(add(c2, shift), p3) }
  return { ...vector, segments }
}

/** Shift+H / Shift+V: mirror the vertices (and their handles) across their own center. */
export function flipVertices(vector: DesignVector, ids: readonly number[], axis: 'horizontal' | 'vertical'): DesignVector {
  const picked = new Set(ids.filter((i) => vector.vertices[i]))
  if (!picked.size) return vector
  const xs = [...picked].map((i) => (vector.vertices[i] as Point).x)
  const ys = [...picked].map((i) => (vector.vertices[i] as Point).y)
  const cx = (Math.min(...xs) + Math.max(...xs)) / 2
  const cy = (Math.min(...ys) + Math.max(...ys)) / 2
  const flipPoint = (p: Point): Point => (axis === 'horizontal' ? { x: 2 * cx - p.x, y: p.y } : { x: p.x, y: 2 * cy - p.y })
  const flipVector = (v: Point): Point => (axis === 'horizontal' ? { x: -v.x, y: v.y } : { x: v.x, y: -v.y })
  return {
    ...vector,
    vertices: vector.vertices.map((vertex, i) => (picked.has(i) ? flipPoint(vertex) : vertex)),
    segments: vector.segments.map((segment) => {
      const start = picked.has(segment.start)
      const end = picked.has(segment.end)
      if (!start && !end) return segment
      return {
        ...segment,
        tangentStart: start ? flipVector(segment.tangentStart) : segment.tangentStart,
        tangentEnd: end ? flipVector(segment.tangentEnd) : segment.tangentEnd,
      }
    }),
  }
}

/** Shift while drawing: the point keeps its distance from `origin` but snaps to a 45° step. */
export function fixAngle(origin: Point, point: Point): Point {
  const delta = sub(point, origin)
  const distance = length(delta)
  if (distance < EPS) return point
  const step = Math.PI / 4
  const angle = Math.round(Math.atan2(delta.y, delta.x) / step) * step
  return { x: origin.x + Math.cos(angle) * distance, y: origin.y + Math.sin(angle) * distance }
}

/** Snap each axis to the nearest candidate coordinate within `tolerance` (Penpot path snap). */
export function snapToPoints(point: Point, candidates: readonly Point[], tolerance: number): Point {
  let x = point.x
  let y = point.y
  let bestX = tolerance
  let bestY = tolerance
  for (const candidate of candidates) {
    const dx = Math.abs(candidate.x - point.x)
    const dy = Math.abs(candidate.y - point.y)
    if (dx <= bestX) {
      bestX = dx
      x = candidate.x
    }
    if (dy <= bestY) {
      bestY = dy
      y = candidate.y
    }
  }
  return { x, y }
}

/** After a node drag: any moved vertex dropped onto another vertex merges into it. */
export function mergeOnDrop(vector: DesignVector, moved: readonly number[], tolerance: number): DesignVector {
  const movedSet = new Set(moved)
  let next = vector
  for (const i of moved) {
    const vertex = next.vertices[i]
    if (!vertex) continue
    const target = next.vertices.findIndex((other, j) => j !== i && !movedSet.has(j) && length(sub(other, vertex)) <= tolerance)
    if (target < 0) continue
    // Land exactly on the target, then merge (merge averages, so match first).
    next = { ...next, vertices: next.vertices.map((v, j) => (j === i ? next.vertices[target] as Point : v)) }
    next = mergeVertices(next, [target, i])
    break
  }
  return next
}

/** Vertices inside a rectangle (node-local coordinates), for marquee selection. */
export function verticesInRect(vector: DesignVector, rect: { x: number; y: number; w: number; h: number }): number[] {
  const x0 = Math.min(rect.x, rect.x + rect.w)
  const y0 = Math.min(rect.y, rect.y + rect.h)
  const x1 = Math.max(rect.x, rect.x + rect.w)
  const y1 = Math.max(rect.y, rect.y + rect.h)
  return vector.vertices.flatMap((vertex, i) => (vertex.x >= x0 && vertex.x <= x1 && vertex.y >= y0 && vertex.y <= y1 ? [i] : []))
}

export type HandleMode = 'mirror' | 'aligned' | 'independent'

/**
 * Drag one handle to `tip` (node-local). The handle on the other side of the same vertex
 * follows the mode: mirrored, kept straight with its own length (aligned), or left alone.
 */
export function moveHandle(vector: DesignVector, index: number, end: 'start' | 'end', tip: Point, mode: HandleMode): DesignVector {
  const segment = vector.segments[index]
  const vertexIndex = segment ? (end === 'start' ? segment.start : segment.end) : -1
  const vertex = vector.vertices[vertexIndex]
  if (!segment || !vertex) return vector
  const tangent = sub(tip, vertex)
  const segments = vector.segments.slice()
  segments[index] = withTangent(segment, end, tangent)
  if (mode !== 'independent') {
    for (const other of segmentsAt(vector, vertexIndex)) {
      if (other.index === index && other.end === end) continue
      const current = tangentOf(segments[other.index] as DesignVectorSegment, other.end)
      const opposite = mode === 'mirror' ? scale(tangent, -1) : scale(unit(tangent), -length(current))
      segments[other.index] = withTangent(segments[other.index] as DesignVectorSegment, other.end, opposite)
      break
    }
  }
  return { ...vector, segments }
}

/** The handle mode a plain drag keeps: whatever the node already is. */
export function handleModeOf(vector: DesignVector, vertex: number): HandleMode {
  const type = nodeType(vector, vertex)
  return type === 'mirror' ? 'mirror' : type === 'aligned' ? 'aligned' : 'independent'
}

/** Absolute SVG path of one segment, for hit areas and highlights. */
export function segmentSvgPath(vector: DesignVector, index: number): string {
  const curve = segmentCurve(vector, index)
  if (!curve) return ''
  const [p0, c1, c2, p3] = curve
  return `M${p0.x} ${p0.y} C${c1.x} ${c1.y} ${c2.x} ${c2.y} ${p3.x} ${p3.y}`
}

/** Cmd/Ctrl-drag on a node: a smooth handle pair follows the pointer offset `v` (Penpot curve-config). */
export function pullSmoothPair(vector: DesignVector, vertex: number, v: Point): DesignVector {
  const touching = segmentsAt(vector, vertex)
  if (!touching.length) return vector
  const segments = vector.segments.slice()
  for (const item of touching) {
    segments[item.index] = withTangent(segments[item.index] as DesignVectorSegment, item.end, item.end === 'start' ? v : scale(v, -1))
  }
  return { ...vector, segments }
}

/** The vertices and the segments between them, as a standalone piece for the node clipboard. */
export function extractVertices(vector: DesignVector, ids: readonly number[]): DesignVector {
  const picked = [...new Set(ids)].filter((i) => vector.vertices[i])
  const map = new Map(picked.map((i, n) => [i, n]))
  const segments: DesignVectorSegment[] = []
  for (const segment of vector.segments) {
    const start = map.get(segment.start)
    const end = map.get(segment.end)
    if (start != null && end != null) segments.push({ ...segment, start, end })
  }
  return { vertices: picked.map((i) => ({ ...(vector.vertices[i] as Point) })), segments, regions: [] }
}

/** Paste a clipboard piece into the vector, shifted. Returns the new vertex indices. */
export function appendVector(vector: DesignVector, piece: DesignVector, dx: number, dy: number): { vector: DesignVector; ids: number[] } {
  const offset = vector.vertices.length
  const vertices = [...vector.vertices, ...piece.vertices.map((p) => ({ x: p.x + dx, y: p.y + dy }))]
  const segments = [...vector.segments, ...piece.segments.map((s) => ({ ...s, start: s.start + offset, end: s.end + offset }))]
  return { vector: rebuild(vector, vertices, segments), ids: piece.vertices.map((_, i) => offset + i) }
}

/** The point at parameter `t` along a segment. */
export function pointOnSegment(vector: DesignVector, index: number, t: number): Point | null {
  const curve = segmentCurve(vector, index)
  return curve ? bezier(...curve, t) : null
}
