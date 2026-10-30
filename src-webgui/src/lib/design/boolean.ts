// Real boolean geometry. Lines are cubics with controls on the chord thirds,
// so every piece splits and classifies the same way.
import type { DesignBooleanOp, DesignDoc, DesignNode, DesignVector, DesignVectorPoint } from './types'
import { cornerPixels, findDesignNode, spinToParent } from './model'
import { clonePaint, nodeHasPaint } from './paint'

export type Point = { x: number; y: number }
export type Cubic = [Point, Point, Point, Point]
export type Contour = Cubic[]
export type FillRule = 'nonzero' | 'evenodd'

export type BooleanGeometry = {
  contours: Contour[]
  rule: FillRule
}

const KAPPA = 0.5523
const EPS = 1e-8
const CROSS_PRECISION = 0.01
const PIECE_TOL = 0.1
const JOIN_TOL = 0.5

const cache = new WeakMap<DesignNode, BooleanGeometry>()

function near(a: Point, b: Point, tol = PIECE_TOL): boolean {
  return Math.hypot(a.x - b.x, a.y - b.y) <= tol
}

function lerp(a: Point, b: Point, t: number): Point {
  return { x: a.x + (b.x - a.x) * t, y: a.y + (b.y - a.y) * t }
}

function lineCubic(a: Point, b: Point): Cubic {
  return [a, lerp(a, b, 1 / 3), lerp(a, b, 2 / 3), b]
}

export function bezierAt(cubic: Cubic, t: number): Point {
  const u = 1 - t
  const uu = u * u
  const tt = t * t
  const w0 = uu * u
  const w1 = 3 * uu * t
  const w2 = 3 * u * tt
  const w3 = tt * t
  return {
    x: w0 * cubic[0].x + w1 * cubic[1].x + w2 * cubic[2].x + w3 * cubic[3].x,
    y: w0 * cubic[0].y + w1 * cubic[1].y + w2 * cubic[2].y + w3 * cubic[3].y,
  }
}

function bezierTangent(cubic: Cubic, t: number): Point {
  const u = 1 - t
  const a = { x: cubic[1].x - cubic[0].x, y: cubic[1].y - cubic[0].y }
  const b = { x: cubic[2].x - cubic[1].x, y: cubic[2].y - cubic[1].y }
  const c = { x: cubic[3].x - cubic[2].x, y: cubic[3].y - cubic[2].y }
  return {
    x: 3 * u * u * a.x + 6 * u * t * b.x + 3 * t * t * c.x,
    y: 3 * u * u * a.y + 6 * u * t * b.y + 3 * t * t * c.y,
  }
}

export function splitAt(cubic: Cubic, t: number): [Cubic, Cubic] {
  const [p0, p1, p2, p3] = cubic
  const a = lerp(p0, p1, t)
  const b = lerp(p1, p2, t)
  const c = lerp(p2, p3, t)
  const d = lerp(a, b, t)
  const e = lerp(b, c, t)
  const f = lerp(d, e, t)
  return [[p0, a, d, f], [f, e, c, p3]]
}

function almost(n: number): boolean {
  return Math.abs(n) < 1e-9
}

/** Roots of d·t³ + a·t² + b·t + c = 0. */
function solveRoots(a: number, b: number, c: number, d: number): number[] {
  if (almost(d) && almost(a) && almost(b)) return []
  if (almost(d) && almost(a)) return almost(b) ? [] : [-c / b]
  if (almost(d)) {
    if (almost(a)) return almost(b) ? [] : [-c / b]
    const disc = b * b - 4 * a * c
    if (disc < 0) return []
    const s = Math.sqrt(disc)
    return [(-b + s) / (2 * a), (-b - s) / (2 * a)]
  }
  const A = a / d
  const B = b / d
  const C = c / d
  const p = (3 * B - A * A) / 3
  const q = (2 * A * A * A - 9 * A * B + 27 * C) / 27
  const p3 = p / 3
  const q2 = q / 2
  const disc = q2 * q2 + p3 * p3 * p3
  const shift = A / 3
  if (disc < -EPS) {
    const mp3 = -p / 3
    const r = Math.sqrt(Math.max(0, mp3 * mp3 * mp3))
    let cosphi = r === 0 ? 0 : -q / (2 * r)
    if (cosphi < -1) cosphi = -1
    if (cosphi > 1) cosphi = 1
    const phi = Math.acos(cosphi)
    const cr = Math.cbrt(r)
    const t1 = 2 * cr
    return [t1 * Math.cos(phi / 3) - shift, t1 * Math.cos((phi + 2 * Math.PI) / 3) - shift, t1 * Math.cos((phi + 4 * Math.PI) / 3) - shift]
  }
  if (Math.abs(disc) <= EPS) {
    const u1 = q2 < 0 ? Math.cbrt(-q2) : -Math.cbrt(q2)
    return [2 * u1 - shift, -u1 - shift]
  }
  const sd = Math.sqrt(disc)
  const u1 = Math.cbrt(-sd - q2)
  const v1 = Math.cbrt(sd + q2)
  return [-u1 - v1 - shift]
}

