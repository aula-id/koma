import assert from 'node:assert/strict'
import { assignFreshPasteMarkers, formatPasteFence, shouldCollapsePaste, splitPasteMessage } from './pasteText.ts'

assert.equal(shouldCollapsePaste('short'), false)
assert.equal(shouldCollapsePaste('a\nb'), true)
assert.equal(shouldCollapsePaste('x'.repeat(150)), true)

const fence = formatPasteFence({ n: 2, path: 'pastes/02-paste.txt', text: 'hello\nthere' })
const split = splitPasteMessage(`see\n\n${fence}\n\nthanks`)
assert.equal(split.prose, 'see\n\nthanks')
assert.equal(split.pastes.length, 1)
assert.equal(split.pastes[0]?.n, 2)
assert.equal(split.pastes[0]?.text, 'hello\nthere')

const seen = new Set([1])
const queue = [
  { id: 'a', markerN: null, cancelled: false },
  { id: 'b', markerN: null, cancelled: true },
]
const attachments = [
  { markerN: 1, kind: 'pasted_text' },
  { markerN: 2, kind: 'image' },
  { markerN: 3, kind: 'pasted_text' },
  { markerN: 4, kind: 'pasted_text' },
]
const assigned = assignFreshPasteMarkers(queue, seen, attachments)
assert.equal(assigned.length, 2)
assert.equal(assigned[0]?.id, 'a')
assert.equal(assigned[0]?.markerN, 3)
assert.equal(assigned[1]?.cancelled, true)
assert.equal(queue[1]?.markerN, 4)
assert.equal(assignFreshPasteMarkers(queue, seen, attachments).length, 0)
