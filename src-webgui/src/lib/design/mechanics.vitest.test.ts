import { describe, expect, it } from 'vitest'
import { svgFillLayers } from './paint'
import { booleanEligible, booleanPaintNode, booleanResult, contoursToVector, nodeContours, pointInContours, type Contour } from './boolean'
import { designChatHtml } from './chatHtml'
import {
  bendSegment,
  closestOnSegment,
  closestSegment,
  deleteVertices,
  duplicateVertices,
  fixAngle,
  flipVertices,
  joinVertices,
  mergeOnDrop,
  mergeVertices,
  moveHandle,
  moveVertices,
  nodeType,
  separateVertices,
  setNodeType,
  snapToPoints,
  splitSegment,
  verticesInRect,
} from './vectorEdit'
import type { DesignVector } from './types'
import {
  createComponentFromFrame,
  designAxisAligned,
  designChildIds,
  designDistances,
  designParentIds,
  designSnapScene,
  designCanvasBox,
  designTurnSign,
  emptyDesign,
  applyDesignBoolean,
  booleanDesignNodes,
  flattenBooleanNode,
  hitDesign,
  hitDesignDeepest,
  nodeChangesToDesign,
  spinToParent,
  layoutDesign,
  makeInstance,
  moveDesignNodes,
  nodeCanvasPoint,
  nodeCenter,
  parseDesign,
  resizeDesignNode,
  resolveInstanceTree,
  sameDesign,
  serializeDesign,
  stepDesignSibling,
  syncLinkedComponents,
  type DesignDoc,
  type DesignHandle,
  type DesignNode,
} from './index'

const HANDLES: DesignHandle[] = ['nw', 'n', 'ne', 'e', 'se', 's', 'sw', 'w']

function docOf(...screens: DesignNode[]): DesignDoc {
  return { ...emptyDesign(), screens }
}

function rect(id: string, extra: Partial<DesignNode> = {}): DesignNode {
  return { id, kind: 'rect', x: 100, y: 100, w: 120, h: 60, ...extra }
}

/** The handle's opposite point in the node's own box. */
function opposite(handle: DesignHandle, w: number, h: number): { x: number; y: number } {
  return {
    x: handle.includes('w') ? w : handle.includes('e') ? 0 : w / 2,
    y: handle.includes('n') ? h : handle.includes('s') ? 0 : h / 2,
  }
}

function canvasPoint(node: DesignNode, x: number, y: number) {
  const point = nodeCanvasPoint(docOf(node), node.id, x, y)
  if (!point) throw new Error('missing node')
  return point
}

describe('resizeDesignNode', () => {
  for (const rotation of [0, 37, 90, 180]) {
    for (const handle of HANDLES) {
      it(`keeps the opposite handle fixed at ${rotation}° from ${handle}`, () => {
        const node = rect('r', rotation ? { rotation } : {})
        const before = opposite(handle, node.w, node.h)
        const pinned = canvasPoint(node, before.x, before.y)
        const resized = resizeDesignNode(node, handle, 30, 20, 1, false)
        const after = opposite(handle, resized.w, resized.h)
        const moved = canvasPoint(resized, after.x, after.y)
        expect(moved.x).toBeCloseTo(pinned.x, 6)
        expect(moved.y).toBeCloseTo(pinned.y, 6)
      })
    }
  }

  it('grows along the node axes, not the parent axes', () => {
    const node = rect('r', { rotation: 90 })
    const resized = resizeDesignNode(node, 'e', 50, 0, 1, false)
    expect(resized.w).toBe(170)
    expect(resized.h).toBe(60)
  })

  it('grows both sides around the center with the center option', () => {
    const node = rect('r', { rotation: 37 })
    const centerBefore = canvasPoint(node, node.w / 2, node.h / 2)
    const resized = resizeDesignNode(node, 'se', 10, 10, 1, false, false, { center: true })
    expect(resized.w).toBe(140)
    expect(resized.h).toBe(80)
    const centerAfter = canvasPoint(resized, resized.w / 2, resized.h / 2)
    expect(centerAfter.x).toBeCloseTo(centerBefore.x, 6)
    expect(centerAfter.y).toBeCloseTo(centerBefore.y, 6)
  })

  it('flips when dragged past the opposite edge', () => {
    const node = rect('r', { w: 100 })
    const resized = resizeDesignNode(node, 'e', -150, 0, 1, false)
    expect(resized.flipX).toBe(true)
    expect(resized.w).toBe(50)
    expect(designCanvasBox(docOf(resized), 'r')).toEqual({ x: 50, y: 100, w: 50, h: 60 })
  })

  it('locks the ratio from an edge handle around the cross-axis center', () => {
    const node = rect('r', { w: 100, h: 50 })
    const resized = resizeDesignNode(node, 'e', 100, 0, 1, false, true)
    expect(resized.w).toBe(200)
    expect(resized.h).toBe(100)
    expect(resized.y).toBe(75)
  })

  it('snaps the moving edge to the grid unless the axis was object-snapped', () => {
    const node = rect('r', { x: 0, y: 0, w: 100, h: 40 })
    expect(resizeDesignNode(node, 'e', 13, 0, 8, true).w).toBe(112)
    expect(resizeDesignNode(node, 'e', 13, 0, 8, true, false, { gridX: false }).w).toBe(113)
    expect(resizeDesignNode(node, 'w', 13, 0, 8, true).x).toBe(16)
  })
})