function bezierCoordRoots(c0: number, c1: number, c2: number, c3: number, target: number): number[] {
  const d = -c0 + 3 * c1 - 3 * c2 + c3
  const a = 3 * c0 - 6 * c1 + 3 * c2
  const b = -3 * c0 + 3 * c1
  const c = c0 - target
  const roots: number[] = []
  for (const t of solveRoots(a, b, c, d)) {
    if (Number.isFinite(t) && t >= -1e-6 && t <= 1 + 1e-6) roots.push(Math.min(1, Math.max(0, t)))
  }
  return roots
}

function derivativeRoots(c0: number, c1: number, c2: number, c3: number): number[] {
  const a = -3 * c0 + 9 * c1 - 9 * c2 + 3 * c3
  const b = 6 * c0 - 12 * c1 + 6 * c2
  const c = 3 * c1 - 3 * c0
  return solveRoots(a, b, c, 0).filter((t) => Number.isFinite(t) && t > 0.01 && t < 0.99)
}

type Box = { x: number; y: number; w: number; h: number }

export function cubicBounds(cubic: Cubic): Box {
  const xs = [cubic[0].x, cubic[3].x]
  const ys = [cubic[0].y, cubic[3].y]
  for (const t of derivativeRoots(cubic[0].x, cubic[1].x, cubic[2].x, cubic[3].x)) {
    xs.push(bezierAt(cubic, t).x)
  }
  for (const t of derivativeRoots(cubic[0].y, cubic[1].y, cubic[2].y, cubic[3].y)) {
    ys.push(bezierAt(cubic, t).y)
  }
  const minX = Math.min(...xs)
  const minY = Math.min(...ys)
  return { x: minX, y: minY, w: Math.max(...xs) - minX, h: Math.max(...ys) - minY }
}

function boxesOverlap(a: Box, b: Box, pad = 0): boolean {
  return a.x <= b.x + b.w + pad && a.x + a.w + pad >= b.x && a.y <= b.y + b.h + pad && a.y + a.h + pad >= b.y
}

function asLine(cubic: Cubic): [Point, Point] | null {
  const chord1 = lerp(cubic[0], cubic[3], 1 / 3)
  const chord2 = lerp(cubic[0], cubic[3], 2 / 3)
  if (near(cubic[1], chord1, 0.05) && near(cubic[2], chord2, 0.05)) return [cubic[0], cubic[3]]
  return null
}

function lineLine(a0: Point, a1: Point, b0: Point, b1: Point): { t1: number; t2: number } | null {
  const dxa = a1.x - a0.x
  const dya = a1.y - a0.y
  const dxb = b1.x - b0.x
  const dyb = b1.y - b0.y
  const den = dxa * dyb - dya * dxb
  if (Math.abs(den) < 1e-9) return null
  const t1 = ((b0.x - a0.x) * dyb - (b0.y - a0.y) * dxb) / den
  const t2 = ((b0.x - a0.x) * dya - (b0.y - a0.y) * dxa) / den
  if (t1 < -1e-5 || t1 > 1 + 1e-5 || t2 < -1e-5 || t2 > 1 + 1e-5) return null
  return { t1: Math.min(1, Math.max(0, t1)), t2: Math.min(1, Math.max(0, t2)) }
}

