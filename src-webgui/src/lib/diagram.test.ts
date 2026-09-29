import assert from 'node:assert/strict'
import {
  diagramFileName,
  arrowHead,
  copyNode,
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
  routeMidpoint,
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
  const end = arrowHead('end', 'arrow')
  const start = arrowHead('start', 'arrow')
  assert.ok(end && start)
  assert.equal(end.refX, 7)
  assert.equal(end.refY, 3)
  assert.equal(start.refX, 0)
  assert.equal(start.refY, 3)
  assert.equal(end.d?.includes('L7,3'), true)
  assert.equal(start.d?.includes('L0,3'), true)
  assert.equal(arrowHead('end', 'open')?.fill, false)
  assert.equal(arrowHead('end', 'open')?.d?.endsWith('Z'), false)
  assert.equal(arrowHead('end', 'none'), null)
  assert.equal(arrowHead('end', 'dot')?.circle?.cx, 4.5)
  assert.equal(arrowHead('start', 'dot')?.circle?.cx, 2.5)
  assert.equal(arrowHead('start', 'dot')?.refX, 0)
  assert.equal(arrowHead('end', 'diamond')?.fill, true)
  assert.equal(arrowHead('start', 'diamondOpen')?.fill, false)
  assert.equal(arrowHead('start', 'bar')?.d, 'M0,0.5 V5.5')
  const doc = emptyDiagram()
  doc.edges.push({ id: 'e', from: 'a', to: 'b', start: 'diamond', end: 'circle' })
  const parsed = parseDiagram(serializeDiagram(doc))
  assert.equal(parsed.doc.edges[0]?.start, 'diamond')
  assert.equal(parsed.doc.edges[0]?.end, 'circle')
  const plain = parseDiagram(serializeDiagram(emptyDiagram()))
  plain.doc.edges.push({ id: 'e', from: 'a', to: 'b', start: 'bar', end: 'dot' })
  const kept = parseDiagram(serializeDiagram(plain.doc))
  assert.equal(kept.doc.edges[0]?.start, 'bar')
  assert.equal(kept.doc.edges[0]?.end, 'dot')
}

{
  const text = { id: 't', kind: 'text' as const, x: 10, y: 20, w: 120, h: 36, text: 'hello' }
  const copied = copyNode(text, 't2', 16, 16)
  assert.equal(copied.id, 't2')
  assert.equal(copied.kind, 'text')
  assert.equal(copied.text, 'hello')
  assert.equal(copied.x, 26)
  assert.equal(copied.y, 36)
  assert.equal(copied.w, 120)
  assert.equal(copied.h, 36)
  assert.equal(copied.fill, undefined)
  assert.equal(copied.stroke, undefined)
  const shaped = copyNode(
    { id: 'r', kind: 'rect', x: 0, y: 0, w: 80, h: 40, text: 'label', rotation: 15, fill: false, fillColor: '#112233' },
    'r2',
    16,
    16,
  )
  assert.equal(shaped.rotation, 15)
  assert.equal(shaped.fill, false)
  assert.equal(shaped.fillColor, '#112233')
  assert.equal(shaped.stroke, undefined)
  const noted = copyNode({ ...shaped, detail: 'The login card' }, 'r3', 0, 0)
  assert.equal(noted.detail, 'The login card')
}

{
  const doc = emptyDiagram()
  doc.nodes.push({ id: 'n', kind: 'rect', x: 0, y: 0, w: 160, h: 64, text: 'Login', detail: 'The login screen' })
  doc.edges.push({ id: 'e', from: 'n', to: 'n', text: 'submits', detail: 'Posts the form' })
  const parsed = parseDiagram(serializeDiagram(doc))
  assert.equal(parsed.error, null)
  assert.equal(parsed.doc.nodes[0]?.detail, 'The login screen')
  assert.equal(parsed.doc.edges[0]?.text, 'submits')
  assert.equal(parsed.doc.edges[0]?.detail, 'Posts the form')
  const blank = parseDiagram(serializeDiagram({ ...doc, nodes: [{ ...doc.nodes[0], detail: '  ' }], edges: [{ ...doc.edges[0], text: ' ', detail: '' }] }))
  assert.equal(blank.doc.nodes[0]?.detail, undefined)
  assert.equal(blank.doc.edges[0]?.text, undefined)
  assert.equal(blank.doc.edges[0]?.detail, undefined)
}

{
  assert.deepEqual(routeMidpoint([{ x: 0, y: 0 }, { x: 100, y: 0 }]), { x: 50, y: 0 })
  assert.deepEqual(routeMidpoint([{ x: 0, y: 0 }, { x: 100, y: 0 }, { x: 100, y: 100 }]), { x: 100, y: 0 })
  assert.deepEqual(routeMidpoint([]), { x: 0, y: 0 })
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