describe('transform helpers', () => {
  it('finds the canvas center of a rotated node in a rotated parent', () => {
    const child = rect('c', { x: 0, y: 0, w: 40, h: 20, rotation: 45 })
    const parent: DesignNode = { id: 'p', kind: 'frame', x: 0, y: 0, w: 200, h: 100, rotation: 90, children: [child] }
    const doc = docOf(parent)
    // The parent turns its content around (100, 50); the child's own turn keeps its center put.
    const center = nodeCenter(doc, 'c')
    expect(center?.x).toBeCloseTo(140, 6)
    expect(center?.y).toBeCloseTo(-30, 6)
    expect(designAxisAligned(doc, 'c')).toBe(false)
    expect(designAxisAligned(docOf(rect('r')), 'r')).toBe(true)
  })

  it('reverses the turn direction under a flipped ancestor', () => {
    const child = rect('c', { x: 0, y: 0 })
    expect(designTurnSign(docOf({ id: 'p', kind: 'frame', x: 0, y: 0, w: 300, h: 300, flipX: true, children: [child] }), 'c')).toBe(-1)
    expect(designTurnSign(docOf({ id: 'p', kind: 'frame', x: 0, y: 0, w: 300, h: 300, flipX: true, flipY: true, children: [child] }), 'c')).toBe(1)
    expect(designTurnSign(docOf({ ...rect('r'), flipX: true }), 'r')).toBe(1)
  })
})

describe('instances', () => {
  function buttonDoc(): DesignDoc {
    const button: DesignNode = {
      id: 'b', kind: 'frame', x: 0, y: 0, w: 120, h: 40, layout: 'row', justify: 'center', align: 'center',
      children: [{ id: 'label', kind: 'text', x: 0, y: 0, w: 60, h: 20, text: 'OK' }],
    }
    const doc = createComponentFromFrame(docOf(button), 'b', 'cmp', () => 'x')
    if (!doc) throw new Error('component')
    return doc
  }

  it('re-lays out a resized instance instead of capping or scaling it', () => {
    const doc = buttonDoc()
    const instance = makeInstance(doc, 'cmp', 'i', 0, 0)
    if (!instance) throw new Error('instance')
    const wide = resolveInstanceTree(doc, { ...instance, w: 300, wMode: 'fixed' })
    expect(wide?.w).toBe(300)
    expect(wide?.children?.[0]).toMatchObject({ x: 120, w: 60 })
    const placed = layoutDesign({ ...doc, screens: [...doc.screens, { ...instance, w: 300, wMode: 'fixed' }] })
    expect(placed.screens.at(-1)?.w).toBe(300)
  })

  it('reads the old size pin as hug so old files keep following the component', () => {
    const doc = buttonDoc()
    const pinned: DesignNode = { id: 'old', kind: 'instance', x: 0, y: 0, w: 120, h: 40, wMode: 'fixed', hMode: 'fixed', maxW: 120, maxH: 40, component: 'cmp' }
    const parsed = parseDesign(serializeDesign({ ...doc, screens: [pinned] })).doc.screens[0]
    expect(parsed).toMatchObject({ wMode: 'hug', hMode: 'hug' })
    expect(parsed.maxW).toBeUndefined()
  })
})