function lineCurve(line: [Point, Point], curve: Cubic): { t1: number; t2: number }[] {
  const [a, b] = line
  const dx = b.x - a.x
  const dy = b.y - a.y
  const len = Math.hypot(dx, dy)
  if (len < 1e-9) return []
  const cos = dx / len
  const sin = dy / len
  const map = (p: Point): Point => {
    const x = p.x - a.x
    const y = p.y - a.y
    return { x: x * cos + y * sin, y: -x * sin + y * cos }
  }
  const mapped: Cubic = [map(curve[0]), map(curve[1]), map(curve[2]), map(curve[3])]
  const hits: { t1: number; t2: number }[] = []
  for (const raw of bezierCoordRoots(mapped[0].y, mapped[1].y, mapped[2].y, mapped[3].y, 0)) {
    const point = bezierAt(mapped, raw)
    if (point.x < -CROSS_PRECISION || point.x > len + CROSS_PRECISION) continue
    if (Math.abs(point.y) > 0.05) continue
    hits.push({ t1: Math.min(1, Math.max(0, point.x / len)), t2: raw })
  }
  return hits
}

function curveCurve(c1: Cubic, c2: Cubic): { t1: number; t2: number }[] {
  const found: { t1: number; t2: number; d: number }[] = []
  const walk = (a: Cubic, b: Cubic, t1a: number, t1b: number, t2a: number, t2b: number, depth: number) => {
    const boxA = cubicBounds(a)
    const boxB = cubicBounds(b)
    if (!boxesOverlap(boxA, boxB, CROSS_PRECISION)) return
    const startA = a[0]
    const startB = b[0]
    const dist = Math.hypot(startA.x - startB.x, startA.y - startB.y)
    const span = Math.max(boxA.w, boxA.h, boxB.w, boxB.h)
    if (dist <= CROSS_PRECISION || span <= CROSS_PRECISION || depth > 24) {
      if (dist <= Math.max(CROSS_PRECISION * 4, span)) found.push({ t1: t1a, t2: t2a, d: dist })
      return
    }
    const [aL, aR] = splitAt(a, 0.5)
    const [bL, bR] = splitAt(b, 0.5)
    const t1m = (t1a + t1b) / 2
    const t2m = (t2a + t2b) / 2
    walk(aL, bL, t1a, t1m, t2a, t2m, depth + 1)
    walk(aL, bR, t1a, t1m, t2m, t2b, depth + 1)
    walk(aR, bL, t1m, t1b, t2a, t2m, depth + 1)
    walk(aR, bR, t1m, t1b, t2m, t2b, depth + 1)
  }
  walk(c1, c2, 0, 1, 0, 1, 0)
  found.sort((p, q) => p.d - q.d)
  const kept: { t1: number; t2: number }[] = []
  for (const hit of found) {
    const point = bezierAt(c1, hit.t1)
    if (kept.some((other) => near(bezierAt(c1, other.t1), point, 0.5))) continue
    kept.push({ t1: hit.t1, t2: hit.t2 })
  }
  return kept
}

export function cubicCrossings(c1: Cubic, c2: Cubic): { t1: number; t2: number }[] {
  const line1 = asLine(c1)
  const line2 = asLine(c2)
  if (line1 && line2) {
    const hit = lineLine(line1[0], line1[1], line2[0], line2[1])
    return hit ? [hit] : []
  }
  if (line1) return lineCurve(line1, c2)
  if (line2) return lineCurve(line2, c1).map((hit) => ({ t1: hit.t2, t2: hit.t1 }))
  return curveCurve(c1, c2)
}

function splitAtTs(cubic: Cubic, times: number[]): Cubic[] {
  const cuts = [...new Set(times.filter((t) => t > 1e-4 && t < 1 - 1e-4))].sort((a, b) => a - b)
  if (!cuts.length) return [cubic]
  const parts: Cubic[] = []
  let rest = cubic
  let prev = 0
  for (const t of cuts) {
    const local = (t - prev) / (1 - prev)
    if (local <= 1e-5 || local >= 1 - 1e-5) continue
    const [left, right] = splitAt(rest, local)
    parts.push(left)
    rest = right
    prev = t
  }
  parts.push(rest)
  return parts.filter((part) => !near(part[0], part[3], 0.05))
}

function splitPieces(pieces: Cubic[], other: Cubic[]): Cubic[] {
  const out: Cubic[] = []
  for (const piece of pieces) {
    const box = cubicBounds(piece)
    const times: number[] = []
    for (const item of other) {
      if (!boxesOverlap(box, cubicBounds(item), 0.01)) continue
      for (const hit of cubicCrossings(piece, item)) times.push(hit.t1)
    }
    out.push(...splitAtTs(piece, times))
  }
  return out
}

