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

/** Marker keys present in `prev` but removed from `next` (user edited the draft). */
export function attachmentMarkersRemoved(prevText: string, nextText: string): Set<string> {
  const prev = markerKeysInText(prevText)
  const next = markerKeysInText(nextText)
  const removed = new Set<string>()
  for (const key of prev) {
    if (!next.has(key)) removed.add(key)
  }
  return removed
}

export function attachmentMarkerInDraft(
  draft: string,
  kind: 'image' | 'pasted_text' | 'file',
  markerN: number,
): boolean {
  if (kind === 'image') return draft.includes(imageMarker(markerN))
  if (kind === 'pasted_text') return draft.includes(pasteMarker(markerN))
  return false
}

export type ListedComposerAttachment = {
  key: string
  kind: 'image' | 'file' | 'pasted_text'
  markerN: number | null
  name: string
  id?: string
}

/** Unplaced session attachments plus in-flight pastes for the composer strip. */
export function listedComposerAttachments(
  draft: string,
  attachments: Array<{ kind: 'image' | 'file' | 'pasted_text'; markerN: number; name: string }>,
  localPastes: Array<{ id: string; markerN?: number; n?: number }>,
): ListedComposerAttachment[] {
  const listed: ListedComposerAttachment[] = []
  const seen = new Set<string>()
  for (const att of attachments) {
    if (att.kind !== 'file' && attachmentMarkerInDraft(draft, att.kind, att.markerN)) continue
    const key = `${att.kind}:${att.markerN}`
    listed.push({ key, kind: att.kind, markerN: att.markerN, name: att.name })
    seen.add(key)
  }
  let unmatchedSessionPastes = attachments.filter(
    (att) =>
      att.kind === 'pasted_text' &&
      !attachmentMarkerInDraft(draft, att.kind, att.markerN) &&
      !localPastes.some((paste) => paste.markerN === att.markerN),
  ).length
  for (const paste of localPastes) {
    if (paste.markerN != null) {
      const key = `pasted_text:${paste.markerN}`
      if (seen.has(key) || attachmentMarkerInDraft(draft, 'pasted_text', paste.markerN)) continue
      listed.push({
        key: paste.id,
        kind: 'pasted_text',
        markerN: paste.markerN,
        name: `Pasted Text #${paste.markerN}`,
        id: paste.id,
      })
      seen.add(key)
      continue
    }
    if (unmatchedSessionPastes > 0) {
      unmatchedSessionPastes -= 1
      continue
    }
    listed.push({
      key: paste.id,
      kind: 'pasted_text',
      markerN: null,
      name: 'Pasted Text',
      id: paste.id,
    })
  }
  return listed
}

/** Markers appended on send when the user never dropped the pile into the draft. */
export function trailingAttachmentMarkers(
  prose: string,
  attachments: Array<{ kind: string; markerN: number }>,
  localPastes: Array<{ markerN?: number }>,
): string[] {
  const markers: string[] = []
  const seen = new Set<string>()
  const add = (marker: string) => {
    if (seen.has(marker) || prose.includes(marker)) return
    seen.add(marker)
    markers.push(marker)
  }
  for (const item of localPastes) {
    if (item.markerN != null) add(pasteMarker(item.markerN))
  }
  for (const item of attachments) {
    if (item.kind === 'pasted_text') add(pasteMarker(item.markerN))
  }
  for (const item of attachments) {
    if (item.kind === 'image') add(imageMarker(item.markerN))
  }
  return markers
}

/** True while a paste/image row is still waiting on the daemon marker number. */
export function attachmentQueueStillPending(
  queue: readonly { kind: 'image' | 'pasted_text'; markerN: number | null; cancelled: boolean }[],
  kind: 'image' | 'pasted_text',
  markerN: number,
): boolean {
  return queue.some((row) => row.kind === kind && !row.cancelled && (row.markerN == null || row.markerN === markerN))
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
    const row = queue.find((item) => item.kind === att.kind && item.markerN == null && !item.cancelled)
    if (!row) continue
    seen.add(key)
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
