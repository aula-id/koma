import assert from 'node:assert/strict'
import {
  diagramFileName,
  edgeRoute,
  emptyDiagram,
  isDiagramPath,
  nearestSide,
  nextRotation,
  parseDiagram,
  resizeNode,
  routePath,
  serializeDiagram,
  slideSegment,
  snap,
} from './diagram.ts'

assert.equal(snap(20, 16, true), 16)
assert.equal(snap(24, 16, true), 32)
assert.equal(snap(24, 16, false), 24)
assert.equal(snap(24, 0, true), 24)

assert.equal(diagramFileName(' auth '), 'auth.diag')
assert.equal(diagramFileName('auth.diag'), 'auth.diag')
assert.equal(diagramFileName('auth.DIAG'), 'auth.diag')
assert.equal(diagramFileName('my.flow'), 'my.flow.diag')
assert.equal(diagramFileName('../x'), null)
assert.equal(diagramFileName('a/b'), null)
assert.equal(diagramFileName('.diag'), null)
assert.equal(diagramFileName(''), null)

assert.equal(isDiagramPath('.koma/auth.diag'), true)
assert.equal(isDiagramPath('.koma/nested/auth.diag'), false)
assert.equal(isDiagramPath('auth.diag'), false)

{
  const doc = emptyDiagram()
  doc.snap = false
  doc.nodes.push({ id: 'n1', kind: 'rect', x: 16, y: 32, w: 160, h: 64, text: 'start' })
  doc.edges.push({ id: 'e1', from: 'n1', to: 'n2' })
  const parsed = parseDiagram(serializeDiagram(doc))
  assert.equal(parsed.error, null)
  assert.deepEqual(parsed.doc, doc)
}

{
  const parsed = parseDiagram('{')
  assert.equal(parsed.error, 'This file is not a diagram')
  assert.deepEqual(parsed.doc, emptyDiagram())
}

{
  const parsed = parseDiagram('{"snap":false}')
  assert.equal(parsed.error, 'This file is not a diagram')
  assert.equal(parsed.doc.snap, true)
}

const box = { id: 'a', kind: 'rect' as const, x: 0, y: 0, w: 100, h: 40, text: '' }
assert.equal(nearestSide(box, { x: 180, y: 20 }), 'e')
assert.equal(nearestSide(box, { x: 50, y: -30 }), 'n')

{
  const grown = resizeNode(box, 'e', 20, 0, 16, true)
  assert.equal(grown.x, 0)
  // East edge lands on 120, halfway between 112 and 128, and snap rounds half up.
  assert.equal(grown.w, 128)
  const west = resizeNode({ ...box, x: 16, w: 160 }, 'w', 20, 0, 16, true)
  assert.equal(west.x, 32)
  assert.equal(west.x + west.w, 176)
  const min = resizeNode(box, 'e', -80, 0, 16, false)
  assert.equal(min.w, 32)
}

assert.equal(nextRotation(0, 0, 14, true), 15)
assert.equal(nextRotation(10, 0, 0, false), 10)

function aligned(points: { x: number; y: number }[]): boolean {
  for (let i = 1; i < points.length; i++) {
    const prev = points[i - 1]
    const point = points[i]
    if (Math.abs(prev.x - point.x) > 0.01 && Math.abs(prev.y - point.y) > 0.01) return false
  }
  return points.length >= 2
}

{
  const from = { ...box, id: 'a' }
  const to = { ...box, id: 'b', x: 220, y: 80 }
  const points = edgeRoute(from, to, { id: 'e', from: 'a', to: 'b' })
  assert.equal(aligned(points), true)
  assert.deepEqual(points[0], { x: 100, y: 20 })
  assert.deepEqual(points[points.length - 1], { x: 220, y: 100 })
  const straight = edgeRoute(from, to, { id: 'e', from: 'a', to: 'b', route: 'straight' })
  assert.equal(straight.length, 2)
  assert.equal(routePath(points, true).includes('Q'), true)
  assert.equal(routePath(points, false).includes('Q'), false)
}

{
  const slid = slideSegment([{ x: 0, y: 0 }, { x: 100, y: 0 }], 0, 20)
  assert.equal(aligned(slid), true)
  assert.deepEqual(slid[0], { x: 0, y: 0 })
  assert.deepEqual(slid[slid.length - 1], { x: 100, y: 0 })
  assert.equal(slid.some((point) => point.y === 20), true)
}

{
  const doc = emptyDiagram()
  doc.nodes.push({ ...box, rotation: 15 })
  doc.edges.push({
    id: 'e1',
    from: 'a',
    to: 'b',
    stroke: false,
    color: '#112233',
    width: 2,
    dash: 'dashed',
    start: 'open',
    end: 'none',
    corner: 'rounded',
    route: 'straight',
    fromSide: 'e',
    toSide: 'w',
    bends: [{ x: 40, y: 12 }],
  })
  const parsed = parseDiagram(serializeDiagram(doc))
  assert.equal(parsed.error, null)
  assert.deepEqual(parsed.doc.nodes[0]?.rotation, 15)
  assert.deepEqual(parsed.doc.edges[0], doc.edges[0])
}