function contourBounds(contours: Contour[]): Box | null {
  let minX = Infinity
  let minY = Infinity
  let maxX = -Infinity
  let maxY = -Infinity
  for (const contour of contours) {
    for (const cubic of contour) {
      const box = cubicBounds(cubic)
      minX = Math.min(minX, box.x)
      minY = Math.min(minY, box.y)
      maxX = Math.max(maxX, box.x + box.w)
      maxY = Math.max(maxY, box.y + box.h)
    }
  }
  if (!Number.isFinite(minX)) return null
  return { x: minX, y: minY, w: maxX - minX, h: maxY - minY }
}

function rayCrossings(point: Point, contours: Contour[]): { count: number; winding: number } {
  let count = 0
  let winding = 0
  for (const contour of contours) {
    for (const cubic of contour) {
      const box = cubicBounds(cubic)
      if (point.y < box.y - 0.01 || point.y > box.y + box.h + 0.01) continue
      if (point.x > box.x + box.w + 0.01) continue
      // A vertex is shared by two cubics. Count it on the cubic that starts there, so a ray through a circle's rightmost point still crosses once.
      const seen: number[] = []
      for (const t of bezierCoordRoots(cubic[0].y, cubic[1].y, cubic[2].y, cubic[3].y, point.y)) {
        if (t < -1e-5 || t >= 1 - 1e-6) continue
        if (seen.some((prev) => Math.abs(prev - t) < 1e-4)) continue
        seen.push(t)
        const at = bezierAt(cubic, t)
        if (at.x <= point.x + 1e-4) continue
        const tangent = bezierTangent(cubic, t)
        if (Math.abs(tangent.y) < 1e-6) continue
        count += 1
        winding += tangent.y > 0 ? -1 : 1
      }
    }
  }
  return { count, winding }
}

export function windingAt(point: Point, contours: Contour[]): number {
  return rayCrossings(point, contours).winding
}

function onCubic(point: Point, cubic: Cubic): boolean {
  const walk = (c: Cubic, depth: number): boolean => {
    const box = cubicBounds(c)
    if (point.x < box.x - PIECE_TOL || point.x > box.x + box.w + PIECE_TOL || point.y < box.y - PIECE_TOL || point.y > box.y + box.h + PIECE_TOL) return false
    if (depth > 12 || (box.w < PIECE_TOL && box.h < PIECE_TOL)) return Math.hypot(bezierAt(c, 0.5).x - point.x, bezierAt(c, 0.5).y - point.y) <= PIECE_TOL
    const [left, right] = splitAt(c, 0.5)
    return walk(left, depth + 1) || walk(right, depth + 1)
  }
  return walk(cubic, 0)
}

export function onBorder(point: Point, contours: Contour[]): boolean {
  for (const contour of contours) {
    for (const cubic of contour) if (onCubic(point, cubic)) return true
  }
  return false
}

function insidePoint(point: Point, contours: Contour[]): boolean {
  const box = contourBounds(contours)
  if (box && (point.x < box.x || point.x > box.x + box.w || point.y < box.y || point.y > box.y + box.h)) return false
  return windingAt(point, contours) !== 0
}

function containsPoint(point: Point, contours: Contour[]): boolean {
  return insidePoint(point, contours) || onBorder(point, contours)
}

export function pointInContours(point: Point, contours: Contour[], rule: FillRule): boolean {
  const box = contourBounds(contours)
  if (!box || point.x < box.x - 0.5 || point.x > box.x + box.w + 0.5 || point.y < box.y - 0.5 || point.y > box.y + box.h + 0.5) return false
  const ray = rayCrossings(point, contours)
  if (rule === 'evenodd') return ray.count % 2 === 1
  return ray.winding !== 0
}

export function samePiece(a: Cubic, b: Cubic): boolean {
  const forward = near(a[0], b[0]) && near(a[3], b[3]) && near(a[1], b[1]) && near(a[2], b[2])
  const back = near(a[0], b[3]) && near(a[3], b[0]) && near(a[1], b[2]) && near(a[2], b[1])
  return forward || back
}

function overlapsAny(piece: Cubic, pieces: Cubic[]): boolean {
  return pieces.some((other) => samePiece(piece, other))
}

function signedArea(contours: Contour[]): number {
  let area = 0
  for (const contour of contours) {
    for (const cubic of contour) {
      const steps = asLine(cubic) ? 1 : 8
      let prev = cubic[0]
      for (let i = 1; i <= steps; i++) {
        const next = i === steps ? cubic[3] : bezierAt(cubic, i / steps)
        area += prev.x * next.y - next.x * prev.y
        prev = next
      }
    }
  }
  return area / 2
}

