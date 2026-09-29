// Long composer pastes become `[Pasted Text #N]` chips. The threshold matches
// the TUI (`should_collapse_paste`): 150 characters, or more than one line.

export const PASTE_COLLAPSE_MIN_CHARS = 150
export const PASTE_SOFT_MAX_BYTES = 2 * 1024 * 1024

export type PastedBlock = { n: number; path: string; text: string }

export function pasteCharCount(text: string): number {
  return [...text].length
}

export function pasteByteLength(text: string): number {
  return new TextEncoder().encode(text).length
}

export function shouldCollapsePaste(text: string): boolean {
  return pasteCharCount(text) >= PASTE_COLLAPSE_MIN_CHARS || text.includes('\n')
}

export function pasteMarker(n: number): string {
  return `[Pasted Text #${n}]`
}

export type PasteMarkerRow = { id: string; markerN: number | null; cancelled: boolean }

/**
 * Bind paste chips that just appeared in the snapshot to composer rows, in
 * list order. `seen` holds marker numbers already on screen, so a reload does
 * not steal them for a new paste. Mutates `queue` and `seen`.
 */
export function assignFreshPasteMarkers(
  queue: PasteMarkerRow[],
  seen: Set<number>,
  attachments: { markerN: number; kind: string }[],
): PasteMarkerRow[] {
  const assigned: PasteMarkerRow[] = []
  for (const att of attachments) {
    if (att.kind !== 'pasted_text') continue
    if (seen.has(att.markerN)) continue
    seen.add(att.markerN)
    const row = queue.find((item) => item.markerN == null)
    if (!row) continue
    row.markerN = att.markerN
    assigned.push({ id: row.id, markerN: att.markerN, cancelled: row.cancelled })
  }
  return assigned
}

export function formatPasteFence(block: PastedBlock): string {
  const body = block.text.replace(/\r\n/g, '\n').replace(/\r/g, '\n')
  return `<<<pasted_text n=${block.n} path="${block.path}">>>\n${body}\n<<<end_pasted_text n=${block.n}>>>`
}

const FENCE = /<<<pasted_text n=(\d+) path="([^"]*)">>>\r?\n?([\s\S]*?)\r?\n?<<<end_pasted_text n=\d+>>>/g

/** Pull paste fences out of a user message. The fence body is what the model read. */
export function splitPasteMessage(content: string): { prose: string; pastes: PastedBlock[] } {
  const pastes: PastedBlock[] = []
  const prose = content.replace(FENCE, (_fence, n: string, path: string, body: string) => {
    pastes.push({ n: Number(n), path, text: body })
    return ''
  })
  return { prose: prose.replace(/\n{3,}/g, '\n\n').trim(), pastes }
}