describe('smoothness helpers', () => {
  it('compares documents by content, reusing shared parts', () => {
    const doc = docOf(rect('a'), rect('b'))
    expect(sameDesign(doc, { ...doc, screens: [...doc.screens] })).toBe(true)
    expect(sameDesign(doc, { ...doc, screens: [rect('a'), { ...rect('b'), x: 1 }] })).toBe(false)
    expect(sameDesign(doc, { ...doc, grid: 4 })).toBe(false)
  })

  it('snaps only to root screens, its own screen, unclipped shapes and the viewport', () => {
    const hidden = rect('hidden', { x: 500, y: 0, w: 20, h: 20 })
    const home: DesignNode = { id: 'home', kind: 'frame', x: 0, y: 0, w: 200, h: 200, children: [rect('mover', { x: 10, y: 10, w: 20, h: 20 }), rect('peer', { x: 100, y: 100, w: 20, h: 20 }), hidden] }
    const away: DesignNode = { id: 'away', kind: 'frame', x: 400, y: 0, w: 200, h: 200, children: [rect('inner', { x: 10, y: 10, w: 20, h: 20 })] }
    const far = rect('far', { x: 5000, y: 5000, w: 10, h: 10 })
    const doc = docOf(home, away, far)
    const scene = designSnapScene(doc, ['mover'], { x: -50, y: -50, w: 800, h: 400 })
    const xs = scene?.targets.map((box) => box.x).sort((a, b) => a - b)
    // home, peer, away; not inner (another screen), hidden (clipped out) or far (off screen).
    expect(xs).toEqual([0, 100, 400])
  })

  it('leaves linked components alone when their screen did not change', () => {
    const doc = createComponentFromFrame(docOf({ id: 'b', kind: 'frame', x: 0, y: 0, w: 80, h: 40 }), 'b', 'cmp', () => 'x')
    if (!doc) throw new Error('component')
    const synced = syncLinkedComponents(doc)
    expect(syncLinkedComponents(synced).components).toBe(synced.components)
  })

  it('caches a resolved instance until its node or component changes', () => {
    const doc = createComponentFromFrame(docOf({ id: 'b', kind: 'frame', x: 0, y: 0, w: 80, h: 40 }), 'b', 'cmp', () => 'x')
    const instance = doc && makeInstance(doc, 'cmp', 'i', 0, 0)
    if (!doc || !instance) throw new Error('instance')
    expect(resolveInstanceTree(doc, instance)).toBe(resolveInstanceTree({ ...doc, screens: [] }, instance))
    expect(resolveInstanceTree(doc, { ...instance })).not.toBe(resolveInstanceTree(doc, instance))
  })
})

describe('selection navigation', () => {
  const group: DesignNode = { id: 'g', kind: 'group', x: 0, y: 0, w: 100, h: 100, children: [rect('a', { x: 0, y: 0, w: 50, h: 50 }), rect('b', { x: 50, y: 50, w: 50, h: 50 })] }
  const screen: DesignNode = { id: 's', kind: 'frame', x: 0, y: 0, w: 300, h: 300, children: [group, rect('c', { x: 200, y: 200, w: 50, h: 50 }), rect('d', { x: 0, y: 200, w: 50, h: 50 })] }
  const doc = docOf(screen)

  it('reaches through groups on a deep click', () => {
    expect(hitDesignDeepest(doc, 60, 60)?.id).toBe('b')
    expect(hitDesignDeepest(doc, 290, 10)?.id).toBe('s')
  })

  it('steps through siblings down the layers list and wraps', () => {
    expect(stepDesignSibling(doc, 'd', 1)).toBe('c')
    expect(stepDesignSibling(doc, 'g', 1)).toBe('d')
    expect(stepDesignSibling(doc, 'g', -1)).toBe('c')
  })

  it('selects children and parents', () => {
    expect(designChildIds(doc, ['g'])).toEqual(['a', 'b'])
    expect(designChildIds(doc, ['c'])).toEqual(['c'])
    expect(designParentIds(doc, ['a', 'b'])).toEqual(['g'])
    expect(designParentIds(doc, ['s'])).toEqual(['s'])
  })

  it('moves several layers next to a row in the order the panel lists them', () => {
    const ids = (d: DesignDoc) => d.screens[0].children?.map((child) => child.id)
    // Panel order is front to back: d, c, g. Dropping [d, c] above g puts them in front, d frontmost.
    expect(ids(moveDesignNodes(doc, ['d', 'c'], 's', 'g', 'before'))).toEqual(['g', 'c', 'd'])
    // Below g sends them behind it, keeping d above c.
    expect(ids(moveDesignNodes(doc, ['d', 'c'], 's', 'g', 'after'))).toEqual(['c', 'd', 'g'])
  })

  it('measures from a box to the edges of the box around it', () => {
    const inner = { x: 10, y: 20, w: 30, h: 40 }
    const outer = { x: 0, y: 0, w: 100, h: 100 }
    expect(designDistances(inner, outer).map((gap) => gap.label)).toEqual(['10', '60', '20', '40'])
  })
})