function clockwise(contours: Contour[]): boolean {
  return signedArea(contours) > 0
}

function reverseCubic(cubic: Cubic): Cubic {
  return [cubic[3], cubic[2], cubic[1], cubic[0]]
}

function reverseContours(contours: Contour[]): Contour[] {
  return contours.map((contour) => contour.slice().reverse().map(reverseCubic))
}

function flattenContours(contours: Contour[]): Cubic[] {
  return contours.flat()
}

type Chain = { cubics: Cubic[] }

function chainStart(chain: Chain): Point {
  return chain.cubics[0][0]
}

function chainEnd(chain: Chain): Point {
  return chain.cubics[chain.cubics.length - 1][3]
}

function reverseChain(chain: Chain): Chain {
  return { cubics: chain.cubics.slice().reverse().map(reverseCubic) }
}

function closedChain(chain: Chain): boolean {
  return near(chainStart(chain), chainEnd(chain), JOIN_TOL)
}

function stitch(pieces: Cubic[]): Contour[] {
  let chains: Chain[] = pieces.filter((piece) => !near(piece[0], piece[3], 0.05)).map((piece) => ({ cubics: [piece] }))
  let moved = true
  while (moved) {
    moved = false
    for (let i = 0; i < chains.length && !moved; i++) {
      if (closedChain(chains[i])) continue
      for (let j = 0; j < chains.length; j++) {
        if (i === j || closedChain(chains[j])) continue
        const a = chains[i]
        const b = chains[j]
        let next: Chain | null = null
        if (near(chainEnd(a), chainStart(b), JOIN_TOL)) next = { cubics: [...a.cubics, ...b.cubics] }
        else if (near(chainEnd(b), chainStart(a), JOIN_TOL)) next = { cubics: [...b.cubics, ...a.cubics] }
        else if (near(chainEnd(a), chainEnd(b), JOIN_TOL)) next = { cubics: [...a.cubics, ...reverseChain(b).cubics] }
        else if (near(chainStart(a), chainStart(b), JOIN_TOL)) next = { cubics: [...reverseChain(b).cubics, ...a.cubics] }
        if (!next) continue
        chains = chains.filter((_, index) => index !== i && index !== j)
        chains.push(next)
        moved = true
        break
      }
    }
  }
  const contours: Contour[] = []
  for (const chain of chains) {
    if (!chain.cubics.length) continue
    const cubics = chain.cubics.slice()
    const first = cubics[0][0]
    const last = cubics[cubics.length - 1]
    if (near(first, last[3], JOIN_TOL) && !near(first, last[3], 0.01)) cubics[cubics.length - 1] = [last[0], last[1], last[2], first]
    if (Math.abs(signedArea([cubics])) < 0.5) continue
    contours.push(cubics)
  }
  return contours
}

function chainInOrder(pieces: Cubic[]): Contour[] {
  const contours: Contour[] = []
  let current: Cubic[] = []
  const flush = () => {
    if (!current.length) return
    const cubics = current
    current = []
    const first = cubics[0][0]
    const last = cubics[cubics.length - 1]
    if (near(first, last[3], JOIN_TOL) && !near(first, last[3], 0.01)) cubics[cubics.length - 1] = [last[0], last[1], last[2], first]
    if (Math.abs(signedArea([cubics])) < 0.5 && cubics.length < 2) return
    contours.push(cubics)
  }
  for (const piece of pieces) {
    if (near(piece[0], piece[3], 0.05)) continue
    if (current.length && !near(current[current.length - 1][3], piece[0], JOIN_TOL)) flush()
    current.push(piece)
  }
  flush()
  return contours
}

function pieceInside(piece: Cubic, contours: Contour[]): boolean {
  return insidePoint(bezierAt(piece, 0.5), contours)
}

function pieceContains(piece: Cubic, contours: Contour[]): boolean {
  return containsPoint(bezierAt(piece, 0.5), contours)
}

function dedupe(pieces: Cubic[]): Cubic[] {
  const kept: Cubic[] = []
  for (const piece of pieces) {
    if (kept.some((other) => samePiece(piece, other))) continue
    kept.push(piece)
  }
  return kept
}

