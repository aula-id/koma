import assert from 'node:assert/strict'
import { emptyDiagram } from './diagram.ts'
import {
  captureDiagram,
  diagramToMermaid,
  parseMermaidDiagram,
  diagramMessageParts,
  splitDiagramMessage,
} from './diagramMermaid.ts'

{
  const doc = emptyDiagram()
  doc.nodes.push(
    { id: 'a', kind: 'rect', x: 0, y: 0, w: 100, h: 40, text: 'Start' },
    { id: 'b', kind: 'ellipse', x: 0, y: 80, w: 100, h: 40, text: 'Go' },
    { id: 'c', kind: 'diamond', x: 200, y: 0, w: 80, h: 40, text: 'Say "hi"' },
    { id: 'd', kind: 'text', x: 0, y: 160, w: 80, h: 24, text: 'note' },
  )
  doc.edges.push(
    { id: 'e1', from: 'a', to: 'b' },
    { id: 'e2', from: 'b', to: 'c', dash: 'dashed', start: 'arrow', end: 'none' },
    { id: 'e3', from: 'a', to: 'd', stroke: false },
  )
  const mermaid = diagramToMermaid(doc, 'auth')
  assert.equal(
    mermaid,
    [
      '```mermaid',
      'flowchart TD',
      '  %% auth',
      '  %% koma-text d',
      '  a["Start"]',
      '  c{"Say #quot;hi#quot;"}',
      '  b(("Go"))',
      '  d["note"]',
      '  a --> b',
      '  b <-.- c',
      '```',
    ].join('\n'),
  )
  const parsed = parseMermaidDiagram(mermaid)
  assert.equal(parsed.title, 'auth')
  assert.deepEqual(
    parsed.doc.nodes.map((node) => [node.id, node.kind, node.text]),
    [
      ['a', 'rect', 'Start'],
      ['c', 'diamond', 'Say "hi"'],
      ['b', 'ellipse', 'Go'],
      ['d', 'text', 'note'],
    ],
  )
  assert.equal(parsed.doc.edges.length, 2)
  assert.equal(parsed.doc.edges[0].end ?? 'arrow', 'arrow')
  assert.equal(parsed.doc.edges[1].dash, 'dashed')
  assert.equal(parsed.doc.edges[1].start, 'arrow')
  assert.equal(parsed.doc.edges[1].end, 'none')
}

{
  const doc = emptyDiagram()
  doc.nodes.push(
    { id: 'a', kind: 'rect', x: 0, y: 0, w: 100, h: 40, text: 'A' },
    { id: 'b', kind: 'rect', x: 0, y: 200, w: 100, h: 40, text: 'B' },
  )
  doc.edges.push({ id: 'e', from: 'a', to: 'b' })
  const inside = captureDiagram(doc, { x: 10, y: 10, w: 20, h: 20 })
  assert.deepEqual(inside.nodes.map((node) => node.id), ['a'])
  assert.equal(inside.edges.length, 0)
}

{
  const doc = emptyDiagram()
  doc.nodes.push(
    { id: 'a', kind: 'rect', x: 0, y: 0, w: 40, h: 40, text: 'A' },
    { id: 'b', kind: 'rect', x: 200, y: 0, w: 40, h: 40, text: 'B' },
  )
  doc.edges.push({ id: 'e', from: 'a', to: 'b', route: 'straight' })
  const crossed = captureDiagram(doc, { x: 80, y: 0, w: 40, h: 40 })
  assert.deepEqual(crossed.nodes.map((node) => node.id), ['a', 'b'])
  assert.equal(crossed.edges.length, 1)
}

{
  const split = splitDiagramMessage('look\n\n```mermaid\nflowchart TD\n  a["A"]\n```\n\nthanks')
  assert.equal(split.prose, 'look\n\nthanks')
  assert.equal(split.diagrams.length, 1)
  assert.equal(split.diagrams[0].mermaid.includes('a["A"]'), true)
  const parts = diagramMessageParts('look\n\n```mermaid\nflowchart TD\n  a["A"]\n```\n\nthanks')
  assert.deepEqual(
    parts.map((part) => part.type),
    ['text', 'diagram', 'text'],
  )
  assert.equal(parts[0].type === 'text' && parts[0].text.startsWith('look'), true)
  assert.equal(parts[1].type === 'diagram' && parts[1].mermaid.includes('a["A"]'), true)
  assert.equal(parts[2].type === 'text' && parts[2].text.includes('thanks'), true)
}

assert.equal(diagramToMermaid(emptyDiagram(), 'empty'), '')