describe('path editing', () => {
  const zero = { x: 0, y: 0 }
  const square = (): DesignVector => ({
    vertices: [{ x: 0, y: 0 }, { x: 100, y: 0 }, { x: 100, y: 100 }, { x: 0, y: 100 }],
    segments: [0, 1, 2, 3].map((i) => ({ start: i, end: (i + 1) % 4, tangentStart: zero, tangentEnd: zero })),
    regions: [{ winding: 'nonzero', loops: [[0, 1, 2, 3]] }],
  })
  const open = (): DesignVector => ({
    vertices: [{ x: 0, y: 0 }, { x: 50, y: 0 }, { x: 100, y: 0 }],
    segments: [
      { start: 0, end: 1, tangentStart: zero, tangentEnd: zero },
      { start: 1, end: 2, tangentStart: zero, tangentEnd: zero },
    ],
    regions: [],
  })

  it('deleting a corner of a closed shape heals it and keeps it filled', () => {
    const next = deleteVertices(square(), [1], true)
    expect(next.vertices).toHaveLength(3)
    expect(next.segments).toHaveLength(3)
    expect(next.regions[0]?.loops[0]).toHaveLength(3)
    // Shift+Delete removes the segments too, which opens the shape.
    const cut = deleteVertices(square(), [1], false)
    expect(cut.segments).toHaveLength(2)
    expect(cut.regions).toEqual([])
  })

  it('splitting a segment keeps the fill loop whole', () => {
    const { vector, vertex } = splitSegment(square(), 0, 0.5)
    expect(vector.vertices[vertex]).toEqual({ x: 50, y: 0 })
    expect(vector.regions[0]?.loops[0]).toHaveLength(5)
  })

  it('join, merge and separate rewire the network', () => {
    const joined = joinVertices(open(), 0, 2)
    expect(joined.segments).toHaveLength(3)
    const merged = mergeVertices(open(), [1, 2])
    expect(merged.vertices).toHaveLength(2)
    expect(merged.vertices[1]).toEqual({ x: 75, y: 0 })
    expect(merged.segments).toHaveLength(1)
    const split = separateVertices(open(), [1])
    expect(split.vertices).toHaveLength(4)
    expect(new Set(split.segments.flatMap((s) => [s.start, s.end])).size).toBe(4)
  })

  it('make curve and make corner round-trip a node', () => {
    const curved = setNodeType(open(), [1], 'mirror')
    expect(nodeType(curved, 1)).toBe('mirror')
    expect(curved.segments[0].tangentEnd.x).toBeCloseTo(-50 / 3)
    expect(nodeType(setNodeType(curved, [1], 'corner'), 1)).toBe('corner')
  })

  it('bending a straight segment pulls its middle to the pointer', () => {
    const vector = open()
    const bent = bendSegment(vector, 0, vector.segments[0], 0.5, { x: 25, y: 30 })
    const hit = closestOnSegment(bent, 0, { x: 25, y: 30 })
    expect(hit?.distance).toBeLessThan(0.5)
  })

  it('a click near the outline finds the segment and its parameter', () => {
    const hit = closestSegment(square(), { x: 103, y: 40 }, 6)
    expect(hit?.index).toBe(1)
    expect(hit?.t).toBeCloseTo(0.4, 1)
  })

  it('Shift fixes angles to 45° steps and points snap per axis', () => {
    const fixed = fixAngle({ x: 0, y: 0 }, { x: 100, y: 20 })
    expect(fixed.y).toBeCloseTo(0)
    expect(fixed.x).toBeCloseTo(Math.hypot(100, 20))
    expect(snapToPoints({ x: 52, y: 97 }, [{ x: 50, y: 0 }, { x: 0, y: 100 }], 4)).toEqual({ x: 50, y: 100 })
  })

  it('a vertex dropped onto another merges into it', () => {
    const moved = moveVertices(open(), [2], -48, 1)
    const merged = mergeOnDrop(moved, [2], 4)
    expect(merged.vertices).toHaveLength(2)
  })

  it('flip mirrors the selection around its own center', () => {
    const flipped = flipVertices(square(), [0, 1], 'horizontal')
    expect(flipped.vertices[0]).toEqual({ x: 100, y: 0 })
    expect(flipped.vertices[1]).toEqual({ x: 0, y: 0 })
    expect(verticesInRect(square(), { x: -1, y: -1, w: 102, h: 10 })).toEqual([0, 1])
  })

  it('duplicating copies the segments between copied vertices', () => {
    const { vector, ids } = duplicateVertices(open(), [0, 1], 10, 10)
    expect(ids).toEqual([3, 4])
    expect(vector.segments).toHaveLength(3)
  })

  it('handle drags mirror, align or leave the opposite handle', () => {
    const curved = setNodeType(open(), [1], 'mirror')
    const tip = { x: 50 + 10, y: 20 }
    const mirrored = moveHandle(curved, 1, 'start', tip, 'mirror')
    expect(mirrored.segments[0].tangentEnd).toEqual({ x: -10, y: -20 })
    const aligned = moveHandle(curved, 1, 'start', tip, 'aligned')
    expect(Math.hypot(aligned.segments[0].tangentEnd.x, aligned.segments[0].tangentEnd.y)).toBeCloseTo(50 / 3)
    const free = moveHandle(curved, 1, 'start', tip, 'independent')
    expect(free.segments[0].tangentEnd).toEqual(curved.segments[0].tangentEnd)
  })
})