export function booleanPair(op: DesignBooleanOp, a: Contour[], b: Contour[]): Contour[] {
  if (!a.length) return op === 'subtract' || op === 'intersect' ? [] : b
  if (!b.length) return op === 'intersect' ? [] : a
  const boxA = contourBounds(a)
  const boxB = contourBounds(b)
  if (boxA && boxB && !boxesOverlap(boxA, boxB, 0.01)) {
    if (op === 'intersect') return []
    if (op === 'subtract') return a
    return [...a, ...b]
  }
  const sameTurn = clockwise(a) === clockwise(b)
  const nextB = op !== 'union' && sameTurn ? reverseContours(b) : b
  const aPieces = flattenContours(a)
  const bPieces = flattenContours(nextB)
  const aSplit = splitPieces(aPieces, bPieces)
  const bSplit = splitPieces(bPieces, aPieces)
  if (op === 'exclude') return [...chainInOrder(aSplit), ...chainInOrder(bSplit)]
  let kept: Cubic[]
  if (op === 'union') {
    const body = [...aSplit.filter((piece) => !pieceContains(piece, nextB)), ...bSplit.filter((piece) => !pieceContains(piece, a))]
    const border = bSplit.filter((piece) => pieceContains(piece, a) && overlapsAny(piece, aSplit) && !pieceInside(piece, stitch(body)))
    kept = dedupe([...body, ...border])
  } else if (op === 'subtract') {
    kept = dedupe([
      ...aSplit.filter((piece) => !pieceContains(piece, nextB)),
      ...bSplit.filter((piece) => pieceContains(piece, a) && !overlapsAny(piece, aSplit)),
    ])
  } else {
    kept = dedupe([...aSplit.filter((piece) => pieceContains(piece, nextB)), ...bSplit.filter((piece) => pieceContains(piece, a))])
  }
  return stitch(kept)
}

/** Twice the signed area of a closed contour, from its endpoints and three points along each curve. */
function contourArea(contour: Contour): number {
  const points: Point[] = []
  for (const cubic of contour) {
    points.push(cubic[0], bezierAt(cubic, 0.25), bezierAt(cubic, 0.5), bezierAt(cubic, 0.75))
  }
  let sum = 0
  for (let i = 0; i < points.length; i++) {
    const a = points[i]
    const b = points[(i + 1) % points.length]
    sum += a.x * b.y - b.x * a.y
  }
  return sum
}

/** Slivers a boolean leaves behind: a lone edge shared by two shapes, or a contour with no area. */
function dropDegenerate(contours: Contour[]): Contour[] {
  return contours.filter((contour) => contour.length >= 2 && Math.abs(contourArea(contour)) / 2 >= 0.5)
}

export function booleanContours(op: DesignBooleanOp, lists: Contour[][]): BooleanGeometry {
  const rule: FillRule = op === 'exclude' ? 'evenodd' : 'nonzero'
  const ready = lists.filter((item) => item.length)
  if (!ready.length) return { contours: [], rule }
  let acc = ready[0]
  for (let index = 1; index < ready.length; index++) acc = dropDegenerate(booleanPair(op, acc, ready[index]))
  return { contours: ready.length > 1 ? acc : dropDegenerate(acc), rule }
}

function arcCubic(cx: number, cy: number, rx: number, ry: number, a0: number, a1: number): Cubic {
  const at = (angle: number) => ({ x: cx + rx * Math.cos(angle), y: cy + ry * Math.sin(angle) })
  const tan = (angle: number) => ({ x: -rx * Math.sin(angle), y: ry * Math.cos(angle) })
  const p0 = at(a0)
  const p3 = at(a1)
  const t0 = tan(a0)
  const t1 = tan(a1)
  return [p0, { x: p0.x + KAPPA * t0.x, y: p0.y + KAPPA * t0.y }, { x: p3.x - KAPPA * t1.x, y: p3.y - KAPPA * t1.y }, p3]
}

function roundedRect(w: number, h: number, corners: { tl: number; tr: number; br: number; bl: number }): Contour {
  const limit = Math.min(w, h) / 2
  const tl = Math.min(Math.max(0, corners.tl), limit)
  const tr = Math.min(Math.max(0, corners.tr), limit)
  const br = Math.min(Math.max(0, corners.br), limit)
  const bl = Math.min(Math.max(0, corners.bl), limit)
  const cubics: Cubic[] = []
  const pushLine = (a: Point, b: Point) => {
    if (!near(a, b, 0.05)) cubics.push(lineCubic(a, b))
  }
  pushLine({ x: tl, y: 0 }, { x: w - tr, y: 0 })
  if (tr > 0) cubics.push(arcCubic(w - tr, tr, tr, tr, -Math.PI / 2, 0))
  pushLine({ x: w, y: tr }, { x: w, y: h - br })
  if (br > 0) cubics.push(arcCubic(w - br, h - br, br, br, 0, Math.PI / 2))
  pushLine({ x: w - br, y: h }, { x: bl, y: h })
  if (bl > 0) cubics.push(arcCubic(bl, h - bl, bl, bl, Math.PI / 2, Math.PI))
  pushLine({ x: 0, y: h - bl }, { x: 0, y: tl })
  if (tl > 0) cubics.push(arcCubic(tl, tl, tl, tl, Math.PI, Math.PI * 1.5))
  return cubics
}

