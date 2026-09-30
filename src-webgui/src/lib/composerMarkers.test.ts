import assert from 'node:assert/strict'
import {
  assignFreshMarkerInserts,
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

{
  const seen = new Set<string>()
  const empty: { id: string; kind: 'image' | 'pasted_text'; markerN: number | null; cancelled: boolean }[] = []
  assert.deepEqual(assignFreshMarkerInserts(empty, seen, [{ markerN: 1, kind: 'image' }]), [])
  assert.equal(seen.has('image:1'), false)
  const waiting = [{ id: 'q1', kind: 'image' as const, markerN: null, cancelled: false }]
  const got = assignFreshMarkerInserts(waiting, seen, [{ markerN: 1, kind: 'image' }])
  assert.equal(got.length, 1)
  assert.equal(seen.has('image:1'), true)
}

console.log('composerMarkers.test.ts ok')
