import assert from 'node:assert/strict'
import { emptyDiagram } from './diagram.ts'
import { diagramDetailImageNames, orphanDiagramNoteImages } from './diagramNoteAttach.ts'
import { diagramFolder, diagramNotesPath, renderDiagramNotes } from './diagramNotes.ts'

assert.equal(diagramNotesPath('.koma/auth.diag'), '.koma/auth/notes.md')
assert.equal(diagramFolder('.koma/auth.diag'), '.koma/auth')
assert.equal(diagramNotesPath('auth.diag'), null)
assert.equal(renderDiagramNotes(emptyDiagram(), '.koma/auth.diag'), '')

{
  const doc = emptyDiagram()
  doc.nodes.push(
    { id: 'n1', kind: 'rect', x: 0, y: 0, w: 10, h: 10, text: 'Login', detail: 'The login screen.\n\n![wire](img-1.png)' },
    { id: 'n2', kind: 'rect', x: 0, y: 80, w: 10, h: 10, text: 'Home' },
  )
  doc.edges.push({ id: 'e1', from: 'n1', to: 'n2', text: 'submits', detail: 'Posts the form.' })
  const notes = renderDiagramNotes(doc, '.koma/auth.diag')
  assert.equal(notes.startsWith('# auth\n'), true)
  assert.equal(notes.includes('`.koma/auth/img.png`'), true)
  assert.equal(notes.includes('## Login'), true)
  assert.equal(notes.includes('id: n1'), true)
  assert.equal(notes.includes('kind: rect'), true)
  assert.equal(notes.includes('![wire](img-1.png)'), true)
  assert.equal(notes.includes('## submits'), true)
  assert.equal(notes.includes('kind: line'), true)
  assert.equal(notes.includes('from: Login (`n1`)'), true)
  assert.equal(notes.includes('to: Home (`n2`)'), true)
  assert.equal(notes.includes('## Home'), false)
  assert.deepEqual(diagramDetailImageNames(doc, '.koma/auth.diag').sort(), ['img-1.png'])
}

{
  const entries = [
    { name: 'notes.md', path: '.koma/auth/notes.md', isDir: false },
    { name: 'img-1.png', path: '.koma/auth/img-1.png', isDir: false },
    { name: 'old.png', path: '.koma/auth/old.png', isDir: false },
    { name: 'subdir', path: '.koma/auth/subdir', isDir: true },
  ]
  assert.deepEqual(orphanDiagramNoteImages(entries, new Set(['img-1.png'])), ['.koma/auth/old.png'])
}