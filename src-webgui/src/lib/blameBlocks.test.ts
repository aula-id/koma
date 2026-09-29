import assert from 'node:assert/strict'
import { LANE_COLORS } from './gitGraphLayout'
import {
  blameBlocks,
  blameLaneColor,
  relativeBlameTime,
  visibleBlameBlocks,
  type BlameRow,
} from './blameBlocks'

const NOW = 1_700_000_000_000
const nowSec = NOW / 1000

function row(oid: string, extra: Partial<BlameRow> = {}): BlameRow {
  return {
    oid,
    author: extra.author ?? 'Ada',
    time: extra.time ?? String(nowSec),
    summary: extra.summary ?? `subject ${oid}`,
    text: extra.text ?? 'line',
    body: extra.body,
  }
}

{
  assert.deepEqual(blameBlocks([]), [])
}

{
  const blocks = blameBlocks([
    row('aaa', { body: 'why', summary: 'first' }),
    row('aaa', { body: '', summary: 'ignored' }),
    row('bbb', { summary: 'middle' }),
    row('aaa', { body: 'again', summary: 'later' }),
  ])
  assert.equal(blocks.length, 3)
  assert.deepEqual(
    blocks.map((b) => [b.start, b.end, b.oid, b.summary, b.body]),
    [
      [1, 2, 'aaa', 'first', 'why'],
      [3, 3, 'bbb', 'middle', ''],
      [4, 4, 'aaa', 'later', 'again'],
    ],
  )
}

{
  const blocks = blameBlocks([
    row('aaa', { body: '' }),
    row('aaa', { body: 'filled later' }),
  ])
  assert.equal(blocks[0].body, 'filled later')
  assert.equal(blocks[0].end, 2)
}

{
  const blocks = [
    { start: 1, end: 2, oid: 'a', author: '', time: '', summary: '', body: '' },
    { start: 10, end: 12, oid: 'b', author: '', time: '', summary: '', body: '' },
  ]
  assert.deepEqual(
    visibleBlameBlocks(blocks, 0, 40, 18, 0).map((b) => b.oid),
    ['a'],
  )
  assert.deepEqual(
    visibleBlameBlocks(blocks, 400, 40, 18, 0).map((b) => b.oid),
    [],
  )
  assert.deepEqual(
    visibleBlameBlocks(blocks, 0, 40, 18, 200).map((b) => b.oid),
    ['a', 'b'],
  )
}

{
  assert.equal(relativeBlameTime('nope', NOW), '')
  assert.equal(relativeBlameTime(String(nowSec - 10), NOW), 'just now')
  assert.equal(relativeBlameTime(String(nowSec - 60), NOW), '1 minute ago')
  assert.equal(relativeBlameTime(String(nowSec - 2 * 3600), NOW), '2 hours ago')
  assert.equal(relativeBlameTime(String(nowSec - 86400), NOW), '1 day ago')
  assert.equal(relativeBlameTime(String(nowSec - 21 * 86400), NOW), '3 weeks ago')
  assert.equal(relativeBlameTime(String(nowSec - 40 * 86400), NOW), '1 month ago')
  assert.equal(relativeBlameTime(String(nowSec - 365 * 86400), NOW), '1 year ago')
  assert.equal(relativeBlameTime(String(nowSec + 3 * 86400), NOW), 'in 3 days')
  assert.equal(relativeBlameTime(String(nowSec + 30), NOW), 'in a moment')
}

{
  assert.equal(blameLaneColor('abc123'), blameLaneColor('abc123'))
  assert.ok(LANE_COLORS.includes(blameLaneColor('abc123') as (typeof LANE_COLORS)[number]))
  assert.ok(LANE_COLORS.includes(blameLaneColor('') as (typeof LANE_COLORS)[number]))
}

console.log('blameBlocks.test.ts: ok')