function square(id: string, x: number, y: number, size = 100): DesignNode {
  return { id, kind: 'rect', name: id, x, y, w: size, h: size, fill: '#d9d9d9', stroke: 'none' }
}

function boolGroup(id: string, op: 'union' | 'subtract' | 'intersect' | 'exclude', children: DesignNode[]): DesignNode {
  const minX = Math.min(...children.map((child) => child.x))
  const minY = Math.min(...children.map((child) => child.y))
  const maxX = Math.max(...children.map((child) => child.x + child.w))
  const maxY = Math.max(...children.map((child) => child.y + child.h))
  return {
    id,
    kind: 'group',
    name: op,
    booleanOp: op,
    x: minX,
    y: minY,
    w: Math.max(1, maxX - minX),
    h: Math.max(1, maxY - minY),
    fill: children[0]?.fill ?? '#d9d9d9',
    stroke: 'none',
    children: children.map((child) => ({ ...child, x: child.x - minX, y: child.y - minY })),
  }
}

function rasterArea(contours: Contour[], rule: 'nonzero' | 'evenodd'): number {
  let minX = Infinity
  let minY = Infinity
  let maxX = -Infinity
  let maxY = -Infinity
  for (const contour of contours) {
    for (const cubic of contour) {
      minX = Math.min(minX, cubic[0].x, cubic[3].x)
      minY = Math.min(minY, cubic[0].y, cubic[3].y)
      maxX = Math.max(maxX, cubic[0].x, cubic[3].x)
      maxY = Math.max(maxY, cubic[0].y, cubic[3].y)
    }
  }
  if (!Number.isFinite(minX)) return 0
  const step = 2
  let hits = 0
  for (let y = minY; y < maxY; y += step) {
    for (let x = minX; x < maxX; x += step) {
      if (pointInContours({ x: x + step / 2, y: y + step / 2 }, contours, rule)) hits += 1
    }
  }
  return hits * step * step
}

