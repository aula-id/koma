import assert from 'node:assert/strict'
import {
  chipKindFromWire,
  chipPayloadForAttachMarker,
  chipPayloadForFileRef,
  chipPayloadFromWire,
  composerAttachmentPlain,
  displayLabelForWire,
  hasComposerAttachmentDrag,
  readComposerAttachmentDrag,
  writeComposerAttachmentDrag,
} from './composerIpc.ts'

assert.equal(chipKindFromWire('@bump.sh'), 'file')
assert.equal(displayLabelForWire('@bump.sh'), 'bump.sh')
assert.equal(displayLabelForWire('@[2]src/lib/foo.ts'), 'foo.ts')
assert.equal(displayLabelForWire('[Image #2]'), 'Image 2')
assert.equal(displayLabelForWire('[Pasted Text #3]'), 'Paste 3')

const file = chipPayloadForFileRef('@bump.sh')
assert.equal(file.wireText, '@bump.sh')
assert.equal(file.displayLabel, 'bump.sh')
assert.equal(file.kind, 'file')

const image = chipPayloadFromWire('[Image #1]')
assert.equal(image.kind, 'image')
assert.equal(image.markerN, 1)
assert.equal(image.displayLabel, 'Image 1')

const pasteChip = chipPayloadForAttachMarker('pasted_text', 4)
assert.equal(pasteChip.kind, 'paste')
assert.equal(pasteChip.wireText, '[Pasted Text #4]')

const drag = { kind: 'image' as const, markerN: 1 }
assert.deepEqual(readComposerAttachmentDrag(writeComposerAttachmentDrag(drag)), drag)
assert.deepEqual(readComposerAttachmentDrag(null, composerAttachmentPlain(drag)), drag)
assert.equal(readComposerAttachmentDrag('nope', 'plain'), null)
assert.equal(hasComposerAttachmentDrag(['text/plain']), false)
assert.equal(hasComposerAttachmentDrag(['application/x-koma-attachment']), true)

console.log('composerIpc.test.ts ok')
