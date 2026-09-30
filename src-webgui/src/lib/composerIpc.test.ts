import assert from 'node:assert/strict'
import {
  chipKindFromWire,
  chipPayloadForFileRef,
  chipPayloadFromWire,
  displayLabelForWire,
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

console.log('composerIpc.test.ts ok')
