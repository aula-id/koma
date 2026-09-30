import assert from 'node:assert/strict'
import {
  attachmentMarkersRemoved,
  attachmentQueueStillPending,
  imageMarker,
  markerKeysInText,
} from './composerMarkers.ts'

{
  const draft = 'hello '
  assert.deepEqual([...attachmentMarkersRemoved(draft, `${draft}world`)], [])
  assert.deepEqual([...attachmentMarkersRemoved(`${draft}${imageMarker(2)}`, draft)], ['image:2'])
  assert.deepEqual([...markerKeysInText(`a ${imageMarker(1)} b`)], ['image:1'])
}

{
  const queue = [{ kind: 'image' as const, markerN: null, cancelled: false }]
  assert.equal(attachmentQueueStillPending(queue, 'image', 1), true)
  queue[0]!.markerN = 1
  assert.equal(attachmentQueueStillPending(queue, 'image', 1), true)
  queue[0]!.cancelled = true
  assert.equal(attachmentQueueStillPending(queue, 'image', 1), false)
}

console.log('composerMarkers.test.ts ok')
