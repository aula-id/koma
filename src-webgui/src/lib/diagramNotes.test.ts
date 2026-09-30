import assert from 'node:assert/strict'
import { emptyDiagram } from './diagram.ts'
import { orphanDiagramNoteImages } from './diagramNoteAttach.ts'
import {
  diagramFolder,
  diagramShapeFolder,
  diagramShapeNotesPath,
  renderDiagramNotes,
  renderShapeNote,
  shapeNoteAttachName,
} from './diagramNotes.ts'
import { normalizeDiagramNoteMarkdown, repairComposerImageMarkers } from './markdownNote.ts'

assert.equal(diagramFolder('.koma/auth.diag'), '.koma/auth')
assert.equal(diagramShapeFolder('.koma/auth.diag', 'node', 'n1'), '.koma/auth/cards/n1')
assert.equal(diagramShapeFolder('.koma/auth.diag', 'edge', 'e1'), '.koma/auth/lines/e1')
assert.equal(diagramShapeNotesPath('.koma/auth.diag', 'node', 'n1'), '.koma/auth/cards/n1/notes.md')
assert.equal(renderDiagramNotes(emptyDiagram(), '.koma/auth.diag'), '')

{
  const doc = emptyDiagram()
  doc.nodes.push(
    { id: 'n1', kind: 'rect', x: 0, y: 0, w: 10, h: 10, text: 'Login', detail: 'The login screen.\n\n![wire](img-1.png)' },
    { id: 'n2', kind: 'rect', x: 0, y: 80, w: 10, h: 10, text: 'Home' },
  )
  doc.edges.push({ id: 'e1', from: 'n1', to: 'n2', text: 'submits', detail: 'Posts the form.' })
  const nodeNote = renderShapeNote(doc, 'node', 'n1')
  assert.equal(nodeNote.startsWith('# Login\n'), true)
  assert.equal(nodeNote.includes('id: n1'), true)
  assert.equal(nodeNote.includes('![wire](img-1.png)'), true)
  const edgeNote = renderShapeNote(doc, 'edge', 'e1')
  assert.equal(edgeNote.includes('# submits'), true)
  assert.equal(edgeNote.includes('kind: line'), true)
  assert.equal(shapeNoteAttachName(doc, 'node', 'n1'), 'login-n1-notes.md')
  assert.equal(shapeNoteAttachName(doc, 'edge', 'e1').startsWith('line-submits-e1-notes.md'), true)
  const notes = renderDiagramNotes(doc, '.koma/auth.diag')
  assert.equal(notes.includes('# Login'), true)
  assert.equal(notes.includes('# submits'), true)
}

{
  const entries = [
    { name: 'notes.md', path: '.koma/auth/cards/n1/notes.md', isDir: false },
    { name: 'img-1.png', path: '.koma/auth/cards/n1/img-1.png', isDir: false },
    { name: 'old.png', path: '.koma/auth/cards/n1/old.png', isDir: false },
  ]
  assert.deepEqual(orphanDiagramNoteImages(entries, new Set(['img-1.png'])), ['.koma/auth/cards/n1/old.png'])
}

{
  const raw = 'can you see me?\n\n[Image #1]'
  const repaired = repairComposerImageMarkers(raw, ['img-abc.png'])
  assert.equal(repaired.includes('![image](img-abc.png)'), true)
  const norm = normalizeDiagramNoteMarkdown(repaired)
  assert.equal(norm.includes('[Image #1]'), false)
  assert.equal(norm.includes('![image](img-abc.png)'), true)
}

{
  const broken = 'apata ini?\n\n[download](img-mnuo0zc2.png)'
  const fixed = normalizeDiagramNoteMarkdown(broken)
  assert.equal(fixed.includes('![download](img-mnuo0zc2.png)') || fixed.includes('![](img-mnuo0zc2.png)'), true)
  assert.equal(/^\s*\[download\]/.test(fixed.split('\n').pop() ?? ''), false)
}

{
  const hard = 'hello\\\nworld'
  const kept = normalizeDiagramNoteMarkdown(hard)
  assert.equal(kept.includes('hello\\'), true)
  assert.equal(kept.includes('world'), true)
}