function ellipseContour(w: number, h: number): Contour {
  const rx = w / 2
  const ry = h / 2
  return [0, 1, 2, 3].map((index) => arcCubic(rx, ry, rx, ry, (index * Math.PI) / 2, ((index + 1) * Math.PI) / 2))
}

function segmentCubic(vector: DesignVector, index: number): Cubic | null {
  const segment = vector.segments[index]
  const start = segment ? vector.vertices[segment.start] : null
  const end = segment ? vector.vertices[segment.end] : null
  if (!segment || !start || !end) return null
  return [
    start,
    { x: start.x + segment.tangentStart.x, y: start.y + segment.tangentStart.y },
    { x: end.x + segment.tangentEnd.x, y: end.y + segment.tangentEnd.y },
    end,
  ]
}

function closeChain(cubics: Cubic[]): Contour | null {
  if (cubics.length < 2 && !(cubics.length === 1 && !asLine(cubics[0]))) return cubics.length >= 3 ? cubics : null
  const first = cubics[0][0]
  const last = cubics[cubics.length - 1][3]
  if (!near(first, last, 0.5)) cubics = [...cubics, lineCubic(last, first)]
  return cubics.length >= 1 ? cubics : null
}

function vectorContours(vector: DesignVector): Contour[] {
  if (vector.regions.length) {
    const contours: Contour[] = []
    for (const region of vector.regions) {
      for (const loop of region.loops) {
        const cubics = loop.flatMap((index) => {
          const cubic = segmentCubic(vector, index)
          return cubic ? [cubic] : []
        })
        const closed = closeChain(cubics)
        if (closed) contours.push(closed)
      }
    }
    return contours
  }
  const used = new Set<number>()
  const outgoing = new Map<number, number[]>()
  vector.segments.forEach((segment, index) => {
    const list = outgoing.get(segment.start) ?? []
    list.push(index)
    outgoing.set(segment.start, list)
  })
  const contours: Contour[] = []
  for (let index = 0; index < vector.segments.length; index++) {
    if (used.has(index)) continue
    const cubics: Cubic[] = []
    let cursor = index
    while (cursor >= 0 && !used.has(cursor)) {
      used.add(cursor)
      const cubic = segmentCubic(vector, cursor)
      if (cubic) cubics.push(cubic)
      const segment = vector.segments[cursor]
      const followers = segment ? outgoing.get(segment.end) : undefined
      let next = -1
      if (followers) {
        for (const item of followers) {
          if (!used.has(item)) {
            next = item
            break
          }
        }
      }
      cursor = next
    }
    const closed = closeChain(cubics)
    if (closed) contours.push(closed)
  }
  return contours
}

function transformContour(node: DesignNode, contour: Contour): Contour {
  return contour.map((cubic) => cubic.map((point) => spinToParent(node, point.x, point.y)) as Cubic)
}

function contoursInContent(doc: DesignDoc, node: DesignNode): Contour[] {
  if (node.visible === false) return []
  if (node.booleanOp && node.children?.length) return booleanResult(doc, node).contours
  if (node.kind === 'group') {
    if (!node.children?.length) return []
    return booleanContours('union', node.children.map((child) => nodeContours(doc, child))).contours
  }
  if (node.kind === 'rect' || node.kind === 'frame' || node.kind === 'instance') return [roundedRect(node.w, node.h, cornerPixels(doc, node))]
  if (node.kind === 'ellipse') return [ellipseContour(node.w, node.h)]
  if (node.kind === 'vector' && node.vector) return vectorContours(node.vector)
  return []
}

/** Outlines of a node in its parent's space. Lines and text have no area. */
export function nodeContours(doc: DesignDoc, node: DesignNode): Contour[] {
  return contoursInContent(doc, node).map((contour) => transformContour(node, contour))
}

