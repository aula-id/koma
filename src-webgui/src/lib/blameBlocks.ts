// Contiguous blame runs for the gutter beside a read-only Monaco buffer.
// The same oid later in the file is a new block: a gap means a different edit.
// Line boxes stay uniform (no wrap, no folds) so top = (line - 1) * lineHeight.

import { LANE_COLORS } from './gitGraphLayout'

export const BLAME_LINE_HEIGHT = 18
export const BLAME_LANE_WIDTH = 176

export type BlameRow = {
  oid: string
  author: string
  time: string
  summary: string
  text: string
  body?: string
}

export type BlameBlock = {
  start: number
  end: number
  oid: string
  author: string
  time: string
  summary: string
  body: string
}

export function blameBlocks(rows: readonly BlameRow[]): BlameBlock[] {
  const blocks: BlameBlock[] = []
  for (let i = 0; i < rows.length; i++) {
    const row = rows[i]
    const prev = blocks[blocks.length - 1]
    if (prev && prev.oid === row.oid && prev.end === i) {
      prev.end = i + 1
      if (!prev.body && row.body) prev.body = row.body
      continue
    }
    blocks.push({
      start: i + 1,
      end: i + 1,
      oid: row.oid,
      author: row.author,
      time: row.time,
      summary: row.summary,
      body: row.body ?? '',
    })
  }
  return blocks
}

export function visibleBlameBlocks(
  blocks: readonly BlameBlock[],
  scrollTop: number,
  viewHeight: number,
  lineHeight = BLAME_LINE_HEIGHT,
  overscanPx = lineHeight * 30,
): BlameBlock[] {
  const top = scrollTop - overscanPx
  const bottom = scrollTop + Math.max(viewHeight, 0) + overscanPx
  return blocks.filter((block) => {
    const y0 = (block.start - 1) * lineHeight
    const y1 = block.end * lineHeight
    return y1 > top && y0 < bottom
  })
}

const MINUTE = 60
const HOUR = 60 * MINUTE
const DAY = 24 * HOUR
const WEEK = 7 * DAY
const MONTH = 30 * DAY
const YEAR = 365 * DAY

export function relativeBlameTime(unixSeconds: string, nowMs = Date.now()): string {
  const then = Number(unixSeconds)
  if (!Number.isFinite(then)) return ''
  const delta = Math.round(nowMs / 1000 - then)
  const abs = Math.abs(delta)
  const phrase = (count: number, unit: string) => {
    const word = `${count} ${unit}${count === 1 ? '' : 's'}`
    return delta < 0 ? `in ${word}` : `${word} ago`
  }
  if (abs < MINUTE) return delta < 0 ? 'in a moment' : 'just now'
  if (abs < HOUR) return phrase(Math.max(1, Math.round(abs / MINUTE)), 'minute')
  if (abs < DAY) return phrase(Math.max(1, Math.round(abs / HOUR)), 'hour')
  if (abs < WEEK) return phrase(Math.max(1, Math.round(abs / DAY)), 'day')
  if (abs < MONTH) return phrase(Math.max(1, Math.round(abs / WEEK)), 'week')
  if (abs < YEAR) return phrase(Math.max(1, Math.round(abs / MONTH)), 'month')
  return phrase(Math.max(1, Math.round(abs / YEAR)), 'year')
}

function hashOid(oid: string): number {
  let hash = 0
  for (let i = 0; i < oid.length; i++) hash = (hash * 31 + oid.charCodeAt(i)) | 0
  return Math.abs(hash)
}

export function blameLaneColor(oid: string): string {
  return LANE_COLORS[hashOid(oid) % LANE_COLORS.length]
}
