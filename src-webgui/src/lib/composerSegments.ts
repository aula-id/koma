// Composer draft segments: file refs, attachment markers, and prose moves.

import { findAttachmentMarkerRanges } from './composerMarkers'

export type ComposerTokenKind = 'fileRef' | 'image' | 'pasted_text'

export type ComposerToken = {
  kind: ComposerTokenKind
  start: number
  end: number
  label: string
}

function isClaimed(ranges: Array<[number, number]>, s: number, e: number): boolean {
  return ranges.some(([rs, re]) => s < re && e > rs)
}

/** `@label` and `@[n]path` ranges (mirrors Composer overlay pass 1+2). */
export function findFileRefRanges(text: string, pickedTokens: Set<string>): Array<[number, number]> {
  const ranges: Array<[number, number]> = []
  const tokens = Array.from(pickedTokens)
    .filter((t) => t.length > 0)
    .sort((a, b) => b.length - a.length)
  for (const token of tokens) {
    let from = 0
    while (from <= text.length - token.length) {
      const idx = text.indexOf(token, from)
      if (idx === -1) break
      const end = idx + token.length
      if (!isClaimed(ranges, idx, end)) ranges.push([idx, end])
      from = idx + 1
    }
  }
  const sentinelRe = /@\[\d+\]\S+/g
  let m: RegExpExecArray | null
  while ((m = sentinelRe.exec(text))) {
    const idx = m.index
    const end = idx + m[0].length
    if (!isClaimed(ranges, idx, end)) ranges.push([idx, end])
  }
  ranges.sort((a, b) => a[0] - b[0])
  return ranges
}

export function allComposerTokenRanges(text: string, pickedTokens: Set<string>): Array<[number, number]> {
  return [...findFileRefRanges(text, pickedTokens), ...findAttachmentMarkerRanges(text)].sort((a, b) => a[0] - b[0])
}

function tokenKind(slice: string): ComposerTokenKind {
  if (/^\[Image #\d+\]$/.test(slice)) return 'image'
  if (/^\[Pasted Text #\d+\]$/.test(slice)) return 'pasted_text'
  return 'fileRef'
}

export function listComposerTokens(text: string, pickedTokens: Set<string>): ComposerToken[] {
  return allComposerTokenRanges(text, pickedTokens).map(([start, end]) => {
    const label = text.slice(start, end)
    return { kind: tokenKind(label), start, end, label }
  })
}

/** Move a token substring to `toIndex` (caret-style insert point in the pre-move string). */
export function moveComposerToken(
  text: string,
  start: number,
  end: number,
  toIndex: number,
): { text: string; caret: number } {
  if (start < 0 || end <= start || end > text.length) return { text, caret: toIndex }
  const slice = text.slice(start, end)
  let insertAt = Math.max(0, Math.min(toIndex, text.length))
  if (insertAt > start) insertAt -= end - start
  const without = text.slice(0, start) + text.slice(end)
  insertAt = Math.max(0, Math.min(insertAt, without.length))
  const next = without.slice(0, insertAt) + slice + without.slice(insertAt)
  return { text: next, caret: insertAt + slice.length }
}

/** Markdown for live preview: keep attachment markers as inline code. */
export function composerPreviewMarkdown(draft: string): string {
  return draft.replace(/\[(?:Image|Pasted Text) #\d+\]/g, (marker) => `\`${marker}\``)
}

export function wrapSelection(
  text: string,
  start: number,
  end: number,
  before: string,
  after: string,
): { text: string; caretStart: number; caretEnd: number } {
  const selected = text.slice(start, end)
  const next = text.slice(0, start) + before + selected + after + text.slice(end)
  const caretStart = start + before.length
  const caretEnd = caretStart + selected.length
  return { text: next, caretStart, caretEnd: selected.length ? caretEnd : caretStart }
}