describe('boolean geometry', () => {
  const doc = docOf()

  it('treats a square as filled and a rotated square as turned', () => {
    const plain = nodeContours(doc, square('s', 10, 20))
    expect(pointInContours({ x: 60, y: 70 }, plain, 'nonzero')).toBe(true)
    expect(pointInContours({ x: 0, y: 0 }, plain, 'nonzero')).toBe(false)
    const turned = nodeContours(doc, { ...square('r', 0, 0), rotation: 45 })
    const outside = turned.some((contour) => contour.some((cubic) => cubic[0].x < -1 || cubic[0].x > 101 || cubic[0].y < -1 || cubic[0].y > 101))
    expect(outside).toBe(true)
    expect(rasterArea(turned, 'nonzero')).toBeGreaterThan(9000)
    expect(rasterArea(turned, 'nonzero')).toBeLessThan(11000)
  })

  it('unions, subtracts, intersects and excludes two overlapping squares', () => {
    const a = square('a', 0, 0)
    const b = square('b', 40, 30)
    const expectArea = (op: 'union' | 'subtract' | 'intersect' | 'exclude', area: number) => {
      const result = booleanResult(doc, boolGroup('g', op, [a, b]))
      const vector = contoursToVector(result.contours, result.rule)
      expect(vector.vertices.length).toBeGreaterThan(3)
      expect(vector.vertices.length).toBeLessThan(40)
      expect(rasterArea(result.contours, result.rule)).toBeGreaterThan(area - 800)
      expect(rasterArea(result.contours, result.rule)).toBeLessThan(area + 800)
    }
    expectArea('union', 15800)
    expectArea('subtract', 5800)
    expectArea('intersect', 4200)
    expectArea('exclude', 11600)
    expect(booleanResult(doc, boolGroup('x', 'exclude', [a, b])).rule).toBe('evenodd')
  })

  it('keeps the circle when a square is cut out of it', () => {
    const circle: DesignNode = { id: 'c', kind: 'ellipse', x: 0, y: 0, w: 100, h: 100, fill: '#d9d9d9' }
    const hole = square('h', 30, 30, 40)
    const result = booleanResult(doc, boolGroup('g', 'subtract', [circle, hole]))
    const vector = contoursToVector(result.contours, result.rule)
    expect(vector.segments.length).toBeGreaterThan(4)
    expect(vector.segments.length).toBeLessThan(24)
    expect(vector.segments.some((segment) => Math.hypot(segment.tangentStart.x, segment.tangentStart.y) > 1)).toBe(true)
    expect(rasterArea(result.contours, result.rule)).toBeGreaterThan(5600)
    expect(rasterArea(result.contours, result.rule)).toBeLessThan(7000)
  })

  it('folds three shapes from the left', () => {
    const result = booleanResult(doc, boolGroup('g', 'union', [square('a', 0, 0), square('b', 60, 0), square('c', 120, 0)]))
    expect(rasterArea(result.contours, result.rule)).toBeGreaterThan(21000)
    expect(rasterArea(result.contours, result.rule)).toBeLessThan(23000)
  })

  it('nests a subtract inside a union', () => {
    const inner = boolGroup('inner', 'subtract', [square('a', 0, 0), square('b', 25, 25, 50)])
    const outer = boolGroup('outer', 'union', [inner, square('c', 200, 0)])
    const result = booleanResult(doc, outer)
    expect(pointInContours({ x: 10, y: 10 }, result.contours, result.rule)).toBe(true)
    expect(pointInContours({ x: 50, y: 50 }, result.contours, result.rule)).toBe(false)
    expect(pointInContours({ x: 250, y: 50 }, result.contours, result.rule)).toBe(true)
    expect(rasterArea(result.contours, result.rule)).toBeGreaterThan(16000)
    expect(rasterArea(result.contours, result.rule)).toBeLessThan(19000)
  })

  it('hits the remaining solid and misses the hole', () => {
    const group = boolGroup('g', 'subtract', [square('a', 0, 0), square('b', 30, 30, 40)])
    const host = docOf(group)
    expect(hitDesign(host, 10, 10)?.id).toBe('g')
    expect(hitDesign(host, 50, 50)).toBeNull()
    expect(hitDesignDeepest(host, 50, 50)?.id).toBe('b')
  })

  it('copies the back shape paint and draws one path', () => {
    const host: DesignNode = {
      id: 'host',
      kind: 'frame',
      x: 0,
      y: 0,
      w: 200,
      h: 120,
      children: [
        { ...square('a', 0, 0), fill: '#112233' },
        { ...square('b', 40, 0), fill: '#abcdef' },
      ],
    }
    const next = booleanDesignNodes(docOf(host), ['a', 'b'], 'subtract', () => 'bool')
    const group = next?.screens[0].children?.find((child) => child.id === 'bool')
    expect(group?.fill).toBe('#112233')
    expect(group?.booleanOp).toBe('subtract')
    const html = group ? designChatHtml(next!, group) : ''
    expect(html).toContain('<svg')
    expect(html).toContain('fill-rule="nonzero"')
    expect(html).not.toContain('data-koma-name="a"')
    expect(html).not.toContain('data-koma-name="b"')
  })

  it('flattens a boolean into one vector and keeps a turned shape put', () => {
    const group = { ...boolGroup('g', 'union', [square('a', 10, 20), square('b', 50, 20)]), rotation: 30 }
    const flat = flattenBooleanNode(doc, group)
    expect(flat.kind).toBe('vector')
    expect(flat.children).toBeUndefined()
    expect(flat.booleanOp).toBeUndefined()
    expect(flat.fill).toBe('#d9d9d9')
    const source = booleanResult(doc, group).contours[0][0][0]
    const vertex = flat.vector?.vertices[0]
    expect(vertex).toBeTruthy()
    const fromGroup = spinToParent(group, source.x, source.y)
    const fromFlat = spinToParent(flat, vertex!.x, vertex!.y)
    expect(fromFlat.x).toBeCloseTo(fromGroup.x, 1)
    expect(fromFlat.y).toBeCloseTo(fromGroup.y, 1)
    const emptied = flattenBooleanNode(doc, boolGroup('hole', 'subtract', [square('a', 0, 0), square('b', 0, 0)]))
    expect(emptied.vector?.vertices).toEqual([])
    expect(emptied.kind).toBe('vector')
  })

  it('changes one boolean in place and flattens a plain selection', () => {
    const group = { ...boolGroup('g', 'union', [square('a', 0, 0), square('b', 40, 30)]), name: 'Union' }
    const renamed = applyDesignBoolean(docOf(group), ['g'], 'subtract', () => 'x')
    expect(renamed?.screens[0].booleanOp).toBe('subtract')
    expect(renamed?.screens[0].name).toBe('Subtract')
    const custom = applyDesignBoolean(docOf({ ...group, name: 'Window' }), ['g'], 'exclude', () => 'x')
    expect(custom?.screens[0].booleanOp).toBe('exclude')
    expect(custom?.screens[0].name).toBe('Window')
    expect(applyDesignBoolean(docOf(square('a', 0, 0)), ['a'], 'union', () => 'x')).toBeNull()
    const host: DesignNode = {
      id: 'host',
      kind: 'frame',
      x: 0,
      y: 0,
      w: 200,
      h: 160,
      children: [square('a', 0, 0), square('b', 40, 30)],
    }
    const flat = applyDesignBoolean(docOf(host), ['a', 'b'], 'flatten', () => 'flat')
    const node = flat?.screens[0].children?.find((child) => child.id === 'flat')
    expect(node?.kind).toBe('vector')
    expect(node?.booleanOp).toBeUndefined()
    expect(node?.children).toBeUndefined()
    const contours = node ? nodeContours(flat!, node) : []
    expect(rasterArea(contours, 'nonzero')).toBeGreaterThan(15000)
    expect(rasterArea(contours, 'nonzero')).toBeLessThan(16600)
  })

  it('recomputes a fig subtract instead of the baked rectangle', () => {
    const imported = nodeChangesToDesign([
      {
        guid: { sessionID: 1, localID: 1 },
        type: 'BOOLEAN_OPERATION',
        booleanOperation: 'SUBTRACT',
        name: 'Subtract',
        size: { x: 100, y: 100 },
        transform: { m02: 0, m12: 0 },
      },
      {
        guid: { sessionID: 1, localID: 2 },
        parentIndex: { guid: { sessionID: 1, localID: 1 } },
        type: 'RECTANGLE',
        size: { x: 100, y: 100 },
        transform: { m02: 0, m12: 0 },
        fillPaints: [{ type: 'SOLID', color: { r: 0.85, g: 0.85, b: 0.85 } }],
      },
      {
        guid: { sessionID: 1, localID: 3 },
        parentIndex: { guid: { sessionID: 1, localID: 1 } },
        type: 'RECTANGLE',
        size: { x: 40, y: 40 },
        transform: { m02: 30, m12: 30 },
        fillPaints: [{ type: 'SOLID', color: { r: 0, g: 0, b: 0 } }],
      },
    ])
    const node = imported.doc.screens[0]
    expect(node?.kind).toBe('vector')
    expect(node?.booleanOp).toBe('subtract')
    expect(node?.children).toHaveLength(2)
    const result = booleanResult(imported.doc, node)
    expect(pointInContours({ x: 10, y: 10 }, result.contours, result.rule)).toBe(true)
    expect(pointInContours({ x: 50, y: 50 }, result.contours, result.rule)).toBe(false)
    expect(rasterArea(result.contours, result.rule)).toBeGreaterThan(7600)
    expect(rasterArea(result.contours, result.rule)).toBeLessThan(9200)
  })

  it('cancels identical shapes on subtract and exclude', () => {
    const a = square('a', 0, 0)
    const b = square('b', 0, 0)
    const area = (op: 'union' | 'subtract' | 'intersect' | 'exclude') => rasterArea(booleanResult(doc, boolGroup('g', op, [a, b])).contours, op === 'exclude' ? 'evenodd' : 'nonzero')
    expect(area('union')).toBeGreaterThan(9000)
    expect(area('intersect')).toBeGreaterThan(9000)
    expect(area('subtract')).toBeLessThan(400)
    expect(area('exclude')).toBeLessThan(400)
  })

  it('shapes that only touch along an edge intersect to nothing', () => {
    const result = booleanResult(doc, boolGroup('g', 'intersect', [square('a', 0, 0), square('b', 100, 0)]))
    expect(result.contours).toEqual([])
    // Touching shapes still union into one piece.
    const merged = booleanResult(doc, boolGroup('u', 'union', [square('a', 0, 0), square('b', 100, 0)]))
    expect(merged.contours).toHaveLength(1)
  })

  it('a plain group takes an operation, and the same operation turns it back', () => {
    const plain: DesignNode = { ...boolGroup('g', 'union', [square('a', 0, 0), square('b', 40, 30)]), name: 'Group' }
    delete plain.booleanOp
    const made = applyDesignBoolean(docOf(plain), ['g'], 'subtract', () => 'x')
    expect(made?.screens[0].booleanOp).toBe('subtract')
    expect(made?.screens[0].name).toBe('Subtract')
    const back = applyDesignBoolean(made!, ['g'], 'subtract', () => 'x')
    expect(back?.screens[0].booleanOp).toBeUndefined()
    expect(back?.screens[0].name).toBe('Group')
    expect(back?.screens[0].children?.map((child) => child.id)).toEqual(['a', 'b'])
    // A different operation switches instead of toggling off.
    expect(applyDesignBoolean(made!, ['g'], 'exclude', () => 'x')?.screens[0].booleanOp).toBe('exclude')
  })

  it('text and lines block a boolean; a lone shape does too', () => {
    const text: DesignNode = { id: 't', kind: 'text', x: 0, y: 0, w: 40, h: 20, text: 'Hi' }
    const line: DesignNode = { id: 'l', kind: 'line', x: 0, y: 0, w: 60, h: 2 }
    const host: DesignNode = { id: 'host', kind: 'frame', x: 0, y: 0, w: 300, h: 200, children: [square('a', 0, 0), square('b', 40, 30), text, line] }
    const d = docOf(host)
    expect(booleanEligible(d, ['a', 'b']).ok).toBe(true)
    expect(booleanEligible(d, ['a', 't'])).toMatchObject({ ok: false })
    expect(booleanEligible(d, ['a', 'l'])).toMatchObject({ ok: false })
    expect(booleanEligible(d, ['a'])).toMatchObject({ ok: false })
    expect(applyDesignBoolean(d, ['a', 't'], 'union', () => 'x')).toBeNull()
    // A group holding text is blocked as a whole.
    const group: DesignNode = { ...boolGroup('g', 'union', [square('c', 0, 0)]), children: [square('c', 0, 0), text] }
    expect(booleanEligible(docOf(group), ['g'])).toMatchObject({ ok: false })
  })

  it('path fills are SVG colors and gradients, never CSS background layers', () => {
    const red: DesignNode = { ...square('a', 0, 0), fill: '#ff0000' }
    const solid = svgFillLayers(docOf(red), red, '#d9d9d9', 'a', 100, 100)
    expect(solid).toEqual([{ fill: '#ff0000', opacity: undefined }])
    expect(solid[0].fill).not.toContain('gradient')
    // A recolored boolean group paints the new color, not the child's.
    const group = boolGroup('g', 'union', [square('a', 0, 0), square('b', 40, 30)])
    const recolored: DesignNode = { ...group, fill: '#00ff00' }
    expect(svgFillLayers(docOf(recolored), booleanPaintNode(recolored), '#d9d9d9', 'g', 100, 100)[0].fill).toBe('#00ff00')
    // Gradients get a definition whose line runs corner to corner like the CSS gradient.
    const gradient: DesignNode = {
      ...square('c', 0, 0),
      fills: [{ type: 'gradient', kind: 'linear', transform: [90, 0.5, 0.5], stops: [{ color: '#000000', at: 0 }, { color: '#ffffff', at: 1 }] }],
    }
    const layer = svgFillLayers(docOf(gradient), gradient, '#d9d9d9', 'c', 200, 100)[0]
    expect(layer.fill).toBe('url(#c-0)')
    expect(layer.gradient).toMatchObject({ kind: 'linear', x1: 0, y1: 50, x2: 200, y2: 50 })
  })
})
