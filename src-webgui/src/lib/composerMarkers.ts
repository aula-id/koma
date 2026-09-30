// Composer attachment markers — TUI parity with `[Image #N]` / `[Pasted Text #N]`.

import { pasteMarker, type PasteMarkerRow } from './pasteText'

export { pasteMarker }

export function imageMarker(n: number): string {
  return `[Image #${n}]`
}

export type MarkerInsertRow = PasteMarkerRow & { kind: 'image' | 'pasted_text' }

const IMAGE_MARKER = /\[Image #(\d+)\]/g
const PASTE_MARKER = /\[Pasted Text #(\d+)\]/g

export function findAttachmentMarkerRanges(text: string): Array<[number, number]> {
  const ranges: Array<[number, number]> = []
  for (const re of [IMAGE_MARKER, PASTE_MARKER]) {
    re.lastIndex = 0
    let match: RegExpExecArray | null
    while ((match = re.exec(text))) {
      ranges.push([match.index, match.index + match[0].length])
    }
  }
  ranges.sort((a, b) => a[0] - b[0])
  return ranges
}

export function markerLabel(kind: 'image' | 'pasted_text', n: number): string {
  return kind === 'image' ? imageMarker(n) : pasteMarker(n)
}

export function markerKeysInText(text: string): Set<string> {
  const keys = new Set<string>()
  IMAGE_MARKER.lastIndex = 0
  let m: RegExpExecArray | null
  while ((m = IMAGE_MARKER.exec(text))) keys.add(`image:${m[1]}`)
  PASTE_MARKER.lastIndex = 0
  while ((m = PASTE_MARKER.exec(text))) keys.add(`pasted_text:${m[1]}`)
  return keys
}

/** Insert `marker` at `index` in `text`; returns new text and caret after the marker. */
export function insertMarkerAt(text: string, index: number, marker: string): { text: string; caret: number } {
  const at = Math.max(0, Math.min(index, text.length))
  const next = text.slice(0, at) + marker + text.slice(at)
  return { text: next, caret: at + marker.length }
}

/**
 * Bind staged attachment numbers to pending insert rows (images + collapsed pastes).
 * Mutates `queue` and `seen` (`kind:markerN` keys).
 */
export function assignFreshMarkerInserts(
  queue: MarkerInsertRow[],
  seen: Set<string>,
  attachments: { markerN: number; kind: string }[],
): Array<{ id: string; kind: 'image' | 'pasted_text'; markerN: number; marker: string; cancelled: boolean }> {
  const assigned: Array<{ id: string; kind: 'image' | 'pasted_text'; markerN: number; marker: string; cancelled: boolean }> = []
  for (const att of attachments) {
    if (att.kind !== 'pasted_text' && att.kind !== 'image') continue
    const key = `${att.kind}:${att.markerN}`
    if (seen.has(key)) continue
    seen.add(key)
    const row = queue.find((item) => item.kind === att.kind && item.markerN == null && !item.cancelled)
    if (!row) continue
    row.markerN = att.markerN
    const marker = markerLabel(att.kind as 'image' | 'pasted_text', att.markerN)
    assigned.push({
      id: row.id,
      kind: att.kind as 'image' | 'pasted_text',
      markerN: att.markerN,
      marker,
      cancelled: row.cancelled,
    })
  }
  return assigned
}
