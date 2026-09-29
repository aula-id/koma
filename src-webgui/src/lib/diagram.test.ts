import assert from 'node:assert/strict'
import {
  diagramFileName,
  emptyDiagram,
  isDiagramPath,
  parseDiagram,
  serializeDiagram,
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
