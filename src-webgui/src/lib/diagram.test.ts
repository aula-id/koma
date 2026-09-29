import assert from 'node:assert/strict'
import {
  diagramFileName,
  edgeRoute,
  emptyDiagram,
  isDiagramPath,
  nearestPort,
  nearestSide,
  nextRotation,
  nodeStyle,
  portAnchor,
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

{
  const node = { id: 'a', kind: 'rect' as const, x: 0, y: 0, w: 100, h: 50, text: '' }
  assert.deepEqual(portAnchor(node, 0), { x: 0, y: 0 })
  assert.deepEqual(portAnchor(node, 2), { x: 50, y: 0 })
  assert.deepEqual(portAnchor(node, 7), { x: 100, y: 37.5 })
  assert.deepEqual(portAnchor(node, 15), { x: 0, y: 12.5 })
  assert.equal(nearestPort(node, { x: 100, y: 40 }), 7)
  const ell = portAnchor({ ...node, kind: 'ellipse' }, 0)
  assert.ok(Math.abs((ell.x - 50) ** 2 / 50 ** 2 + (ell.y - 25) ** 2 / 25 ** 2 - 1) < 1e-6)
  const other = { ...node, id: 'b', x: 200 }
  const routed = edgeRoute(node, other, { id: 'e', from: 'a', to: 'b', fromPort: 5, toPort: 12 })
  assert.deepEqual(routed[0], portAnchor(node, 5))
  assert.deepEqual(routed[routed.length - 1], portAnchor(other, 12))
  const doc = emptyDiagram()
  doc.edges.push({ id: 'e', from: 'a', to: 'b', fromPort: 0, toPort: 15 })
  const parsed = parseDiagram(serializeDiagram(doc))
  assert.equal(parsed.doc.edges[0]?.fromPort, 0)
  assert.equal(parsed.doc.edges[0]?.toPort, 15)
  assert.equal(parsed.doc.edges[0]?.fromSide, undefined)
}

{
  const plain = { id: 't', kind: 'text' as const, x: 0, y: 0, w: 120, h: 36, text: 'Text' }
  const labeled = { ...plain, id: 't2', fill: true, stroke: true, fillColor: '#aabbcc', strokeColor: '#ddeeff' }
  const shaped = { id: 'r', kind: 'rect' as const, x: 0, y: 0, w: 80, h: 40, text: 'Card', fill: false, fillColor: '#112233', stroke: false, strokeColor: '#445566' }
  assert.equal(nodeStyle(plain).fill, false)
  assert.equal(nodeStyle(plain).stroke, false)
  assert.equal(nodeStyle({ ...plain, kind: 'rect' }).fill, true)
  assert.equal(nodeStyle({ ...plain, kind: 'rect' }).stroke, true)
  const doc = emptyDiagram()
  doc.nodes.push(plain, labeled, shaped)
  const parsed = parseDiagram(serializeDiagram(doc))
  assert.deepEqual(parsed.doc.nodes[0], plain)
  assert.deepEqual(parsed.doc.nodes[1], labeled)
  assert.deepEqual(parsed.doc.nodes[2], shaped)
}