export function booleanResult(doc: DesignDoc, node: DesignNode): BooleanGeometry {
  const hit = cache.get(node)
  if (hit) return hit
  const op = node.booleanOp ?? 'union'
  const lists = (node.children ?? []).filter((child) => child.visible !== false).map((child) => nodeContours(doc, child))
  const result = booleanContours(op, lists)
  cache.set(node, result)
  return result
}

export function booleanContains(doc: DesignDoc, node: DesignNode, point: Point): boolean {
  const result = booleanResult(doc, node)
  if (!result.contours.length) return false
  return pointInContours(point, result.contours, result.rule)
}

export function isBooleanNode(node: DesignNode | null | undefined): boolean {
  return !!node?.booleanOp && !!node.children?.length
}

/** Text and lines have no closed area to combine, so a boolean cannot take them. */
function hasOpenShape(node: DesignNode): boolean {
  if (node.kind === 'text' || node.kind === 'line') return true
  return (node.children ?? []).some(hasOpenShape)
}

export type BooleanEligibility = { ok: true } | { ok: false; reason: string }

/**
 * Whether the selection can be combined. Two or more shapes combine; one node qualifies
 * only when it is a group or a boolean (to switch or set its operation). Text and lines
 * (also inside groups) block it, as Penpot blocks text.
 */
export function booleanEligible(doc: DesignDoc, ids: readonly string[]): BooleanEligibility {
  const nodes = ids.map((id) => findDesignNode(doc, id)).filter((node): node is DesignNode => !!node)
  if (!nodes.length) return { ok: false, reason: 'Select shapes to combine' }
  if (nodes.length === 1 && nodes[0].kind !== 'group') return { ok: false, reason: 'Select two or more shapes, or a group' }
  if (nodes.some(hasOpenShape)) return { ok: false, reason: "Text and lines can't be combined" }
  return { ok: true }
}

/** The group's own paint, or the back-most child's when the group has none. */
export function booleanPaintNode(node: DesignNode): DesignNode {
  if (!isBooleanNode(node)) return node
  const bottom = node.children?.find((child) => child.visible !== false)
  if (!bottom) return node
  const next: DesignNode = { ...node }
  if (!nodeHasPaint(node, 'fill')) {
    if (bottom.fill != null) next.fill = bottom.fill
    else delete next.fill
    if (bottom.fills?.length) next.fills = bottom.fills.map(clonePaint)
  }
  if (!nodeHasPaint(node, 'stroke')) {
    if (bottom.stroke != null) next.stroke = bottom.stroke
    else delete next.stroke
    if (bottom.strokes?.length) next.strokes = bottom.strokes.map(clonePaint)
    if (bottom.strokeWidth != null) next.strokeWidth = bottom.strokeWidth
  }
  return next
}

export function contoursToVector(contours: Contour[], rule: FillRule): DesignVector {
  const vertices: DesignVectorPoint[] = []
  const segments: DesignVector['segments'] = []
  const loops: number[][] = []
  for (const contour of contours) {
    if (!contour.length) continue
    const base = vertices.length
    for (const cubic of contour) vertices.push({ x: cubic[0].x, y: cubic[0].y })
    const loop: number[] = []
    contour.forEach((cubic, index) => {
      const endIndex = base + ((index + 1) % contour.length)
      const end = vertices[endIndex]
      if (!end) return
      segments.push({
        start: base + index,
        end: endIndex,
        tangentStart: { x: cubic[1].x - cubic[0].x, y: cubic[1].y - cubic[0].y },
        tangentEnd: { x: cubic[2].x - end.x, y: cubic[2].y - end.y },
      })
      loop.push(segments.length - 1)
    })
    if (loop.length) loops.push(loop)
  }
  return { vertices, segments, regions: loops.length ? [{ winding: rule, loops }] : [] }
}

function fmt(value: number): string {
  return String(Math.round(value * 100) / 100)
}

export function contoursSvgPath(contours: Contour[]): string {
  return contours
    .map((contour) => {
      if (!contour.length) return ''
      const head = `M ${fmt(contour[0][0].x)} ${fmt(contour[0][0].y)}`
      const body = contour
        .map((cubic) => `C ${fmt(cubic[1].x)} ${fmt(cubic[1].y)} ${fmt(cubic[2].x)} ${fmt(cubic[2].y)} ${fmt(cubic[3].x)} ${fmt(cubic[3].y)}`)
        .join(' ')
      return `${head} ${body} Z`
    })
    .filter(Boolean)
    .join(' ')
}
